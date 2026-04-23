//! Wiki classification worker.
//!
//! Spawns a background thread that pulls items off the classify
//! queue, asks the LLM to place each message on a topic, and writes
//! the results back into the wiki tables. Progress is surfaced over
//! an [`EventEmitter`] so the IPC layer can forward it to the Swift
//! shell.
//!
//! The worker replaces the archived Tauri-coupled implementation
//! (see `archive/tauri-v0`). The shape is preserved so the
//! classification logic is unchanged; only the progress plumbing and
//! state handle differ.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::store::wiki_queue::QueueStats;
use crate::store::wiki_topic::{normalize_topic_title, NewTopic, TopicMessageLink};
use crate::store::Store;
use crate::wiki::llm::{classify_batch_size, ClassifiedTopic, LlmClient, MessageForClassify};
use crate::wiki::trending::calculate_trending_score;

/// Pluggable progress channel. The sidecar's IPC server implements
/// this on top of its `ServerEvent` enum. Tests can use the built-in
/// [`NoopEmitter`] to ignore progress entirely.
pub trait EventEmitter: Send + Sync + 'static {
    fn wiki_progress(&self, processed: u64, pending: u64, total: u64);
    fn wiki_error(&self, message: &str, recoverable: bool);
    fn wiki_stopped(&self, reason: &str);
    fn wiki_topics_changed(&self) {}
}

/// Emitter that drops every event on the floor. Useful for tests
/// that exercise the worker state machine without wiring up IPC.
pub struct NoopEmitter;

impl EventEmitter for NoopEmitter {
    fn wiki_progress(&self, _: u64, _: u64, _: u64) {}
    fn wiki_error(&self, _: &str, _: bool) {}
    fn wiki_stopped(&self, _: &str) {}
}

/// Emitter that logs every event via `log::*`. This is the default
/// used by the UniFFI-exposed worker before a real Swift-facing
/// progress callback channel exists.
pub struct LogEmitter;

impl EventEmitter for LogEmitter {
    fn wiki_progress(&self, processed: u64, pending: u64, total: u64) {
        log::info!("wiki progress: processed={processed} pending={pending} total={total}");
    }
    fn wiki_error(&self, message: &str, recoverable: bool) {
        log::warn!("wiki error (recoverable={recoverable}): {message}");
    }
    fn wiki_stopped(&self, reason: &str) {
        log::info!("wiki stopped: {reason}");
    }
}

use std::sync::atomic::AtomicI64;

/// Emitter that forwards events to a foreign (Swift) observer. Falls
/// back to log output when no observer is set. Debounces
/// `on_topics_changed` to at most one call per 500 ms.
pub struct ForeignEmitter {
    pub observer: Arc<Mutex<Option<Arc<dyn crate::uniffi_api::WikiObserver>>>>,
    last_topics_emit_ms: AtomicI64,
}

impl ForeignEmitter {
    pub fn new(observer: Arc<Mutex<Option<Arc<dyn crate::uniffi_api::WikiObserver>>>>) -> Self {
        Self {
            observer,
            last_topics_emit_ms: AtomicI64::new(0),
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn observer(&self) -> Option<Arc<dyn crate::uniffi_api::WikiObserver>> {
        self.observer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn topics_changed(&self) {
        let now = Self::now_ms();
        let last = self.last_topics_emit_ms.load(Ordering::Relaxed);
        if now - last < 500 {
            return;
        }
        self.last_topics_emit_ms.store(now, Ordering::Relaxed);
        if let Some(o) = self.observer() {
            o.on_topics_changed();
        }
    }
}

impl EventEmitter for ForeignEmitter {
    fn wiki_progress(&self, processed: u64, pending: u64, total: u64) {
        if let Some(o) = self.observer() {
            o.on_progress(processed, pending, total);
        } else {
            log::info!("wiki progress: processed={processed} pending={pending} total={total}");
        }
    }
    fn wiki_error(&self, message: &str, recoverable: bool) {
        if let Some(o) = self.observer() {
            o.on_error(message.to_string(), recoverable);
        } else {
            log::warn!("wiki error (recoverable={recoverable}): {message}");
        }
    }
    fn wiki_stopped(&self, reason: &str) {
        log::info!("wiki stopped: {reason}");
    }
    fn wiki_topics_changed(&self) {
        self.topics_changed();
    }
}

/// Handle used to interact with a running worker.
pub struct WorkerHandle {
    pub shutdown: Arc<AtomicBool>,
    pub thread: std::thread::JoinHandle<()>,
}

impl WorkerHandle {
    /// Request shutdown. Returns immediately; call
    /// [`WorkerHandle::join`] to block until the thread finishes.
    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::Relaxed);
    }

    pub fn join(self) {
        let _ = self.thread.join();
    }
}

/// Start a wiki worker in a dedicated OS thread with its own tokio
/// runtime. The worker owns an `Arc<Mutex<Store>>` handle so it can
/// run alongside the IPC server without blocking incoming requests
/// beyond individual short critical sections.
pub fn start_worker<E>(
    store: Arc<Mutex<Store>>,
    emitter: Arc<E>,
    wake: Arc<AtomicBool>,
) -> std::io::Result<WorkerHandle>
where
    E: EventEmitter,
{
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = Arc::clone(&shutdown);
    let wake_clone = Arc::clone(&wake);

    let thread = std::thread::Builder::new()
        .name("seoyu-wiki-worker".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    log::error!("wiki worker: failed to build runtime: {e}");
                    return;
                }
            };
            rt.block_on(run_worker(store, emitter, shutdown_clone, wake_clone));
        })?;

    Ok(WorkerHandle { shutdown, thread })
}

