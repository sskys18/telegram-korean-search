//! Wiki classification worker.
//!
//! Spawns a background thread that pulls items off the v2 classify
//! queue, asks the LLM to assign messages to v2 wiki pages, and writes
//! evidence back into the v2 wiki tables.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::store::wiki_page::{
    derive_reason_code, CandidatePage, NewEvidenceV2, PageRefV2, RewriteApply, TrendingApplyRow,
    TrendingCandidate, TrendingSnapshot, TrendingWindow,
};
use crate::store::wiki_queue::{ClassifyV2Item, QueueStats, RewriteQueueItem};
use crate::store::Store;
use crate::wiki::llm::{
    validate_trending, validate_v2_assignment, validate_v2_rewrite, LlmClient, V2Assignment,
    V2ExistingPage, V2Input, V2InputMessage, V2PageRef, V2Policies, V2RewriteEvidenceIn,
    V2RewriteInput, V2TrendingCandidateIn, V2TrendingInput,
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
    let (batch_size, max_attempts, max_rewrite_attempts, retention_cap, rewrite_hour_cap) = {
        let s = lock(&store);
        (
            s.get_wiki_setting_i64("classify_batch_size", 20).max(1) as usize,
            s.get_wiki_setting_i64("max_classify_attempts", 3),
            s.get_wiki_setting_i64("max_rewrite_attempts", 3),
            s.get_wiki_setting_i64("retention_evidence_per_page", 200)
                .max(1),
            s.get_wiki_setting_i64("rewrite_per_hour_cap", 30).max(0),
        )
    };
    // Sliding window of recent successful-or-attempted rewrite timestamps
    // (unix secs). Spec §5.2 caps codex calls; we count anything we'd have
    // sent to the LLM, including retries. Pre-classify-batch and idle-loop
    // sites both consult this before claiming.
    let mut rewrite_window: std::collections::VecDeque<i64> = std::collections::VecDeque::new();
    let rewrite_allowed = |window: &mut std::collections::VecDeque<i64>| -> bool {
        if rewrite_hour_cap == 0 {
            return false;
        }
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
            - 3600;
        while let Some(&t) = window.front() {
            if t < cutoff {
                window.pop_front();
            } else {
                break;
            }
        }
        (window.len() as i64) < rewrite_hour_cap
    };

    {
        let s = lock(&store);
        match s.recover_stale_v2_claims() {
            Ok(n) if n > 0 => log::info!("wiki worker(v2): recovered {n} stale claims"),
            Err(e) => log::warn!("wiki worker(v2): recover_stale_v2_claims failed: {e}"),
            _ => {}
        }
        match s.recover_stale_rewrite_claims() {
            Ok(n) if n > 0 => log::info!("wiki rewrite: recovered {n} stale rewrite claims"),
            Err(e) => log::warn!("wiki rewrite: recover_stale_rewrite_claims failed: {e}"),
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
            // Idle classify queue → drain rewrites instead of sleeping,
            // gated by `rewrite_per_hour_cap` (spec §5.2).
            let rewrites: Vec<RewriteQueueItem> = if rewrite_allowed(&mut rewrite_window) {
                let s = lock(&store);
                s.claim_rewrite_batch(1).unwrap_or_default()
            } else {
                Vec::new()
            };
            if !rewrites.is_empty() {
                let mut applied = 0usize;
                for item in &rewrites {
                    rewrite_window.push_back(crate::wiki::norm::unix_now());
                    match process_rewrite_one(
                        &store,
                        &llm,
                        &emitter,
                        item,
                        max_rewrite_attempts,
                        retention_cap,
                    )
                    .await
                    {
                        Ok(true) => applied += 1,
                        Ok(false) => {}
                        Err(e) => {
                            log::warn!("wiki rewrite: page={} apply error: {e}", item.page_id);
                        }
                    }
                }
                if applied > 0 {
                    emitter.wiki_topics_changed();
                }
                emit_progress_v2(&emitter, &store);
                continue;
            }
            // Idle path: piggyback one trending refresh per tick (spec §6.4).
            // Picks the most-overdue dirty window; clean ticks are cheap
            // (one MAX(id) + 3 watermark reads).
            if let Err(e) = maybe_refresh_trending(&store, &llm, &emitter).await {
                log::warn!("wiki trending: refresh failed: {e}");
            }
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

        // Spec §9 weighted fair share: piggyback one rewrite per classify
        // batch so rewrites can't starve when ingest is hot. Hourly cap
        // (spec §5.2 `rewrite_per_hour_cap`) gates the claim.
        let rewrites: Vec<RewriteQueueItem> = if rewrite_allowed(&mut rewrite_window) {
            let s = lock(&store);
            s.claim_rewrite_batch(1).unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut rewrite_applied = 0usize;
        for item in &rewrites {
            rewrite_window.push_back(crate::wiki::norm::unix_now());
            match process_rewrite_one(
                &store,
                &llm,
                &emitter,
                item,
                max_rewrite_attempts,
                retention_cap,
            )
            .await
            {
                Ok(true) => rewrite_applied += 1,
                Ok(false) => {}
                Err(e) => {
                    log::warn!("wiki rewrite: page={} apply error: {e}", item.page_id);
                }
            }
        }
        if rewrite_applied > 0 {
            emitter.wiki_topics_changed();
        }

        // Spec §6.4 trending refresh tail-call: classify ingest is the only
        // thing that bumps `wiki_evidence.id`, so this is the natural place
        // to check whether a window has gone dirty. Cheap when nothing is
        // overdue (one MAX(id) + 3 watermark reads).
        if let Err(e) = maybe_refresh_trending(&store, &llm, &emitter).await {
            log::warn!("wiki trending: refresh failed: {e}");
        }

        emit_progress_v2(&emitter, &store);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Pick the most-overdue dirty + gap-eligible window, if any. Returns
/// `(window, max_evidence_id)` snapshotted once. `None` means nothing to do.
fn pick_dirty_window(
    store: &Store,
    now: i64,
) -> Result<Option<(TrendingWindow, i64)>, sqlite::Error> {
    let max_id = store.current_max_evidence_id()?;
    if max_id == 0 {
        return Ok(None);
    }
    let mut best: Option<(TrendingWindow, i64)> = None;
    let mut best_overdue: i64 = i64::MIN;
    for w in TrendingWindow::all() {
        let (last_id, last_ts) = store.read_trending_watermark(w)?;
        // Strict `>` per spec §6.4: clean iff watermark caught up.
        if max_id <= last_id {
            continue;
        }
        let gap = w.min_refresh_gap_secs();
        // Never-computed watermark (last_ts = 0) is most-overdue by far.
        let overdue = if last_ts == 0 {
            i64::MAX / 2
        } else {
            now - last_ts - gap
        };
        if overdue < 0 {
            continue;
        }
        if overdue > best_overdue {
            best_overdue = overdue;
            best = Some((w, max_id));
        }
    }
    Ok(best)
}

/// Run one trending refresh tick if a dirty + gap-eligible window exists.
/// Builds shortlist + per-page metrics + sparkline, calls codex rerank,
/// validates, applies (or fallback to shortlist top-10 with empty hooks).
async fn maybe_refresh_trending<E: EventEmitter>(
    store: &Arc<Mutex<Store>>,
    llm: &LlmClient,
    emitter: &Arc<E>,
) -> Result<(), sqlite::Error> {
    let now = crate::wiki::norm::unix_now();
    let pick = {
        let s = lock(store);
        pick_dirty_window(&s, now)?
    };
    let Some((window, max_id)) = pick else {
        return Ok(());
    };
    let snap = TrendingSnapshot {
        window,
        window_start: now - window.span_secs(),
        prior_start: now - 2 * window.span_secs(),
        now,
        max_evidence_id: max_id,
    };

    // Shortlist + per-candidate enrichment under a single lock.
    struct Enriched {
        cand: TrendingCandidate,
        reason_code: String,
        reason_metrics: String,
        sparkline: String,
        samples: Vec<String>,
    }
    let enriched: Vec<Enriched> = {
        let s = lock(store);
        let cands = s.shortlist_trending(&snap, 30)?;
        let mut out = Vec::with_capacity(cands.len());
        for c in cands {
            let (code, metrics) = derive_reason_code(&c, snap.now);
            let buckets = s.compute_sparkline(c.page_id, &snap)?;
            let sparkline =
                serde_json::to_string(&buckets.to_vec()).unwrap_or_else(|_| "[]".to_string());
            let samples = s.trending_sample_excerpts(c.page_id, &snap, 3)?;
            out.push(Enriched {
                cand: c,
                reason_code: code,
                reason_metrics: metrics,
                sparkline,
                samples,
            });
        }
        out
    };

    if enriched.is_empty() {
        // Empty window: still bump watermark so we don't recompute the
        // same emptiness every tick.
        let s = lock(store);
        s.conn().execute("BEGIN IMMEDIATE")?;
        let res = s.apply_trending(&snap, &[]);
        match res {
            Ok(()) => {
                s.conn().execute("COMMIT")?;
            }
            Err(e) => {
                let _ = s.conn().execute("ROLLBACK");
                return Err(e);
            }
        }
        return Ok(());
    }

    // Build codex input.
    let candidate_ids: std::collections::HashSet<i64> =
        enriched.iter().map(|e| e.cand.page_id).collect();
    let metric_values: Vec<serde_json::Value> = enriched
        .iter()
        .map(|e| {
            serde_json::from_str::<serde_json::Value>(&e.reason_metrics)
                .unwrap_or(serde_json::Value::Null)
        })
        .collect();
    let candidates_in: Vec<V2TrendingCandidateIn<'_>> = enriched
        .iter()
        .zip(metric_values.iter())
        .map(|(e, m)| V2TrendingCandidateIn {
            page_id: e.cand.page_id,
            title: e.cand.title.as_str(),
            kind: e.cand.kind.as_str(),
            reason_code: e.reason_code.as_str(),
            metrics: m,
            samples: &e.samples,
        })
        .collect();
    let input = V2TrendingInput {
        window: window.label(),
        candidates: &candidates_in,
    };

    // Call codex; on any failure (exec or validator) we still write the
    // shortlist top-10 with empty hooks and bump the watermark — spec §6.4
    // line 849. Hot-loop guard.
    let validated = match llm.rerank_trending(&input).await {
        Ok(out) => match validate_trending(&out, &candidate_ids) {
            Ok(v) => Some(v),
            Err(e) => {
                log::warn!(
                    "wiki trending: validator fallback ({}): {e}",
                    window.label()
                );
                None
            }
        },
        Err(e) => {
            log::warn!("wiki trending: rerank failed ({}): {e}", window.label());
            emitter.wiki_error(&e.to_string(), true);
            None
        }
    };

    let rows: Vec<TrendingApplyRow> = if let Some(ranked) = validated {
        // Map ranked → enriched by page_id so reason_code/metrics/sparkline
        // come from local computation, never the LLM.
        ranked
            .into_iter()
            .filter_map(|r| {
                enriched
                    .iter()
                    .find(|e| e.cand.page_id == r.page_id)
                    .map(|e| TrendingApplyRow {
                        page_id: r.page_id,
                        rank: r.rank,
                        hook: r.hook,
                        reason_code: e.reason_code.clone(),
                        reason_metrics: e.reason_metrics.clone(),
                        sparkline: e.sparkline.clone(),
                    })
            })
            .collect()
    } else {
        enriched
            .iter()
            .take(10)
            .enumerate()
            .map(|(i, e)| TrendingApplyRow {
                page_id: e.cand.page_id,
                rank: (i as i64) + 1,
                hook: String::new(),
                reason_code: e.reason_code.clone(),
                reason_metrics: e.reason_metrics.clone(),
                sparkline: e.sparkline.clone(),
            })
            .collect()
    };

    // Atomic apply + watermark bump.
    let s = lock(store);
    s.conn().execute("BEGIN IMMEDIATE")?;
    match s.apply_trending(&snap, &rows) {
        Ok(()) => {
            s.conn().execute("COMMIT")?;
            emitter.wiki_topics_changed();
            Ok(())
        }
        Err(e) => {
            let _ = s.conn().execute("ROLLBACK");
            Err(e)
        }
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
        let mut touched_pages = std::collections::HashSet::<i64>::new();

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
                touched_pages.insert(page_ref.id);
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
        // Spec §6.3: trigger lives inside classify txn, idempotent via PK.
        for pid in &touched_pages {
            store.maybe_enqueue_rewrite(*pid)?;
        }
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

/// Drive one rewrite job end-to-end: load page + evidence, run codex,
/// validate, apply in a single transaction. Returns `Ok(true)` on apply,
/// `Ok(false)` when the page disappeared (no work done; queue marked done),
/// `Err` propagates DB issues.
async fn process_rewrite_one<E: EventEmitter>(
    store: &Arc<Mutex<Store>>,
    llm: &LlmClient,
    emitter: &Arc<E>,
    item: &RewriteQueueItem,
    max_attempts: i64,
    retention_cap: i64,
) -> Result<bool, sqlite::Error> {
    let page = {
        let s = lock(store);
        s.get_page_for_rewrite(item.page_id)?
    };
    let Some(page) = page else {
        let s = lock(store);
        s.mark_rewrite_done(item.page_id)?;
        return Ok(false);
    };
    if matches!(page.state.as_str(), "frozen" | "hidden") {
        let s = lock(store);
        s.mark_rewrite_done(item.page_id)?;
        return Ok(false);
    }

    let evidence_result = {
        let s = lock(store);
        s.select_rewrite_evidence(item.page_id, page.last_rewrite_max_evidence_id)
    };
    let (evidence, snapshot_at, max_evidence_id) = match evidence_result {
        Ok(v) => v,
        Err(e) => {
            // DB error reading evidence — retry, do NOT mark done (was
            // silently dropping the page on transient SQL errors).
            log::warn!(
                "wiki rewrite: page={} select_rewrite_evidence failed: {e}",
                item.page_id
            );
            let s = lock(store);
            s.mark_rewrite_retry(item.page_id, &e.to_string(), max_attempts)?;
            return Ok(false);
        }
    };
    if evidence.is_empty() {
        // Nothing to summarize; close the queue row without pinging codex.
        let s = lock(store);
        s.mark_rewrite_done(item.page_id)?;
        return Ok(false);
    }

    let prior_facts: Option<serde_json::Value> = page
        .facts
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    let evidence_in: Vec<V2RewriteEvidenceIn<'_>> = evidence
        .iter()
        .map(|e| V2RewriteEvidenceIn {
            id: e.id,
            ts: e.ts,
            excerpt: e.excerpt.as_str(),
            salience: e.salience,
            cited: e.cited,
        })
        .collect();
    let input = V2RewriteInput {
        page_id: page.id,
        kind: page.kind.as_str(),
        title: page.title.as_str(),
        state: page.state.as_str(),
        prior_summary_md: page.summary_md.as_str(),
        prior_facts: prior_facts.as_ref(),
        evidence: &evidence_in,
    };

    let raw_out = match llm.rewrite_page(&input).await {
        Ok(o) => o,
        Err(e) => {
            log::warn!("wiki rewrite: page={} llm failed: {e}", item.page_id);
            let s = lock(store);
            s.mark_rewrite_retry(item.page_id, &e.to_string(), max_attempts)?;
            emitter.wiki_error(&e.to_string(), true);
            return Ok(false);
        }
    };

    let validated = match validate_v2_rewrite(&raw_out, &page.state, &page.kind) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("wiki rewrite: page={} validation failed: {e}", item.page_id);
            let s = lock(store);
            s.mark_rewrite_retry(item.page_id, &e.to_string(), max_attempts)?;
            return Ok(false);
        }
    };

    let s = lock(store);
    s.conn().execute("BEGIN IMMEDIATE")?;
    let apply = s.apply_rewrite_v2(&RewriteApply {
        page_id: page.id,
        summary_md: &validated.summary_md,
        facts_json: &validated.facts_json,
        state: &validated.state,
        new_aliases: &validated.new_aliases,
        retention_cap,
        snapshot_at,
        max_evidence_id,
    });
    match apply {
        Ok(()) => {
            s.conn().execute("COMMIT")?;
            Ok(true)
        }
        Err(e) => {
            let _ = s.conn().execute("ROLLBACK");
            let _ = s.mark_rewrite_retry(item.page_id, &e.to_string(), max_attempts);
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

    // ---- Phase 8 trending dirty-window picker -----------------------------

    fn make_trending_store_with_evidence() -> Store {
        let s = Store::open_in_memory().unwrap();
        s.conn()
            .execute("INSERT INTO chats (chat_id, title, chat_type) VALUES (1, 'C', 'channel')")
            .unwrap();
        s.conn().execute("BEGIN").unwrap();
        let p = s.dedup_or_insert_page_v2("topic", "T", &[]).unwrap();
        s.insert_evidence_v2(&NewEvidenceV2 {
            page_id: p.id,
            msg_id: 1,
            chat_id: 1,
            sender_id: 0,
            ts: crate::wiki::norm::unix_now() - 100,
            excerpt: "x",
            salience: 0.5,
        })
        .unwrap();
        s.conn().execute("COMMIT").unwrap();
        s
    }

    #[test]
    fn pick_dirty_returns_none_for_empty_evidence() {
        let s = Store::open_in_memory().unwrap();
        let pick = pick_dirty_window(&s, crate::wiki::norm::unix_now()).unwrap();
        assert!(pick.is_none());
    }

    #[test]
    fn pick_dirty_picks_never_computed_window() {
        let s = make_trending_store_with_evidence();
        let now = crate::wiki::norm::unix_now();
        let pick = pick_dirty_window(&s, now).unwrap();
        assert!(pick.is_some());
        let (_w, max_id) = pick.unwrap();
        assert!(max_id > 0);
    }

    #[test]
    fn pick_dirty_skips_when_caught_up() {
        let s = make_trending_store_with_evidence();
        let now = crate::wiki::norm::unix_now();
        let max_id = s.current_max_evidence_id().unwrap();
        // Pretend every window has caught up to max_id.
        let snap_h1 = TrendingSnapshot {
            window: TrendingWindow::H1,
            window_start: now - 3600,
            prior_start: now - 7200,
            now,
            max_evidence_id: max_id,
        };
        let snap_h24 = TrendingSnapshot {
            window: TrendingWindow::H24,
            window_start: now - 86400,
            prior_start: now - 2 * 86400,
            now,
            max_evidence_id: max_id,
        };
        let snap_d7 = TrendingSnapshot {
            window: TrendingWindow::D7,
            window_start: now - 7 * 86400,
            prior_start: now - 14 * 86400,
            now,
            max_evidence_id: max_id,
        };
        for snap in [&snap_h1, &snap_h24, &snap_d7] {
            s.conn().execute("BEGIN IMMEDIATE").unwrap();
            s.apply_trending(snap, &[]).unwrap();
            s.conn().execute("COMMIT").unwrap();
        }
        // No new evidence → all clean.
        let pick = pick_dirty_window(&s, now + 100_000).unwrap();
        assert!(pick.is_none(), "all windows caught up should yield None");
    }

    #[test]
    fn pick_dirty_respects_gap_eligibility() {
        let s = make_trending_store_with_evidence();
        let now = 100_000_i64;
        // Compute 1h watermark just now with old max_id, but leave 24h alone.
        let snap_h1 = TrendingSnapshot {
            window: TrendingWindow::H1,
            window_start: now - 3600,
            prior_start: now - 7200,
            now,
            max_evidence_id: 0,
        };
        s.conn().execute("BEGIN IMMEDIATE").unwrap();
        s.apply_trending(&snap_h1, &[]).unwrap();
        s.conn().execute("COMMIT").unwrap();
        // Insert another evidence so max_id > all watermarks again.
        s.conn().execute("BEGIN").unwrap();
        s.insert_evidence_v2(&NewEvidenceV2 {
            page_id: 1,
            msg_id: 99,
            chat_id: 1,
            sender_id: 0,
            ts: now - 50,
            excerpt: "y",
            salience: 0.5,
        })
        .unwrap();
        s.conn().execute("COMMIT").unwrap();
        // 1h gap = 300s; only 1s elapsed → 1h ineligible. 24h+7d eligible.
        let pick = pick_dirty_window(&s, now + 1).unwrap();
        let (w, _) = pick.unwrap();
        assert_ne!(w, TrendingWindow::H1, "1h should be gap-blocked");
    }
}
