use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};

use crate::store::wiki_topic::{normalize_topic_title, NewTopic, TopicMessageLink};
use crate::wiki::llm::{ClassifiedTopic, LlmClient, LlmError};
use crate::wiki::trending::calculate_trending_score;
use crate::AppState;

pub fn start_worker(app: AppHandle) -> Arc<AtomicBool> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = Arc::clone(&shutdown);

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            run_worker(app, shutdown_clone).await;
        });
    });

    shutdown
}

async fn run_worker(app: AppHandle, shutdown: Arc<AtomicBool>) {
    let state = app.state::<AppState>();
    let llm = LlmClient::new();
    let mut processed_count: usize = 0;

    {
        let store = state.store.lock().unwrap();
        let recovered = store.recover_stale_claims().unwrap_or(0);
        if recovered > 0 {
            log::info!("Wiki worker: recovered {} stale queue items", recovered);
        }
    }

    let mut total_channels = {
        let store = state.store.lock().unwrap();
        store.get_total_active_channels().unwrap_or(1)
    };

    loop {
        if shutdown.load(Ordering::Relaxed) {
            log::info!("Wiki worker: shutdown requested");
            let _ = app.emit(
                "wiki-worker-stopped",
                serde_json::json!({"reason": "shutdown"}),
            );
            break;
        }

        let items = {
            let store = state.store.lock().unwrap();
            store.dequeue_classify_batch(5).unwrap_or_default()
        };

        if items.is_empty() {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        }

        for item in &items {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            let msg_data = {
                let store = state.store.lock().unwrap();
                store
                    .get_message(item.chat_id, item.message_id)
                    .ok()
                    .flatten()
            };

            let msg = match msg_data {
                Some(m) => m,
                None => {
                    let store = state.store.lock().unwrap();
                    let _ = store.mark_queue_skipped(item.chat_id, item.message_id);
                    continue;
                }
            };

            let chat_title = {
                let store = state.store.lock().unwrap();
                store
                    .get_chat(item.chat_id)
                    .ok()
                    .flatten()
                    .map(|c| c.title)
                    .unwrap_or_else(|| "Unknown".to_string())
            };

            if msg.text_plain.trim().is_empty() {
                let store = state.store.lock().unwrap();
                let _ = store.mark_queue_skipped(item.chat_id, item.message_id);
                continue;
            }

            let classify_result: Result<crate::wiki::llm::ClassifyResponse, LlmError> =
                retry_classify(&llm, &chat_title, msg.timestamp, &msg.text_plain).await;

            match classify_result {
                Ok(response) => {
                    if response.skip || response.topics.is_empty() {
                        let store = state.store.lock().unwrap();
                        let _ = store.mark_queue_skipped(item.chat_id, item.message_id);
                    } else {
                        let store = state.store.lock().unwrap();
                        for classified in &response.topics {
                            if let Err(e) = process_classified_topic(
                                &store,
                                classified,
                                item.chat_id,
                                item.message_id,
                                msg.timestamp,
                            ) {
                                log::warn!("Failed to process topic '{}': {}", classified.topic, e);
                            }
                        }
                        let _ = store.mark_queue_done(item.chat_id, item.message_id);
                    }
                }
                Err(e) => {
                    log::warn!(
                        "Classification failed for ({}, {}): {}",
                        item.chat_id,
                        item.message_id,
                        e
                    );
                    let store = state.store.lock().unwrap();
                    let _ = store.mark_queue_failed(item.chat_id, item.message_id, &e.to_string());
                    let _ = app.emit(
                        "wiki-worker-error",
                        serde_json::json!({
                            "message": e.to_string(),
                            "recoverable": true,
                        }),
                    );
                }
            }

            processed_count += 1;

            if processed_count.is_multiple_of(100) {
                let store = state.store.lock().unwrap();
                total_channels = store.get_total_active_channels().unwrap_or(1);
            }

            if processed_count.is_multiple_of(50) {
                recalculate_trending(&state, total_channels);
            }

            let stats = {
                let store = state.store.lock().unwrap();
                store
                    .get_queue_stats()
                    .unwrap_or(crate::store::wiki_queue::QueueStats {
                        pending: 0,
                        processing: 0,
                        done: 0,
                        failed: 0,
                        skipped: 0,
                    })
            };
            let _ = app.emit(
                "wiki-worker-progress",
                serde_json::json!({
                    "processed": stats.done + stats.skipped,
                    "total": stats.done + stats.skipped + stats.pending + stats.failed,
                    "queue_remaining": stats.pending,
                }),
            );

            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    recalculate_trending(&state, total_channels);
}

async fn retry_classify(
    llm: &LlmClient,
    chat_title: &str,
    timestamp: i64,
    text: &str,
) -> Result<crate::wiki::llm::ClassifyResponse, LlmError> {
    let mut last_err = None;
    for attempt in 0..3 {
        match llm.classify_message(chat_title, timestamp, text).await {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                log::warn!("Classify attempt {} failed: {}", attempt + 1, e);
                last_err = Some(e);
                let delay = std::time::Duration::from_millis(500 * 2u64.pow(attempt));
                tokio::time::sleep(delay).await;
            }
        }
    }
    Err(last_err.unwrap())
}

fn process_classified_topic(
    store: &crate::store::Store,
    classified: &ClassifiedTopic,
    chat_id: i64,
    message_id: i64,
    message_timestamp: i64,
) -> Result<(), sqlite::Error> {
    let topic_id = match store.find_topic_by_alias(&classified.topic)? {
        Some(id) => {
            if let Some(ref ko) = classified.topic_ko {
                store.set_title_ko_if_absent(id, ko)?;
            }
            let alias = normalize_topic_title(&classified.topic);
            store.add_topic_alias(id, &alias)?;
            id
        }
        None => {
            let category_id = store.normalize_category(&classified.category)?;
            let new_topic = NewTopic {
                title: classified.topic.clone(),
                title_ko: classified.topic_ko.clone(),
                category_id,
            };
            store.create_topic(&new_topic)?
        }
    };

    let link = TopicMessageLink {
        topic_id,
        chat_id,
        message_id,
        relevance: classified.relevance,
        assigned_category: classified.category.clone(),
    };
    store.link_message_to_topic(&link)?;
    store.record_topic_stat(topic_id, message_timestamp, chat_id)?;

    if let Some(new_cat_id) = store.check_category_reconciliation(topic_id)? {
        store.update_topic_category(topic_id, new_cat_id)?;
    }

    Ok(())
}

fn recalculate_trending(state: &tauri::State<AppState>, total_channels: i64) {
    let store = state.store.lock().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let topic_ids = store.get_active_topic_ids(30).unwrap_or_default();

    for topic_id in topic_ids {
        let topic = match store.get_topic(topic_id) {
            Ok(Some(t)) => t,
            _ => continue,
        };
        let msgs_24h = store.get_topic_msg_count_days(topic_id, 1).unwrap_or(0);
        let msgs_7d = store.get_topic_msg_count_days(topic_id, 7).unwrap_or(0);
        let channels_7d = store.get_topic_channel_count_days(topic_id, 7).unwrap_or(0);

        let score = calculate_trending_score(
            topic.message_count,
            topic.last_seen_at.unwrap_or(0),
            msgs_24h,
            msgs_7d,
            channels_7d,
            total_channels,
            now,
        );

        let _ = store.update_trending_score(topic_id, score);
    }
}