async fn run_worker<E>(
    store: Arc<Mutex<Store>>,
    emitter: Arc<E>,
    shutdown: Arc<AtomicBool>,
    wake: Arc<AtomicBool>,
) where
    E: EventEmitter,
{
    let llm = LlmClient::new();
    let batch_size = classify_batch_size();
    let mut processed_count: usize = 0;

    // Recover any items a previous crash left marked as
    // 'processing' so nothing is stranded.
    {
        let store = lock(&store);
        match store.recover_stale_claims() {
            Ok(n) if n > 0 => {
                log::info!("wiki worker: recovered {n} stale queue items");
            }
            Err(e) => log::warn!("wiki worker: recover_stale_claims failed: {e}"),
            _ => {}
        }
    }

    let mut total_channels = {
        let store = lock(&store);
        store.get_total_active_channels().unwrap_or(1)
    };

    loop {
        if shutdown.load(Ordering::Relaxed) {
            log::info!("wiki worker: shutdown requested");
            emitter.wiki_stopped("shutdown");
            break;
        }

        let items = {
            let store = lock(&store);
            store.dequeue_classify_batch(batch_size).unwrap_or_default()
        };

        if items.is_empty() {
            for _ in 0..20 {
                if shutdown.load(Ordering::Relaxed) || wake.load(Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            wake.store(false, Ordering::Relaxed);
            emit_progress(&emitter, &store);
            continue;
        }

        // Read message text + chat titles for each queue item,
        // filtering out empty messages.
        let mut batch_messages: Vec<MessageForClassify> = Vec::new();
        let mut item_map: Vec<(i64, i64, i64)> = Vec::new();

        for (i, item) in items.iter().enumerate() {
            let (msg_data, chat_title) = {
                let store = lock(&store);
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
                    let store = lock(&store);
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
            emit_progress(&emitter, &store);
            continue;
        }

        log::info!(
            "wiki worker: classifying batch of {} messages",
            batch_messages.len()
        );

        let (existing_categories, existing_topics) = {
            let store = lock(&store);
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

        let batch_result = llm
            .classify_batch(&batch_messages, &existing_categories, &existing_topics)
            .await;

        match batch_result {
            Ok(results) => {
                for (index, response) in &results {
                    if *index >= item_map.len() {
                        continue;
                    }
                    let (chat_id, message_id, timestamp) = item_map[*index];
                    let store = lock(&store);
                    if response.skip || response.topics.is_empty() {
                        let _ = store.mark_queue_skipped(chat_id, message_id);
                    } else {
                        for classified in &response.topics {
                            if let Err(e) = process_classified_topic(
                                &store, classified, chat_id, message_id, timestamp,
                            ) {
                                log::warn!(
                                    "wiki worker: failed to process topic '{}': {e}",
                                    classified.topic
                                );
                            }
                        }
                        let _ = store.mark_queue_done(chat_id, message_id);
                    }
                }

                // Anything the LLM didn't return for goes back to the
                // queue as failed so it gets retried next round.
                let returned: std::collections::HashSet<usize> =
                    results.iter().map(|(idx, _)| *idx).collect();
                {
                    let store = lock(&store);
                    for (i, &(chat_id, message_id, _)) in item_map.iter().enumerate() {
                        if !returned.contains(&i) {
                            let _ = store.mark_queue_failed(
                                chat_id,
                                message_id,
                                "missing from batch response",
                            );
                        }
                    }
                }

                processed_count += results.len();
                emitter.wiki_topics_changed();
            }
            Err(e) => {
                log::warn!("wiki worker: batch classification failed: {e}");
                {
                    let store = lock(&store);
                    for &(chat_id, message_id, _) in &item_map {
                        let _ = store.mark_queue_failed(chat_id, message_id, &e.to_string());
                    }
                }
                emitter.wiki_error(&e.to_string(), true);
            }
        }

        if processed_count.is_multiple_of(100) {
            let store = lock(&store);
            total_channels = store.get_total_active_channels().unwrap_or(1);
            let recovered = store.recover_stale_claims().unwrap_or(0);
            if recovered > 0 {
                log::info!("wiki worker: recovered {recovered} stale claims");
            }
        }

        if processed_count.is_multiple_of(50) {
            recalculate_trending(&store, total_channels);
        }

        emit_progress(&emitter, &store);

        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    recalculate_trending(&store, total_channels);
}

fn emit_progress<E: EventEmitter>(emitter: &Arc<E>, store: &Arc<Mutex<Store>>) {
    let stats: QueueStats = {
        let store = lock(store);
        store.get_queue_stats().unwrap_or(QueueStats {
            pending: 0,
            processing: 0,
            done: 0,
            failed: 0,
            skipped: 0,
        })
    };
    let nn = |n: i64| n.max(0) as u64;
    emitter.wiki_progress(
        nn(stats.done + stats.skipped),
        nn(stats.pending),
        nn(stats.done + stats.skipped + stats.pending + stats.failed),
    );
}

fn lock(store: &Arc<Mutex<Store>>) -> std::sync::MutexGuard<'_, Store> {
    store.lock().unwrap_or_else(|e| e.into_inner())
}

/// Apply one LLM topic assignment to the store. Extracted so it can
/// be called both by the worker and by potential replay tools.
pub fn process_classified_topic(
    store: &Store,
    classified: &ClassifiedTopic,
    chat_id: i64,
    message_id: i64,
    message_timestamp: i64,
) -> Result<(), sqlite::Error> {
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
            if let Some(fuzzy_id) = store.find_topic_fuzzy(&classified.topic)? {
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

fn recalculate_trending(store: &Arc<Mutex<Store>>, total_channels: i64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let topic_ids = {
        let store = lock(store);
        store.get_active_topic_ids(30).unwrap_or_default()
    };

    for topic_id in topic_ids {
        let store = lock(store);
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
