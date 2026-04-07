use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};

use crate::store::wiki_topic::{normalize_topic_title, NewTopic, TopicMessageLink};
use crate::wiki::llm::{classify_batch_size, ClassifiedTopic, LlmClient, MessageForClassify};
use crate::wiki::trending::calculate_trending_score;
use crate::AppState;

pub fn start_worker(app: AppHandle) -> (Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = Arc::clone(&shutdown);

    let handle = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            run_worker(app, shutdown_clone).await;
        });
    });

    (shutdown, handle)
}

async fn run_worker(app: AppHandle, shutdown: Arc<AtomicBool>) {
    let state = app.state::<AppState>();
    let llm = LlmClient::new();
    let batch_size = classify_batch_size();
    let mut processed_count: usize = 0;

    // Recover stale claims on startup
    {
        let store = state.lock_store();
        let recovered = store.recover_stale_claims().unwrap_or(0);
        if recovered > 0 {
            log::info!("Wiki worker: recovered {} stale queue items", recovered);
        }
    }

    let mut total_channels = {
        let store = state.lock_store();
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

        // Dequeue a batch of items
        let items = {
            let store = state.lock_store();
            store.dequeue_classify_batch(batch_size).unwrap_or_default()
        };

        if items.is_empty() {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        }

        // Build batch: read message text + chat titles for all items
        let mut batch_messages: Vec<MessageForClassify> = Vec::new();
        let mut item_map: Vec<(i64, i64, i64)> = Vec::new(); // (chat_id, message_id, timestamp)

        for (i, item) in items.iter().enumerate() {
            let (msg_data, chat_title) = {
                let store = state.lock_store();
                let msg = store
                    .get_message(item.chat_id, item.message_id)
                    .ok()
                    .flatten();
                let title = store
                    .get_chat(item.chat_id)
                    .ok()
                    .flatten()
                    .map(|c| c.title)
                    .unwrap_or_else(|| "Unknown".to_string());
                (msg, title)
            };

            let msg = match msg_data {
                Some(m) if !m.text_plain.trim().is_empty() => m,
                _ => {
                    // Skip empty/missing messages
                    let store = state.lock_store();
                    let _ = store.mark_queue_skipped(item.chat_id, item.message_id);
                    continue;
                }
            };

            batch_messages.push(MessageForClassify {
                index: i,
                chat_title,
                timestamp: msg.timestamp,
                text: msg.text_plain.clone(),
            });
            item_map.push((item.chat_id, item.message_id, msg.timestamp));
        }

        if batch_messages.is_empty() {
            continue;
        }

        log::info!(
            "Wiki worker: classifying batch of {} messages",
            batch_messages.len()
        );

        // Fetch existing category + topic names so LLM prefers reusing them
        let (existing_categories, existing_topics) = {
            let store = state.lock_store();
            let cats: Vec<String> = store
                .get_all_categories()
                .unwrap_or_default()
                .into_iter()
                .map(|c| c.name)
                .collect();
            let topics: Vec<String> = store
                .get_trending_topics(80, 0, None)
                .unwrap_or_default()
                .into_iter()
                .map(|t| t.title)
                .collect();
            (cats, topics)
        };

        // Call LLM batch classification
        let batch_result = llm
            .classify_batch(&batch_messages, &existing_categories, &existing_topics)
            .await;

        match batch_result {
            Ok(results) => {
                let store = state.lock_store();
                // Process each result
                for (index, response) in &results {
                    if *index >= item_map.len() {
                        continue;
                    }
                    let (chat_id, message_id, timestamp) = item_map[*index];

                    if response.skip || response.topics.is_empty() {
                        let _ = store.mark_queue_skipped(chat_id, message_id);
                    } else {
                        for classified in &response.topics {
                            if let Err(e) = process_classified_topic(
                                &store, classified, chat_id, message_id, timestamp,
                            ) {
                                log::warn!("Failed to process topic '{}': {}", classified.topic, e);
                            }
                        }
                        let _ = store.mark_queue_done(chat_id, message_id);
                    }
                }

                // Mark any items not in the response as needing retry
                let returned_indices: std::collections::HashSet<usize> =
                    results.iter().map(|(idx, _)| *idx).collect();
                for (i, &(chat_id, message_id, _)) in item_map.iter().enumerate() {
                    if !returned_indices.contains(&i) {
                        let _ = store.mark_queue_failed(
                            chat_id,
                            message_id,
                            "Missing from batch response",
                        );
                    }
                }

                processed_count += results.len();
            }
            Err(e) => {
                log::warn!("Batch classification failed: {}", e);
                // Fall back to marking all as failed (will retry)
                let store = state.lock_store();
                for &(chat_id, message_id, _) in &item_map {
                    let _ = store.mark_queue_failed(chat_id, message_id, &e.to_string());
                }
                let _ = app.emit(
                    "wiki-worker-error",
                    serde_json::json!({
                        "message": e.to_string(),
                        "recoverable": true,
                    }),
                );
            }
        }

        // Periodic maintenance
        if processed_count.is_multiple_of(100) {
            let store = state.lock_store();
            total_channels = store.get_total_active_channels().unwrap_or(1);
            let recovered = store.recover_stale_claims().unwrap_or(0);
            if recovered > 0 {
                log::info!("Wiki worker: recovered {} stale claims", recovered);
            }
        }

        if processed_count.is_multiple_of(50) {
            recalculate_trending(&state, total_channels);
        }

        // Emit progress
        let stats = {
            let store = state.lock_store();
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

        // Brief pause between batches
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    // Final trending recalculation
    recalculate_trending(&state, total_channels);
}

fn process_classified_topic(
    store: &crate::store::Store,
    classified: &ClassifiedTopic,
    chat_id: i64,
    message_id: i64,
    message_timestamp: i64,
) -> Result<(), sqlite::Error> {
    // Always resolve the category to get both id and canonical name
    let (category_id, canonical_category) = store
        .resolve_category_with_name(&classified.category, classified.category_ko.as_deref())?;

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
            // Try fuzzy matching before creating a new topic
            if let Some(fuzzy_id) = store.find_topic_fuzzy(&classified.topic)? {
                log::debug!(
                    "Wiki worker: fuzzy-matched '{}' to existing topic {}",
                    classified.topic,
                    fuzzy_id
                );
                if let Some(ref ko) = classified.topic_ko {
                    store.set_title_ko_if_absent(fuzzy_id, ko)?;
                }
                let alias = normalize_topic_title(&classified.topic);
                store.add_topic_alias(fuzzy_id, &alias)?;
                fuzzy_id
            } else {
                let new_topic = NewTopic {
                    title: classified.topic.clone(),
                    title_ko: classified.topic_ko.clone(),
                    category_id,
                };
                store.create_topic(&new_topic)?
            }
        }
    };

    // Store the canonical category name, not the raw LLM output
    let link = TopicMessageLink {
        topic_id,
        chat_id,
        message_id,
        relevance: classified.relevance,
        assigned_category: canonical_category,
    };
    store.link_message_to_topic(&link)?;
    store.record_topic_stat(topic_id, message_timestamp, chat_id)?;

    if let Some(new_cat_id) = store.check_category_reconciliation(topic_id)? {
        store.update_topic_category(topic_id, new_cat_id)?;
    }

    Ok(())
}

fn recalculate_trending(state: &tauri::State<AppState>, total_channels: i64) {
    let store = state.lock_store();
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
