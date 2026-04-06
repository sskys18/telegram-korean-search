use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::collector;
use crate::store::message::MessageWithChat;
use crate::store::wiki_category::WikiCategory;
use crate::store::wiki_page::{WikiPage, WikiPageSearchResult};
use crate::store::wiki_topic::WikiTopic;
use crate::wiki;
use crate::AppState;

#[derive(Serialize)]
pub struct ApiCredentials {
    pub api_id: i32,
    pub api_hash: String,
}

#[derive(Serialize)]
pub struct ConnectResult {
    pub authorized: bool,
}

#[derive(Serialize)]
pub struct SignInResponse {
    pub success: bool,
    pub requires_2fa: bool,
    pub hint: Option<String>,
}

#[derive(Serialize)]
pub struct WikiStatus {
    pub queue_pending: i64,
    pub queue_processing: i64,
    pub queue_done: i64,
    pub queue_failed: i64,
    pub queue_skipped: i64,
    pub topics_count: i64,
    pub is_running: bool,
}

#[derive(Serialize)]
pub struct WikiTopicDetail {
    pub topic: WikiTopic,
    pub latest_page: Option<WikiPage>,
    pub source_count: i64,
}

#[derive(Serialize)]
pub struct WikiSearchResult {
    pub topics: Vec<WikiTopic>,
    pub pages: Vec<WikiPageSearchResult>,
}

fn mask_api_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 4 {
        return "*".repeat(chars.len());
    }

    let visible: String = chars[chars.len() - 4..].iter().collect();
    format!("{}{}", "*".repeat(chars.len() - 4), visible)
}

fn count_wiki_topics(store: &crate::store::Store) -> Result<i64, sqlite::Error> {
    let mut stmt = store.conn().prepare("SELECT COUNT(*) FROM wiki_topics")?;
    stmt.next()?;
    stmt.read::<i64, _>(0)
}

fn count_topic_sources(store: &crate::store::Store, topic_id: i64) -> Result<i64, sqlite::Error> {
    let mut stmt = store
        .conn()
        .prepare("SELECT COUNT(*) FROM wiki_topic_messages WHERE topic_id = ?")?;
    stmt.bind((1, topic_id))?;
    stmt.next()?;
    stmt.read::<i64, _>(0)
}

/// Read saved API credentials from the database.
#[tauri::command]
pub fn get_api_credentials(state: State<AppState>) -> Result<Option<ApiCredentials>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let api_id = store.get_meta("tg_api_id").map_err(|e| e.to_string())?;
    let api_hash = store.get_meta("tg_api_hash").map_err(|e| e.to_string())?;
    match (api_id, api_hash) {
        (Some(id_str), Some(hash)) => {
            let id: i32 = id_str
                .parse()
                .map_err(|_| "invalid api_id in database".to_string())?;
            Ok(Some(ApiCredentials {
                api_id: id,
                api_hash: hash,
            }))
        }
        _ => Ok(None),
    }
}

