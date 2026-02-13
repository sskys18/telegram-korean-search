use grammers_client::types::Peer;
use grammers_client::{Client, InvocationError};
use grammers_session::defs::{PeerAuth, PeerId, PeerRef};

use crate::collector::link::build_link;
use crate::store::chat::ChatRow;
use crate::store::message::{strip_whitespace, MessageRow};
use crate::store::Store;

use super::CollectorError;

const BATCH_SIZE: usize = 100;
const MAX_FLOOD_RETRIES: usize = 2;

/// Fetch all dialogs (groups, supergroups, channels) from Telegram.
/// Returns the chat rows without saving to the database.
pub async fn fetch_chats(client: &Client) -> Result<Vec<ChatRow>, CollectorError> {
    let mut dialogs = client.iter_dialogs();
    let mut rows = Vec::new();

    while let Some(dialog) = dialogs
        .next()
        .await
        .map_err(|e| CollectorError::Api(format!("dialog iteration error: {}", e)))?
    {
        let peer = dialog.peer();

        let (chat_type, chat_id, access_hash) = match peer {
            Peer::User(_) => continue, // Skip DMs
            Peer::Group(group) => {
                let id = group.id();
                ("group", id.bot_api_dialog_id(), None)
            }
            Peer::Channel(channel) => {
                let id = peer.id();
                let hash = channel.raw.access_hash;
                ("supergroup", id.bot_api_dialog_id(), hash)
            }
        };

        rows.push(ChatRow {
            chat_id,
            title: peer.name().unwrap_or("").to_string(),
            chat_type: chat_type.to_string(),
            username: peer.username().map(|u| u.to_string()),
            access_hash,
            is_excluded: false,
        });
    }

    Ok(rows)
}

/// Build a PeerRef from stored chat data so we can call iter_messages.
fn peer_ref_from_chat(chat: &ChatRow) -> PeerRef {
    let hash = chat.access_hash.unwrap_or(0);
    // Determine peer kind from chat_type
    match chat.chat_type.as_str() {
        "group" => PeerRef {
            id: PeerId::chat(-chat.chat_id),
            auth: PeerAuth::default(),
        },
        _ => {
            // supergroup / channel: bot_api_dialog_id = -(1000000000000 + bare_id)
            let bare_id = (-chat.chat_id) - 1_000_000_000_000;
            PeerRef {
                id: PeerId::channel(bare_id),
                auth: PeerAuth::from_hash(hash),
            }
        }
    }
}

/// Fetch messages from a single chat over the network.
/// Returns the rows without saving to the database.
/// Fetches from newest to oldest, stopping at `oldest_id` if provided.
pub async fn fetch_messages(
    client: &Client,
    chat: &ChatRow,
    oldest_id: Option<i64>,
) -> Result<Vec<MessageRow>, CollectorError> {
    let peer_ref = peer_ref_from_chat(chat);

    let mut iter = client.iter_messages(peer_ref);
    let mut rows = Vec::with_capacity(BATCH_SIZE);
    let mut fetched = 0;

    while let Some(msg) = iter.next().await.map_err(|e| match &e {
        InvocationError::Rpc(rpc) if rpc.name == "FLOOD_WAIT" => {
            CollectorError::FloodWait(rpc.value.unwrap_or(5))
        }
        _ => CollectorError::Api(format!("message fetch error: {}", e)),
    })? {
        // Stop if we've reached a message we already have
        if let Some(oldest) = oldest_id {
            if (msg.id() as i64) <= oldest {
                break;
            }
        }

        let text = msg.text().to_string();
        if text.is_empty() {
            continue;
        }

        let link = build_link(chat.chat_id, chat.username.as_deref(), msg.id() as i64);

        rows.push(MessageRow {
            message_id: msg.id() as i64,
            chat_id: chat.chat_id,
            timestamp: msg.date().timestamp(),
            text_plain: text.clone(),
            text_stripped: strip_whitespace(&text),
            link: Some(link),
        });

        fetched += 1;
        if fetched >= BATCH_SIZE {
            break;
        }
    }

    Ok(rows)
}

