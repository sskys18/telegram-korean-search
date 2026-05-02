//! Wiki classification worker.
//!
//! Spawns a background thread that pulls items off the v2 classify
//! queue, asks the LLM to assign messages to v2 wiki pages, and writes
//! evidence back into the v2 wiki tables.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::store::wiki_page::{CandidatePage, NewEvidenceV2, PageRefV2};
use crate::store::wiki_queue::{ClassifyV2Item, QueueStats};
use crate::store::Store;
use crate::wiki::llm::{
    validate_v2_assignment, LlmClient, V2Assignment, V2ExistingPage, V2Input, V2InputMessage,
    V2PageRef, V2Policies,
};
use crate::wiki::norm::title_norm;

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
    let (batch_size, max_attempts) = {
        let s = lock(&store);
        (
            s.get_wiki_setting_i64("classify_batch_size", 20).max(1) as usize,
            s.get_wiki_setting_i64("max_classify_attempts", 3),
        )
    };

    {
        let s = lock(&store);
        match s.recover_stale_v2_claims() {
            Ok(n) if n > 0 => log::info!("wiki worker(v2): recovered {n} stale claims"),
            Err(e) => log::warn!("wiki worker(v2): recover_stale_v2_claims failed: {e}"),
            _ => {}
        }
    }

    loop {
        if shutdown.load(Ordering::Relaxed) {
            emitter.wiki_stopped("shutdown");
            break;
        }

        let items: Vec<ClassifyV2Item> = {
            let s = lock(&store);
            s.claim_classify_v2_batch(batch_size).unwrap_or_default()
        };

        if items.is_empty() {
            for _ in 0..20 {
                if shutdown.load(Ordering::Relaxed) || wake.load(Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            wake.store(false, Ordering::Relaxed);
            emit_progress_v2(&emitter, &store);
            continue;
        }

        struct Loaded {
            item: ClassifyV2Item,
            chat_title: String,
            text: String,
            ts: i64,
            sender_id: i64,
        }

        let loaded: Vec<Loaded> = {
            let s = lock(&store);
            items
                .into_iter()
                .filter_map(|item| {
                    let Some(m) = s.get_message(item.chat_id, item.msg_id).ok().flatten() else {
                        let _ = s.mark_classify_v2_done(item.msg_id, item.chat_id);
                        return None;
                    };
                    if m.text_plain.trim().is_empty() {
                        let _ = s.mark_classify_v2_done(item.msg_id, item.chat_id);
                        return None;
                    }
                    let chat_title = s
                        .get_chat(item.chat_id)
                        .ok()
                        .flatten()
                        .map(|c| c.title)
                        .unwrap_or_else(|| "Unknown".to_string());
                    Some(Loaded {
                        item,
                        chat_title,
                        text: m.text_plain,
                        ts: m.timestamp,
                        sender_id: m.sender_id,
                    })
                })
                .collect()
        };

        if loaded.is_empty() {
            emit_progress_v2(&emitter, &store);
            continue;
        }

        let mut tokens = Vec::new();
        let mut fts_terms = Vec::new();
        for l in &loaded {
            tokens.extend(candidate_tokens_from_text(&l.text));
            for word in l.text.split_whitespace().take(8) {
                if let Some(term) = fts_term(word) {
                    fts_terms.push(term);
                }
            }
        }
        tokens.sort();
        tokens.dedup();
        fts_terms.sort();
        fts_terms.dedup();
        let fts_query = fts_terms.join(" OR ");

        let candidates: Vec<CandidatePage> = {
            let s = lock(&store);
            s.classify_candidates_v2(&tokens, &fts_query, 30)
                .unwrap_or_default()
        };

        let existing: Vec<V2ExistingPage> = candidates
            .iter()
            .map(|c| V2ExistingPage {
                id: c.id,
                kind: c.kind.as_str(),
                title: c.title.as_str(),
                aliases: c.aliases.as_slice(),
            })
            .collect();
        let candidate_ids: std::collections::HashSet<i64> =
            candidates.iter().map(|c| c.id).collect();

        let messages_in: Vec<V2InputMessage> = loaded
            .iter()
            .map(|l| V2InputMessage {
                msg_id: l.item.msg_id,
                chat_id: l.item.chat_id,
                chat_title: l.chat_title.as_str(),
                sender: "",
                ts: l.ts,
                text: l.text.as_str(),
                hint_successor_for: l.item.hint_page_id,
            })
            .collect();

        let policies = V2Policies {
            max_pages_per_message: 3,
            skip_if_salience_below: 0.2,
            may_propose_new: true,
        };
        let input = V2Input {
            existing_pages: &existing,
            messages: &messages_in,
            policies: &policies,
        };

        let v2_out = match llm.classify_batch_v2(&input).await {
            Ok(o) => o,
            Err(e) => {
                log::warn!("wiki worker(v2): batch failed: {e}");
                let s = lock(&store);
                for l in &loaded {
                    let _ = s.mark_classify_v2_retry(
                        l.item.msg_id,
                        l.item.chat_id,
                        &e.to_string(),
                        max_attempts,
                    );
                }
                emitter.wiki_error(&e.to_string(), true);
                emit_progress_v2(&emitter, &store);
                continue;
            }
        };

        let mut by_msg: std::collections::HashMap<i64, Vec<V2Assignment>> =
            std::collections::HashMap::new();
        for ma in v2_out.assignments {
            by_msg.entry(ma.msg_id).or_default().extend(ma.assignments);
        }

        let mut applied = 0usize;
        for l in &loaded {
            let assignments = by_msg.remove(&l.item.msg_id).unwrap_or_default();
            let s = lock(&store);
            match apply_classify_v2(
                &s,
                ApplyClassifyV2 {
                    item: &l.item,
                    msg_text: &l.text,
                    ts: l.ts,
                    sender_id: l.sender_id,
                    assignments: &assignments,
                    candidate_ids: &candidate_ids,
                    max_attempts,
                },
            ) {
                Ok(true) => applied += 1,
                Ok(false) => {}
                Err(e) => {
                    log::warn!(
                        "wiki worker(v2): apply failed msg={} chat={}: {e}",
                        l.item.msg_id,
                        l.item.chat_id
                    );
                    let _ = s.mark_classify_v2_retry(
                        l.item.msg_id,
                        l.item.chat_id,
                        &e.to_string(),
                        max_attempts,
                    );
                }
            }
        }

        if applied > 0 {
            emitter.wiki_topics_changed();
        }
        emit_progress_v2(&emitter, &store);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn candidate_tokens_from_text(text: &str) -> Vec<String> {
    let words: Vec<String> = text
        .split_whitespace()
        .map(title_norm)
        .filter(|w| {
            let n = w.chars().count();
            (2..=40).contains(&n)
        })
        .collect();
    let mut out = Vec::new();
    for width in 1..=4 {
        for window in words.windows(width) {
            let token = window.join(" ");
            let n = token.chars().count();
            if (2..=80).contains(&n) {
                out.push(token);
            }
        }
    }
    out
}

fn fts_term(word: &str) -> Option<String> {
    let cleaned: String = word.chars().filter(|c| c.is_alphanumeric()).collect();
    if cleaned.chars().count() < 3 {
        return None;
    }
    Some(format!("\"{}\"", cleaned.replace('"', "\"\"")))
}

struct ApplyClassifyV2<'a> {
    item: &'a ClassifyV2Item,
    msg_text: &'a str,
    ts: i64,
    sender_id: i64,
    assignments: &'a [V2Assignment],
    candidate_ids: &'a std::collections::HashSet<i64>,
    max_attempts: i64,
}

/// Apply per-message classify result inside one transaction. Returns
/// `Ok(false)` when validation failed and the row was retried.
fn apply_classify_v2(store: &Store, input: ApplyClassifyV2<'_>) -> Result<bool, sqlite::Error> {
    if input.assignments.is_empty() {
        store.mark_classify_v2_done(input.item.msg_id, input.item.chat_id)?;
        return Ok(true);
    }

    let mut validated = Vec::with_capacity(input.assignments.len());
    for a in input.assignments {
        let cleaned = match validate_v2_assignment(a, input.msg_text, input.candidate_ids) {
            Ok(s) => s,
            Err(e) => {
                store.mark_classify_v2_retry(
                    input.item.msg_id,
                    input.item.chat_id,
                    &e.to_string(),
                    input.max_attempts,
                )?;
                return Ok(false);
            }
        };
        validated.push((a, cleaned));
    }

    store.conn().execute("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<bool, sqlite::Error> {
        let mut needs_successor = None;
        let mut any_succeeded = false;

        for (a, excerpt) in &validated {
            let page_ref: PageRefV2 = match &a.page_ref {
                V2PageRef::Existing { existing_id } => {
                    let mut st = store
                        .conn()
                        .prepare("SELECT state, kind FROM wiki_pages_v2 WHERE id = ?")?;
                    st.bind((1, *existing_id))?;
                    if let sqlite::State::Row = st.next()? {
                        PageRefV2 {
                            id: *existing_id,
                            state: st.read::<String, _>(0)?,
                            kind: st.read::<String, _>(1)?,
                        }
                    } else {
                        continue;
                    }
                }
                V2PageRef::New { new } => {
                    store.dedup_or_insert_page_v2(&new.kind, &new.title, &new.aliases)?
                }
            };

            match page_ref.state.as_str() {
                "frozen" | "hidden" => continue,
                "resolved" => {
                    needs_successor.get_or_insert(page_ref.id);
                    continue;
                }
                _ => {}
            }

            let salience = a.salience.clamp(0.0, 1.0);
            if store
                .insert_evidence_v2(&NewEvidenceV2 {
                    page_id: page_ref.id,
                    msg_id: input.item.msg_id,
                    chat_id: input.item.chat_id,
                    sender_id: input.sender_id,
                    ts: input.ts,
                    excerpt,
                    salience,
                })?
                .is_some()
            {
                any_succeeded = true;
            }
        }

        if let Some(hint) = needs_successor {
            if !any_succeeded {
                store.mark_classify_v2_successor_needed(
                    input.item.msg_id,
                    input.item.chat_id,
                    hint,
                )?;
                return Ok(true);
            }
        }
        store.mark_classify_v2_done(input.item.msg_id, input.item.chat_id)?;
        Ok(true)
    })();

    match result {
        Ok(v) => {
            store.conn().execute("COMMIT")?;
            Ok(v)
        }
        Err(e) => {
            let _ = store.conn().execute("ROLLBACK");
            Err(e)
        }
    }
}

fn emit_progress_v2<E: EventEmitter>(emitter: &Arc<E>, store: &Arc<Mutex<Store>>) {
    let stats = {
        let s = lock(store);
        s.get_classify_v2_stats().unwrap_or(QueueStats {
            pending: 0,
            processing: 0,
            done: 0,
            failed: 0,
            skipped: 0,
        })
    };
    let nn = |n: i64| n.max(0) as u64;
    let total = stats.done + stats.pending + stats.failed + stats.processing;
    emitter.wiki_progress(nn(stats.done), nn(stats.pending), nn(total));
}

fn lock(store: &Arc<Mutex<Store>>) -> std::sync::MutexGuard<'_, Store> {
    store.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::message::MessageRow;
    use crate::wiki::llm::{V2NewPage, V2PageRef};

    fn store_with_one_msg() -> Store {
        let s = Store::open_in_memory().unwrap();
        s.conn()
            .execute(
                "INSERT INTO chats (chat_id, title, chat_type) VALUES (1, 'Crypto', 'channel')",
            )
            .unwrap();
        s.insert_messages_batch(&[MessageRow {
            message_id: 100,
            chat_id: 1,
            timestamp: 1_700_000_000,
            text_plain: "Bitcoin ETF approved by SEC today".into(),
            text_stripped: "BitcoinETFapprovedbySECtoday".into(),
            link: None,
            sender_id: 42,
        }])
        .unwrap();
        s.conn()
            .execute("DELETE FROM wiki_classify_queue_v2")
            .unwrap();
        s
    }

    fn fake_item() -> ClassifyV2Item {
        ClassifyV2Item {
            msg_id: 100,
            chat_id: 1,
            attempts: 0,
            hint: None,
            hint_page_id: None,
            text_hash: vec![0; 16],
        }
    }

    fn insert_processing_row(s: &Store) {
        s.conn()
            .execute(
                "INSERT INTO wiki_classify_queue_v2
              (msg_id, chat_id, status, attempts, text_hash, enqueued_at, next_attempt_at)
             VALUES (100, 1, 'processing', 1, X'00', 0, 0)",
            )
            .unwrap();
    }

    #[test]
    fn apply_v2_empty_assignments_marks_done() {
        let s = store_with_one_msg();
        insert_processing_row(&s);
        let cset = std::collections::HashSet::new();
        let item = fake_item();
        apply_classify_v2(
            &s,
            ApplyClassifyV2 {
                item: &item,
                msg_text: "Bitcoin ETF approved by SEC today",
                ts: 0,
                sender_id: 42,
                assignments: &[],
                candidate_ids: &cset,
                max_attempts: 3,
            },
        )
        .unwrap();
        let stats = s.get_classify_v2_stats().unwrap();
        assert_eq!(stats.done, 1);
    }

    #[test]
    fn apply_v2_new_page_creates_evidence() {
        let s = store_with_one_msg();
        insert_processing_row(&s);
        let cset = std::collections::HashSet::new();
        let a = V2Assignment {
            page_ref: V2PageRef::New {
                new: V2NewPage {
                    kind: "topic".into(),
                    title: "Bitcoin ETF".into(),
                    aliases: vec!["BTC ETF".into()],
                },
            },
            excerpt: "Bitcoin ETF approved".into(),
            salience: 0.9,
        };
        let item = fake_item();
        apply_classify_v2(
            &s,
            ApplyClassifyV2 {
                item: &item,
                msg_text: "Bitcoin ETF approved by SEC today",
                ts: 1_700_000_000,
                sender_id: 42,
                assignments: std::slice::from_ref(&a),
                candidate_ids: &cset,
                max_attempts: 3,
            },
        )
        .unwrap();

        let mut q = s
            .conn()
            .prepare("SELECT COUNT(*) FROM wiki_evidence")
            .unwrap();
        q.next().unwrap();
        assert_eq!(q.read::<i64, _>(0).unwrap(), 1);
        let stats = s.get_classify_v2_stats().unwrap();
        assert_eq!(stats.done, 1);
    }

    #[test]
    fn apply_v2_excerpt_not_in_text_retries() {
        let s = store_with_one_msg();
        insert_processing_row(&s);
        let cset = std::collections::HashSet::new();
        let a = V2Assignment {
            page_ref: V2PageRef::New {
                new: V2NewPage {
                    kind: "topic".into(),
                    title: "X".into(),
                    aliases: vec![],
                },
            },
            excerpt: "TOTALLY HALLUCINATED".into(),
            salience: 0.5,
        };
        let item = fake_item();
        apply_classify_v2(
            &s,
            ApplyClassifyV2 {
                item: &item,
                msg_text: "Bitcoin ETF approved by SEC today",
                ts: 0,
                sender_id: 42,
                assignments: std::slice::from_ref(&a),
                candidate_ids: &cset,
                max_attempts: 3,
            },
        )
        .unwrap();
        let stats = s.get_classify_v2_stats().unwrap();
        assert_eq!(stats.pending + stats.failed, 1);
        assert_eq!(stats.done, 0);
    }
}