/// Save API credentials to the database.
#[tauri::command]
pub fn save_api_credentials(
    state: State<AppState>,
    api_id: i32,
    api_hash: String,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .set_meta("tg_api_id", &api_id.to_string())
        .map_err(|e| e.to_string())?;
    store
        .set_meta("tg_api_hash", &api_hash)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Connect to Telegram using saved credentials.
/// Stores the client in AppState and checks if already authorized.
/// If the session is stale (AUTH_KEY_UNREGISTERED), deletes it and reconnects fresh.
#[tauri::command]
pub async fn connect_telegram(state: State<'_, AppState>) -> Result<ConnectResult, String> {
    // Read api_id and auth flag from DB
    let (api_id, was_authenticated) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let id_str = store
            .get_meta("tg_api_id")
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "API credentials not configured".to_string())?;
        let api_id = id_str
            .parse::<i32>()
            .map_err(|_| "invalid api_id in database".to_string())?;
        let authenticated = store
            .get_meta("tg_authenticated")
            .map_err(|e| e.to_string())?
            .is_some_and(|v| v == "1");
        (api_id, authenticated)
    };

    let session_path = collector::session_path();

    // Abort any existing runner before connecting
    if let Some(old) = state.runner_handle.lock().await.take() {
        old.abort();
    }
    // Clear the old client
    *state.client.lock().await = None;

    // Only try to reuse an existing session if login was previously completed.
    // Otherwise, delete any leftover session file to avoid stale auth key issues.
    if was_authenticated && session_path.exists() {
        let (client, runner) = collector::connect(api_id)
            .await
            .map_err(|e| e.to_string())?;

        let auth_check = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            collector::auth::is_authorized(&client),
        )
        .await;

        match auth_check {
            Ok(Ok(true)) => {
                *state.client.lock().await = Some(client);
                *state.runner_handle.lock().await = Some(runner);
                return Ok(ConnectResult { authorized: true });
            }
            _ => {
                // Session expired — clean up and reconnect fresh.
                runner.abort();
                let _ = std::fs::remove_file(&session_path);
                // Clear the authenticated flag since session is no longer valid.
                let store = state.store.lock().map_err(|e| e.to_string())?;
                let _ = store.delete_meta("tg_authenticated");
                log::info!("Session expired, reconnecting fresh");
            }
        }
    } else if session_path.exists() {
        // Session file exists but user never completed login — just delete it.
        let _ = std::fs::remove_file(&session_path);
    }

    // Fresh connection
    let (client, runner) = collector::connect(api_id)
        .await
        .map_err(|e| e.to_string())?;

    *state.client.lock().await = Some(client);
    *state.runner_handle.lock().await = Some(runner);

    Ok(ConnectResult { authorized: false })
}

/// Request a login code for the given phone number.
#[tauri::command]
pub async fn request_login_code(state: State<'_, AppState>, phone: String) -> Result<(), String> {
    let client_guard = state.client.lock().await;
    let client = client_guard
        .as_ref()
        .ok_or_else(|| "Client not connected".to_string())?;

    // Read api_hash from DB for the login code request
    let api_hash = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .get_meta("tg_api_hash")
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "API credentials not configured".to_string())?
    };

    let token = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        collector::auth::request_login_code(client, &phone, &api_hash),
    )
    .await
    .map_err(|_| "Connection timed out. Please try again.".to_string())?
    .map_err(|e| e.to_string())?;
    *state.login_token.lock().await = Some(token);
    Ok(())
}

/// Submit the login code. Returns whether 2FA is needed.
#[tauri::command]
pub async fn submit_login_code(
    state: State<'_, AppState>,
    code: String,
) -> Result<SignInResponse, String> {
    let client_guard = state.client.lock().await;
    let client = client_guard
        .as_ref()
        .ok_or_else(|| "Client not connected".to_string())?;

    let token = state
        .login_token
        .lock()
        .await
        .take()
        .ok_or_else(|| "No login token. Call request_login_code first.".to_string())?;

    let result = collector::auth::sign_in(client, &token, &code)
        .await
        .map_err(|e| e.to_string())?;

    match result {
        collector::auth::SignInResult::Success => {
            // Mark as authenticated so we can reuse the session on next launch.
            let store = state.store.lock().map_err(|e| e.to_string())?;
            let _ = store.set_meta("tg_authenticated", "1");
            Ok(SignInResponse {
                success: true,
                requires_2fa: false,
                hint: None,
            })
        }
        collector::auth::SignInResult::TwoFactorRequired {
            password_token,
            hint,
        } => {
            *state.password_token.lock().await = Some(password_token);
            Ok(SignInResponse {
                success: false,
                requires_2fa: true,
                hint: Some(hint),
            })
        }
    }
}