/// Wrapper around `fetch_messages` that retries on FLOOD_WAIT errors.
/// Sleeps for the requested duration and retries up to MAX_FLOOD_RETRIES times.
pub async fn fetch_messages_with_retry(
    client: &Client,
    chat: &ChatRow,
    oldest_id: Option<i64>,
) -> Result<Vec<MessageRow>, CollectorError> {
    for attempt in 0..=MAX_FLOOD_RETRIES {
        match fetch_messages(client, chat, oldest_id).await {
            Ok(rows) => return Ok(rows),
            Err(CollectorError::FloodWait(secs)) if attempt < MAX_FLOOD_RETRIES => {
                log::warn!(
                    "FloodWait {} secs for {}, retrying ({}/{})",
                    secs,
                    chat.title,
                    attempt + 1,
                    MAX_FLOOD_RETRIES
                );
                tokio::time::sleep(std::time::Duration::from_secs(secs as u64)).await;
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

/// Run incremental sync for all active chats concurrently.
/// Fetches new messages since last sync, up to 3 chats at a time.
pub async fn incremental_sync(
    client: &Client,
    store: &std::sync::Mutex<Store>,
) -> Result<usize, CollectorError> {
    // Read all chats and their sync states upfront (single brief lock)
    let chat_states: Vec<_> = {
        let s = store
            .lock()
            .map_err(|e| CollectorError::Api(e.to_string()))?;
        let chats = s
            .get_active_chats()
            .map_err(|e| CollectorError::Api(format!("failed to get active chats: {}", e)))?;
        chats
            .into_iter()
            .map(|chat| {
                let oldest_id = s
                    .get_sync_state(chat.chat_id)
                    .ok()
                    .flatten()
                    .map(|s| s.last_message_id);
                (chat, oldest_id)
            })
            .collect()
    };

    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(3));
    let mut join_set = tokio::task::JoinSet::new();

    for (i, (chat, oldest_id)) in chat_states.into_iter().enumerate() {
        let sem = std::sync::Arc::clone(&semaphore);
        let cli = client.clone();

        join_set.spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            if i > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            let result = fetch_messages_with_retry(&cli, &chat, oldest_id).await;
            (chat, result)
        });
    }

    let mut total = 0;

    while let Some(join_result) = join_set.join_next().await {
        let (chat, fetch_result) = match join_result {
            Ok(r) => r,
            Err(e) => {
                log::warn!("Sync task panicked: {}", e);
                continue;
            }
        };

        match fetch_result {
            Ok(rows) => {
                let count = rows.len();
                if !rows.is_empty() {
                    let s = store
                        .lock()
                        .map_err(|e| CollectorError::Api(e.to_string()))?;
                    s.insert_messages_batch(&rows)
                        .map_err(|e| CollectorError::Api(format!("message save error: {}", e)))?;
                }
                total += count;
                log::info!("Synced {} messages for chat {}", count, chat.title);
            }
            Err(e) => {
                log::warn!("Failed to sync chat {}: {}", chat.title, e);
            }
        }
    }

    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_whitespace_for_messages() {
        assert_eq!(strip_whitespace("삼성 전자 주가"), "삼성전자주가");
    }

    #[test]
    fn test_peer_ref_from_group_chat() {
        let chat = ChatRow {
            chat_id: -123456, // bot_api_dialog_id for groups = -bare_id
            title: "Test Group".to_string(),
            chat_type: "group".to_string(),
            username: None,
            access_hash: None,
            is_excluded: false,
        };
        let pr = peer_ref_from_chat(&chat);
        assert_eq!(pr.id.bare_id(), 123456);
    }

    #[test]
    fn test_peer_ref_from_supergroup_chat() {
        let chat = ChatRow {
            chat_id: -1001234567890, // bot_api_dialog_id for channels = -(1000000000000 + bare_id)
            title: "Test Supergroup".to_string(),
            chat_type: "supergroup".to_string(),
            username: Some("testchat".to_string()),
            access_hash: Some(12345),
            is_excluded: false,
        };
        let pr = peer_ref_from_chat(&chat);
        assert_eq!(pr.id.bare_id(), 1234567890);
        assert_eq!(pr.auth.hash(), 12345);
    }
}