/// Submit 2FA password.
#[tauri::command]
pub async fn submit_password(state: State<'_, AppState>, password: String) -> Result<(), String> {
    let client_guard = state.client.lock().await;
    let client = client_guard
        .as_ref()
        .ok_or_else(|| "Client not connected".to_string())?;

    let token = state
        .password_token
        .lock()
        .await
        .take()
        .ok_or_else(|| "No password token. Complete sign_in first.".to_string())?;

    collector::auth::check_password(client, *token, &password)
        .await
        .map_err(|e| e.to_string())?;

    // Mark as authenticated so we can reuse the session on next launch.
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let _ = store.set_meta("tg_authenticated", "1");
    Ok(())
}

/// Start initial message collection in a background thread.
/// Emits progress events: "collection-progress", "collection-complete", "collection-error".
#[tauri::command]
pub async fn start_collection(app: AppHandle) -> Result<(), String> {
    let client = app
        .state::<AppState>()
        .client
        .lock()
        .await
        .as_ref()
        .ok_or_else(|| "Client not connected".to_string())?
        .clone();

    std::thread::spawn(move || {
        run_collection(app, client);
    });

    Ok(())
}

#[tauri::command]
pub fn save_openai_api_key(state: State<AppState>, key: String) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .set_meta("openai_api_key", &key)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_openai_api_key(state: State<AppState>) -> Result<Option<String>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let key = store
        .get_meta("openai_api_key")
        .map_err(|e| e.to_string())?;
    Ok(key.map(|value| mask_api_key(&value)))
}

#[tauri::command]
pub async fn validate_openai_api_key(key: String) -> Result<bool, String> {
    wiki::llm::LlmClient::new(key)
        .validate_key()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_wiki_worker(app: AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();

    {
        let mut guard = state.wiki_worker_shutdown.lock().await;
        if let Some(shutdown) = guard.as_ref() {
            if !shutdown.load(Ordering::Relaxed) {
                return Err("Wiki worker is already running".to_string());
            }
        }
        *guard = None;
    }

    let api_key = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .get_meta("openai_api_key")
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "OpenAI API key not configured".to_string())?
    };

    let shutdown = wiki::worker::start_worker(app.clone(), api_key);
    let mut guard = state.wiki_worker_shutdown.lock().await;
    *guard = Some(shutdown);
    Ok(())
}

#[tauri::command]
pub async fn stop_wiki_worker(state: State<'_, AppState>) -> Result<(), String> {
    let mut guard = state.wiki_worker_shutdown.lock().await;
    if let Some(shutdown) = guard.take() {
        shutdown.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[tauri::command]
pub async fn get_wiki_status(state: State<'_, AppState>) -> Result<WikiStatus, String> {
    let (queue_stats, topics_count) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        (
            store.get_queue_stats().map_err(|e| e.to_string())?,
            count_wiki_topics(&store).map_err(|e| e.to_string())?,
        )
    };

    let is_running = state
        .wiki_worker_shutdown
        .lock()
        .await
        .as_ref()
        .is_some_and(|shutdown| !shutdown.load(Ordering::Relaxed));

    Ok(WikiStatus {
        queue_pending: queue_stats.pending,
        queue_processing: queue_stats.processing,
        queue_done: queue_stats.done,
        queue_failed: queue_stats.failed,
        queue_skipped: queue_stats.skipped,
        topics_count,
        is_running,
    })
}

#[tauri::command]
pub fn reprocess_wiki(state: State<AppState>) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.clear_classify_queue().map_err(|e| e.to_string())?;
    store.clear_wiki_pages().map_err(|e| e.to_string())?;
    store.clear_wiki_topics().map_err(|e| e.to_string())?;
    store.clear_wiki_stats().map_err(|e| e.to_string())?;
    store.enqueue_all_messages().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn clear_wiki_data(state: State<AppState>) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.clear_classify_queue().map_err(|e| e.to_string())?;
    store.clear_wiki_pages().map_err(|e| e.to_string())?;
    store.clear_wiki_topics().map_err(|e| e.to_string())?;
    store.clear_wiki_stats().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_trending_topics(
    state: State<AppState>,
    limit: usize,
    offset: usize,
    category_id: Option<i64>,
) -> Result<Vec<WikiTopic>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .get_trending_topics(limit, offset, category_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_wiki_categories(state: State<AppState>) -> Result<Vec<WikiCategory>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.get_all_categories().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_topic_detail(state: State<AppState>, topic_id: i64) -> Result<WikiTopicDetail, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let topic = store
        .get_topic(topic_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Topic not found".to_string())?;
    let latest_page = store.get_latest_page(topic_id).map_err(|e| e.to_string())?;
    let source_count = count_topic_sources(&store, topic_id).map_err(|e| e.to_string())?;

    Ok(WikiTopicDetail {
        topic,
        latest_page,
        source_count,
    })
}

#[tauri::command]
pub fn get_topic_sources(
    state: State<AppState>,
    topic_id: i64,
    limit: usize,
    offset: usize,
) -> Result<Vec<MessageWithChat>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .get_topic_sources(topic_id, limit, offset)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn search_wiki(
    state: State<AppState>,
    query: String,
    limit: usize,
) -> Result<WikiSearchResult, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let topics = store
        .search_topics(&query, limit)
        .map_err(|e| e.to_string())?;
    let pages = store
        .search_wiki_pages(&query, limit)
        .map_err(|e| e.to_string())?;

    Ok(WikiSearchResult { topics, pages })
}

#[tauri::command]
pub async fn generate_topic_summary(app: AppHandle, topic_id: i64) -> Result<WikiPage, String> {
    let state = app.state::<AppState>();

    let api_key = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .get_meta("openai_api_key")
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "OpenAI API key not configured".to_string())?
    };

    let (topic, latest_page, needs_regeneration, sources, source_ids) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let topic = store
            .get_topic(topic_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "Topic not found".to_string())?;
        let latest_page = store.get_latest_page(topic_id).map_err(|e| e.to_string())?;
        let needs_regeneration = store
            .needs_regeneration(topic_id)
            .map_err(|e| e.to_string())?;
        let sources = store
            .get_topic_sources(topic_id, 50, 0)
            .map_err(|e| e.to_string())?;
        let source_ids: Vec<(i64, i64)> =
            sources.iter().map(|m| (m.chat_id, m.message_id)).collect();
        (topic, latest_page, needs_regeneration, sources, source_ids)
    };

    if !needs_regeneration {
        if let Some(page) = latest_page {
            return Ok(page);
        }
    }

    let llm = wiki::llm::LlmClient::new(api_key);
    let source_refs: Vec<(usize, i64, &str, &str)> = sources
        .iter()
        .enumerate()
        .map(|(idx, source)| {
            (
                idx + 1,
                source.timestamp,
                source.chat_title.as_str(),
                source.text_plain.as_str(),
            )
        })
        .collect();

    let category = topic.category_name.as_deref().unwrap_or("Other");
    let (content_ko, content_en) = llm
        .generate_summary(&topic.title, category, &source_refs)
        .await
        .map_err(|e| e.to_string())?;

    let page_id = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .insert_wiki_page(topic_id, &content_ko, &content_en, &source_ids)
            .map_err(|e| e.to_string())?
    };

    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .get_latest_page(topic_id)
        .map_err(|e| e.to_string())?
        .filter(|page| page.page_id == page_id)
        .ok_or_else(|| "Failed to load generated wiki page".to_string())
}

// Runs on a dedicated thread with a multi-threaded tokio runtime.
// Network I/O is parallelized (up to 3 concurrent channels via Semaphore).
// DB writes are serialized in the join_next() loop — no mutex contention.
fn run_collection(app: AppHandle, client: grammers_client::Client) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let state = app.state::<AppState>();

        // Phase 1: Fetch chats from network (no store lock needed)
        let _ = app.emit(
            "collection-progress",
            serde_json::json!({
                "phase": "chats",
                "detail": "Fetching chat list..."
            }),
        );

        let chat_rows = match collector::messages::fetch_chats(&client).await {
            Ok(rows) => rows,
            Err(e) => {
                log::error!("Chat fetch failed: {}", e);
                let _ = app.emit("collection-error", e.to_string());
                return;
            }
        };

        // Brief lock: save chats and read excluded set
        let excluded_ids: std::collections::HashSet<i64> = {
            let store = state.store.lock().unwrap();
            for row in &chat_rows {
                if let Err(e) = store.upsert_chat(row) {
                    log::warn!("Failed to save chat {}: {}", row.title, e);
                }
            }
            let active = store.get_active_chats().unwrap_or_default();
            let active_ids: std::collections::HashSet<i64> =
                active.iter().map(|c| c.chat_id).collect();
            chat_rows
                .iter()
                .filter(|c| !active_ids.contains(&c.chat_id))
                .map(|c| c.chat_id)
                .collect()
        };

        // Filter out excluded chats, then sort by chat type priority:
        // broadcast channels first, small groups next, large supergroups last.
        let mut chats: Vec<_> = chat_rows
            .into_iter()
            .filter(|c| !excluded_ids.contains(&c.chat_id))
            .collect();

        // Due to grammers routing, broadcast channels are stored as "supergroup",
        // while both old groups and supergroups are stored as "group".
        // Distinguish supergroups from old groups by ID range (channel IDs < -1T).
        chats.sort_by_key(|c| {
            if c.chat_type == "supergroup" {
                0 // broadcast channels (Peer::Channel → mislabeled "supergroup")
            } else if c.chat_type == "dm" {
                3 // DMs last
            } else if c.chat_id > -1_000_000_000_000 {
                1 // old-style small groups
            } else {
                2 // actual supergroups (large discussion groups)
            }
        });
        let chats_total = chats.len();

        // Phase 2: Fetch messages concurrently (3 at a time)
        let semaphore = Arc::new(Semaphore::new(3));
        let chats_done = Arc::new(AtomicUsize::new(0));
        let active_titles: Arc<tokio::sync::Mutex<Vec<String>>> =
            Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let client = Arc::new(client);

        let mut join_set = JoinSet::new();

        for (i, chat) in chats.into_iter().enumerate() {
            let sem = Arc::clone(&semaphore);
            let cli = Arc::clone(&client);
            let titles = Arc::clone(&active_titles);

            join_set.spawn(async move {
                let _permit = sem.acquire().await.unwrap();

                // Stagger requests slightly after acquiring the permit
                if i > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }

                // Track active channel
                titles.lock().await.push(chat.title.clone());

                let result =
                    collector::messages::fetch_messages_with_retry(&cli, &chat, None).await;

                // Remove from active list
                titles.lock().await.retain(|t| t != &chat.title);

                (chat, result)
            });
        }

        // Collect results and write to DB one at a time
        while let Some(join_result) = join_set.join_next().await {
            let (chat, fetch_result) = match join_result {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("Task panicked: {}", e);
                    continue;
                }
            };

            match fetch_result {
                Ok(rows) => {
                    let count = rows.len();
                    if !rows.is_empty() {
                        let store = state.store.lock().unwrap();
                        if let Err(e) = store.insert_messages_batch(&rows) {
                            log::warn!("Failed to save messages for {}: {}", chat.title, e);
                        } else {
                            let items: Vec<(i64, i64)> =
                                rows.iter().map(|r| (r.chat_id, r.message_id)).collect();
                            let _ = store.enqueue_for_classification(&items);
                        }
                    }
                    log::info!("Fetched {} messages for {}", count, chat.title);
                }
                Err(e) => log::warn!("Failed to fetch messages for {}: {}", chat.title, e),
            }

            let done = chats_done.fetch_add(1, Ordering::Relaxed) + 1;
            let current_active = active_titles.lock().await.clone();
            let _ = app.emit(
                "collection-progress",
                serde_json::json!({
                    "phase": "messages",
                    "chats_done": done,
                    "chats_total": chats_total,
                    "active_chats": current_active,
                }),
            );
        }

        let _ = app.emit(
            "collection-complete",
            serde_json::json!({ "chats": chats_total }),
        );
    });
}
