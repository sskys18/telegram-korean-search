//! Public FFI surface exposed to the Swift shell via UniFFI.
//!
//! Everything Swift is allowed to touch lives here. The rest of the
//! crate (`store`, `search`, `wiki`, `ipc`, `security`) is internal
//! and cannot be imported from the Swift side. Keeping a single
//! surface module makes it easy to see exactly what the shell can
//! call and to evolve the binding without rewriting scaffolding.
//!
//! ### Shape
//!
//! `Seoyu` is a UniFFI `Object` (interface-like, reference-counted
//! on the Swift side). Its methods operate on a shared
//! `Arc<Mutex<Store>>` so calls from multiple Swift threads do not
//! corrupt SQLite. Inputs and outputs that cross the boundary are
//! UniFFI `Record`s — plain structs with `derive(uniffi::Record)`
//! that UniFFI marshals to matching Swift structs.
//!
//! ### Errors
//!
//! All fallible methods return `Result<T, SeoyuError>`. UniFFI
//! generates a Swift `throws` signature so callers use idiomatic
//! `try` / `catch`. Do not panic across the FFI boundary; convert to
//! an error instead.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::search::{engine, SearchResult as CoreSearchResult};
use crate::store::chat::ChatRow;
use crate::store::message::{
    strip_whitespace, Cursor, IndexOutcome as CoreIndexOutcome, MessageRef as CoreMessageRef,
    MessageRow,
};
use crate::store::wiki_page::{AskEvidence, AskPage};
use crate::store::Store;
use crate::wiki::llm::{
    parse_ask_stream, resolve_ask_model, strip_citation_markers, validate_cites, AskEvidenceIn,
    AskInput, AskRunError, AskRunState, LlmClient,
};
use crate::wiki::worker::WorkerHandle;

/// Swift-facing errors. Anything that went wrong inside the crate is
/// flattened into one of these three variants. The actual backtrace
/// is logged server-side; the Swift side only ever sees the message.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SeoyuError {
    /// SQLite, filesystem, or schema migration failure.
    #[error("store error: {0}")]
    Store(String),

    /// Caller passed something the core rejected: invalid cursor,
    /// empty query, missing topic, etc.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// Bucket for anything that doesn't fit above.
    #[error("{0}")]
    Other(String),
}

impl From<sqlite::Error> for SeoyuError {
    fn from(e: sqlite::Error) -> Self {
        SeoyuError::Store(e.to_string())
    }
}

// ---------- Records crossed over the FFI boundary ----------

#[derive(uniffi::Record, Clone)]
pub struct IndexedMessage {
    pub chat_id: i64,
    pub message_id: i64,
    pub timestamp: i64,
    pub text: String,
    pub link: Option<String>,
    pub sender_id: i64,
}

#[derive(uniffi::Record, Clone)]
pub struct IndexOutcome {
    pub inserted: u64,
    pub updated: u64,
}

#[derive(uniffi::Record, Clone)]
pub struct MessageRef {
    pub chat_id: i64,
    pub message_id: i64,
}

#[derive(uniffi::Record, Clone)]
pub struct ChatInfo {
    pub chat_id: i64,
    pub title: String,
    pub chat_type: String,
    pub username: Option<String>,
    pub access_hash: Option<i64>,
    pub is_excluded: bool,
}

#[derive(uniffi::Record, Clone)]
pub struct SearchHit {
    pub chat_id: i64,
    pub message_id: i64,
    pub timestamp: i64,
    pub text: String,
    pub link: Option<String>,
    pub chat_title: String,
    pub highlight_starts: Vec<u32>,
    pub highlight_ends: Vec<u32>,
}

#[derive(uniffi::Record, Clone)]
pub struct SearchPage {
    pub items: Vec<SearchHit>,
    pub next_cursor: Option<SearchCursor>,
}

#[derive(uniffi::Record, Clone)]
pub struct SearchCursor {
    pub rank: f64,
    pub timestamp: i64,
    pub chat_id: i64,
    pub message_id: i64,
}

#[derive(uniffi::Enum, Clone)]
pub enum SearchScope {
    All,
    Chat { chat_id: i64 },
}

#[derive(uniffi::Record, Clone)]
pub struct WikiTopicSummary {
    pub id: i64,
    pub title: String,
    pub title_ko: Option<String>,
    pub category: String,
    pub message_count: i64,
    pub trending_score: f64,
}

#[derive(uniffi::Record, Clone)]
pub struct WikiTopicDetail {
    pub summary: WikiTopicSummary,
    pub article_md: Option<String>,
    pub article_md_ko: Option<String>,
}

#[derive(uniffi::Record, Clone)]
pub struct WikiDigest {
    pub date_ymd: String,
    pub topic_count: i64,
    pub message_count: i64,
    pub hot_topics: Vec<WikiTopicSummary>,
}

/// Phase 8 trending row (spec §6.4). Cached per window in
/// `trending_cache`; populated atomically by the wiki worker.
#[derive(uniffi::Record, Clone)]
pub struct WikiTrendingRow {
    pub page_id: i64,
    pub rank: i64,
    pub kind: String,
    pub title: String,
    pub hook: String,
    pub reason_code: String,
    pub reason_metrics: String,
    pub sparkline: String,
    pub computed_at: i64,
}

/// Phase 8 pinned trending row. Pinned pages live in their own UI
/// slot above the ranked list; no cached entry — computed on read.
#[derive(uniffi::Record, Clone)]
pub struct WikiPinnedTrendingRow {
    pub page_id: i64,
    pub kind: String,
    pub title: String,
    pub ec: i64,
    pub last_ts: i64,
    pub sparkline: String,
}

/// Phase 9 digest row (spec §6.5). One per (chat, page) pair where
/// the post-`wiki_last_open` evidence count crossed the threshold.
#[derive(uniffi::Record, Clone)]
pub struct WikiDigestRow {
    pub chat_id: i64,
    pub page_id: i64,
    pub kind: String,
    pub state: String,
    pub title: String,
    pub n: i64,
    pub last_ts: i64,
}

#[derive(uniffi::Record, Clone)]
pub struct WikiCategory {
    pub id: i64,
    pub name: String,
    pub name_ko: Option<String>,
    pub topic_count: i64,
}

#[uniffi::export(with_foreign)]
pub trait WikiObserver: Send + Sync {
    fn on_progress(&self, processed: u64, pending: u64, total: u64);
    fn on_error(&self, message: String, recoverable: bool);
    fn on_topics_changed(&self);
}

/// One evidence row presented to the UI as a citable source. `source_id`
/// is the 1-based presentation index — the LLM only ever sees this id,
/// so unknown cites can be stripped before any character renders.
#[derive(uniffi::Record, Clone, Debug)]
pub struct EvidenceSummary {
    pub source_id: u32,
    pub evidence_id: i64,
    pub page_id: i64,
    pub page_title: String,
    pub chat_id: i64,
    pub chat_title: String,
    pub msg_id: i64,
    pub sender_id: i64,
    pub ts: i64,
    pub excerpt: String,
}

/// Spec §6.6 streaming handler. Callbacks for a single ask are
/// serialized — the sidecar dispatches every invocation through the
/// macOS main queue (libdispatch `dispatch_async_f` on
/// `_dispatch_main_q`), so Swift implementations may touch AppKit
/// directly without their own `DispatchQueue.main.async` hop. Spec §7
/// threading contract.
///
/// Per-segment delivery order: `on_delta` first, then 0+ `on_source`
/// for that segment's validated cites. Segments with no valid cites
/// still emit `on_delta`. Terminal callback is exactly one of
/// `on_finished` / `on_cancelled` / `on_error`.
///
/// Cancellation is explicit only: Swift calls `wiki_cancel_ask(id)`.
/// Spec §7's "implicit cancel on Arc drop" is not implementable at
/// this UniFFI binding shape — the foreign-trait Arc the Rust side
/// receives is independent of any Swift wrapper lifetime, so we have
/// no way to observe a Swift-side release. Documented + deferred.
#[uniffi::export(with_foreign)]
pub trait AskStreamHandler: Send + Sync {
    fn on_delta(&self, segment_index: u32, text: String);
    fn on_source(&self, segment_index: u32, tag: u32, source: EvidenceSummary);
    fn on_finished(&self, ask_id: i64);
    fn on_cancelled(&self, ask_id: i64);
    fn on_error(&self, ask_id: i64, message: String);
}

/// Hop a closure to the macOS main dispatch queue. Swift impls of
/// `AskStreamHandler` can therefore touch AppKit directly without a
/// per-callsite `DispatchQueue.main.async`. Tests + non-macos targets
/// fall back to direct invocation: cargo test runs without an active
/// main queue, so dispatching there would queue forever.
#[cfg(all(target_os = "macos", not(test)))]
fn dispatch_to_main<F: FnOnce() + Send + 'static>(f: F) {
    use std::ffi::c_void;
    #[link(name = "System", kind = "framework")]
    extern "C" {
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(queue: *mut c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
    }
    let boxed: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(f));
    let ctx = Box::into_raw(boxed) as *mut c_void;
    extern "C" fn trampoline(ctx: *mut c_void) {
        // SAFETY: trampoline owns the box round-trip; Box::from_raw reclaims it.
        let f: Box<Box<dyn FnOnce() + Send>> = unsafe { Box::from_raw(ctx as *mut _) };
        f();
    }
    unsafe {
        dispatch_async_f(
            &_dispatch_main_q as *const _ as *mut c_void,
            ctx,
            trampoline,
        );
    }
}

#[cfg(any(not(target_os = "macos"), test))]
fn dispatch_to_main<F: FnOnce() + Send + 'static>(f: F) {
    f();
}

/// Worker-side wrapper around `Arc<dyn AskStreamHandler>`. Each call
/// clones the strong Arc into a closure submitted to the main dispatch
/// queue. Holding a strong ref is required: UniFFI's foreign-trait
/// binding hands the Rust side an `Arc` that is independent of any
/// Swift-side wrapper lifetime — downgrading to Weak would make the
/// handler vanish from under the worker thread the moment our function
/// returns, even if Swift believes it still has the trait alive.
///
/// **Implicit drop-cancel (spec §7) is therefore not implementable at
/// this binding shape.** Swift cancels via the explicit
/// `wiki_cancel_ask(id)` API. A future revisit would require either
/// an extra "release" UniFFI callback Swift invokes from `deinit`, or
/// codegen changes to share the strong-count across the FFI boundary.
struct AskDispatch {
    handler: Arc<dyn AskStreamHandler>,
}

impl AskDispatch {
    fn from_arc(handler: Arc<dyn AskStreamHandler>) -> Self {
        Self { handler }
    }
    fn on_delta(&self, seg: u32, text: String) {
        let h = Arc::clone(&self.handler);
        dispatch_to_main(move || h.on_delta(seg, text));
    }
    fn on_source(&self, seg: u32, tag: u32, src: EvidenceSummary) {
        let h = Arc::clone(&self.handler);
        dispatch_to_main(move || h.on_source(seg, tag, src));
    }
    fn on_finished(&self, ask_id: i64) {
        let h = Arc::clone(&self.handler);
        dispatch_to_main(move || h.on_finished(ask_id));
    }
    fn on_cancelled(&self, ask_id: i64) {
        let h = Arc::clone(&self.handler);
        dispatch_to_main(move || h.on_cancelled(ask_id));
    }
    fn on_error(&self, ask_id: i64, message: String) {
        let h = Arc::clone(&self.handler);
        dispatch_to_main(move || h.on_error(ask_id, message));
    }
}

// ---------- The Swift-facing `Seoyu` object ----------

/// Root handle the Swift shell keeps for the lifetime of the app.
/// One instance per database file is expected; UniFFI will drop it
/// when Swift releases its last reference.
#[derive(uniffi::Object)]
pub struct Seoyu {
    store: Arc<Mutex<Store>>,
    wiki_worker: Mutex<Option<WorkerHandle>>,
    wiki_observer: Arc<Mutex<Option<Arc<dyn WikiObserver>>>>,
    wiki_wake: Arc<AtomicBool>,
    /// Map of ask_id → cancellation state. Lives only while the ask is
    /// in flight; the worker thread removes its entry in a `finally`-
    /// like guard before exiting. `wiki_cancel_ask` looks up by id and
    /// flips `cancelled`, then sends SIGTERM to the codex pid if known.
    active_asks: Arc<Mutex<HashMap<i64, Arc<AskRunState>>>>,
}

#[uniffi::export]
impl Seoyu {
    /// Open (or create) the sqlite store at `db_path` and run any
    /// pending migrations. Subsequent calls on the returned object
    /// share the same connection, so a single Seoyu must only be
    /// used from a consistent logical owner.
    #[uniffi::constructor]
    pub fn new(db_path: String) -> Result<Arc<Self>, SeoyuError> {
        let path = std::path::PathBuf::from(db_path);
        let store = Store::open(&path)?;
        Ok(Arc::new(Seoyu {
            store: Arc::new(Mutex::new(store)),
            wiki_worker: Mutex::new(None),
            wiki_observer: Arc::new(Mutex::new(None)),
            wiki_wake: Arc::new(AtomicBool::new(false)),
            active_asks: Arc::new(Mutex::new(HashMap::new())),
        }))
    }

    /// Trivial health check. Returns the crate version so the shell
    /// can verify it opened a binary it knows how to talk to.
    pub fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// Insert (or upsert) a chat before indexing messages that
    /// reference it. Required because the store's foreign-key
    /// discipline would otherwise reject messages for unknown chats.
    pub fn upsert_chat(&self, chat: ChatInfo) -> Result<(), SeoyuError> {
        let store = self.lock_store();
        store.upsert_chat(&ChatRow {
            chat_id: chat.chat_id,
            title: chat.title,
            chat_type: chat.chat_type,
            username: chat.username,
            access_hash: chat.access_hash,
            is_excluded: chat.is_excluded,
        })?;
        Ok(())
    }

    /// Mirror a batch of messages into the local store, updating FTS
    /// rows for edited text and returning accurate insert/update counts.
    pub fn index_messages(
        &self,
        messages: Vec<IndexedMessage>,
    ) -> Result<IndexOutcome, SeoyuError> {
        if messages.is_empty() {
            return Ok(IndexOutcome {
                inserted: 0,
                updated: 0,
            });
        }
        let rows: Vec<MessageRow> = messages
            .into_iter()
            .map(|m| MessageRow {
                message_id: m.message_id,
                chat_id: m.chat_id,
                timestamp: m.timestamp,
                text_plain: m.text.clone(),
                text_stripped: strip_whitespace(&m.text),
                link: m.link,
                sender_id: m.sender_id,
            })
            .collect();
        let store = self.lock_store();
        Ok(to_index_outcome(store.insert_messages_batch(&rows)?))
    }

    pub fn delete_messages(&self, refs: Vec<MessageRef>) -> Result<u64, SeoyuError> {
        if refs.is_empty() {
            return Ok(0);
        }
        let core_refs: Vec<CoreMessageRef> = refs
            .into_iter()
            .map(|r| CoreMessageRef {
                chat_id: r.chat_id,
                message_id: r.message_id,
            })
            .collect();
        let store = self.lock_store();
        Ok(store.delete_messages(&core_refs)?)
    }

    /// Run the Korean-aware query planner. Passing `limit = 0` means
    /// "use the crate default"; any other value is used verbatim.
    pub fn search(
        &self,
        query: String,
        scope: SearchScope,
        limit: u32,
        cursor: Option<SearchCursor>,
    ) -> Result<SearchPage, SeoyuError> {
        let core_scope = match scope {
            SearchScope::All => engine::SearchScope::All,
            SearchScope::Chat { chat_id } => engine::SearchScope::Chat(chat_id),
        };
        let core_cursor = cursor.as_ref().map(|c| Cursor {
            rank: c.rank,
            timestamp: c.timestamp,
            chat_id: c.chat_id,
            message_id: c.message_id,
        });
        let limit_opt = if limit == 0 {
            None
        } else {
            Some(limit as usize)
        };
        let store = self.lock_store();
        let result = engine::search(&store, &query, &core_scope, core_cursor.as_ref(), limit_opt)?;
        Ok(to_search_page(result))
    }

    /// Top trending topics, optionally filtered by a category name
    /// (case-insensitive). Missing categories return an empty list
    /// rather than erroring so the UI can ignore stale filters.
    pub fn wiki_trending(
        &self,
        limit: u32,
        offset: u32,
        category: Option<String>,
    ) -> Result<Vec<WikiTopicSummary>, SeoyuError> {
        let store = self.lock_store();
        let category_id = match category.as_deref() {
            Some(name) => {
                let all = store.get_all_categories()?;
                match all.iter().find(|c| c.name.eq_ignore_ascii_case(name)) {
                    Some(c) => Some(c.category_id),
                    None => return Ok(Vec::new()),
                }
            }
            None => None,
        };
        let topics = store.get_trending_topics(limit as usize, offset as usize, category_id)?;
        Ok(topics.into_iter().map(wiki_topic_to_summary).collect())
    }

    /// Topic + latest bilingual page. Returns `None` for unknown ids
    /// so the UI can render a placeholder without catching an error.
    pub fn wiki_topic_detail(&self, topic_id: i64) -> Result<Option<WikiTopicDetail>, SeoyuError> {
        let store = self.lock_store();
        let topic = match store.get_topic(topic_id)? {
            Some(t) => t,
            None => return Ok(None),
        };
        let page = store.get_latest_page(topic_id)?;
        Ok(Some(WikiTopicDetail {
            summary: wiki_topic_to_summary(topic),
            article_md: page.as_ref().map(|p| p.content_en.clone()),
            article_md_ko: page.as_ref().map(|p| p.content_ko.clone()),
        }))
    }

    /// Phase 8 trending readers. `window` must be one of "1h", "24h",
    /// "7d"; unknown labels return InvalidArgument so the UI can never
    /// silently render against the wrong window.
    pub fn wiki_trending_v2(&self, window: String) -> Result<Vec<WikiTrendingRow>, SeoyuError> {
        let w = crate::store::wiki_page::TrendingWindow::from_label(&window)
            .ok_or_else(|| SeoyuError::InvalidArgument(format!("unknown window: {window}")))?;
        let store = self.lock_store();
        let rows = store.list_trending_cache(w)?;
        Ok(rows
            .into_iter()
            .map(|r| WikiTrendingRow {
                page_id: r.page_id,
                rank: r.rank,
                kind: r.kind,
                title: r.title,
                hook: r.hook,
                reason_code: r.reason_code,
                reason_metrics: r.reason_metrics,
                sparkline: r.sparkline,
                computed_at: r.computed_at,
            })
            .collect())
    }

    /// Pinned active pages with ≥1 evidence in the window. Spec §6.4
    /// surfaces these in a separate slot above the ranked list.
    pub fn wiki_trending_pinned(
        &self,
        window: String,
    ) -> Result<Vec<WikiPinnedTrendingRow>, SeoyuError> {
        let w = crate::store::wiki_page::TrendingWindow::from_label(&window)
            .ok_or_else(|| SeoyuError::InvalidArgument(format!("unknown window: {window}")))?;
        let store = self.lock_store();
        let now = crate::wiki::norm::unix_now();
        let rows = store.list_trending_pinned(w, now)?;
        Ok(rows
            .into_iter()
            .map(|r| WikiPinnedTrendingRow {
                page_id: r.page_id,
                kind: r.kind,
                title: r.title,
                ec: r.ec,
                last_ts: r.last_ts,
                sparkline: r.sparkline,
            })
            .collect())
    }

    /// Phase 9 digest (spec §6.5). Per-chat list of pages with ≥3 new
    /// evidence rows since `wiki_last_open[chat_id]`. Hidden + resolved
    /// pages are filtered out. Sorted by chat then by count then by
    /// recency. Pure SQL, no LLM call.
    pub fn wiki_digest_rows(&self, limit: u32) -> Result<Vec<WikiDigestRow>, SeoyuError> {
        let store = self.lock_store();
        let limit = if limit == 0 { 200 } else { limit as i64 };
        let rows = store.list_digest_rows(limit)?;
        Ok(rows
            .into_iter()
            .map(|r| WikiDigestRow {
                chat_id: r.chat_id,
                page_id: r.page_id,
                kind: r.kind,
                state: r.state,
                title: r.title,
                n: r.n,
                last_ts: r.last_ts,
            })
            .collect())
    }

    /// Advance the per-chat digest cursor to "now". Spec §6.5: called
    /// only on explicit "mark read" or chat-open, not on panel-open.
    /// The cursor timestamp is always the current wall clock — exposing
    /// a caller-supplied `at` would let Swift bury the cursor in the
    /// far future and silently suppress every future digest row for
    /// that chat. The store upsert is MAX-monotonic so even within the
    /// trusted timestamp space, a clock skew can't rewind.
    pub fn wiki_mark_chat_read(&self, chat_id: i64) -> Result<(), SeoyuError> {
        let store = self.lock_store();
        let now = crate::wiki::norm::unix_now();
        store.mark_chat_read(chat_id, now)?;
        Ok(())
    }

    /// Spec §6.6 ask. Inserts an `ask_history` row in `streaming` state,
    /// retrieves FTS top-5 pages + top-20 evidence, then either:
    ///   - thin (<3 distinct evidence rows) → emit fallback delta + raw
    ///     evidence as on_source, finalize as `done`; OR
    ///   - call codex on a worker thread, parse NDJSON, dispatch
    ///     callbacks per segment, finalize as `done` / `cancelled` /
    ///     `failed`.
    /// Returns the ask_id immediately; callbacks fire from the worker.
    pub fn wiki_ask(
        &self,
        query: String,
        handler: Arc<dyn AskStreamHandler>,
    ) -> Result<i64, SeoyuError> {
        let q = query.trim().to_string();
        if q.is_empty() {
            return Err(SeoyuError::InvalidArgument("empty query".into()));
        }
        let now = crate::wiki::norm::unix_now();
        let (ask_id, evidence_summaries, page_ctx, thin_below_three, model) = {
            let store = self.lock_store();
            let pages = store.ask_fts_pages(&q, 5)?;
            let evidence = store.ask_fts_evidence(&q, 20, now)?;
            let summaries = build_evidence_summaries(&evidence);
            let setting = store.get_wiki_setting("model_ask").ok().flatten();
            let model = resolve_ask_model(setting.as_deref());
            let ask_id = store.ask_history_insert(&q, &model, now)?;
            let thin_below_three = summaries.len() < 3;
            (ask_id, summaries, pages, thin_below_three, model)
        };

        let state = Arc::new(AskRunState::default());
        {
            let mut map = self.active_asks.lock().unwrap_or_else(|e| e.into_inner());
            map.insert(ask_id, Arc::clone(&state));
        }

        let store = Arc::clone(&self.store);
        let active = Arc::clone(&self.active_asks);
        let handler_for_thread: Arc<dyn AskStreamHandler> = Arc::clone(&handler);
        let q_for_thread = q.clone();

        std::thread::Builder::new()
            .name("seoyu-wiki-ask".into())
            .spawn(move || {
                run_ask_job(
                    store,
                    active,
                    handler_for_thread,
                    state,
                    ask_id,
                    q_for_thread,
                    model,
                    page_ctx,
                    evidence_summaries,
                    thin_below_three,
                );
            })
            .map_err(|e| {
                // Spawn failed: mark history as failed and pop the slot.
                let store = self.lock_store();
                let _ = store.ask_history_finalize(
                    ask_id,
                    "failed",
                    "",
                    "[]",
                    crate::wiki::norm::unix_now(),
                );
                let mut map = self.active_asks.lock().unwrap_or_else(|e| e.into_inner());
                map.remove(&ask_id);
                SeoyuError::Other(format!("spawn ask thread: {e}"))
            })?;

        Ok(ask_id)
    }

    /// Spec §6.6 cancel. Flips the cancellation flag and sends SIGTERM
    /// to the codex pid if one is registered. The worker thread sees
    /// the flag, kills its child, drives the handler's `on_cancelled`,
    /// and writes `ask_history.status='cancelled'`. Calling cancel on
    /// a finished/unknown id is a no-op (returns Ok).
    pub fn wiki_cancel_ask(&self, ask_id: i64) -> Result<(), SeoyuError> {
        let state = {
            let map = self.active_asks.lock().unwrap_or_else(|e| e.into_inner());
            map.get(&ask_id).cloned()
        };
        if let Some(s) = state {
            s.cancelled.store(true, Ordering::Release);
            let pid = s.pid.load(Ordering::Acquire);
            if pid > 0 {
                // SAFETY: kill(2) on POSIX. Pid may have been reaped
                // between load and call; ESRCH is harmless.
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGTERM);
                }
            }
        }
        Ok(())
    }

    /// Search the wiki FTS5 index (Korean + English article text).
    pub fn wiki_search(
        &self,
        query: String,
        limit: u32,
    ) -> Result<Vec<WikiTopicSummary>, SeoyuError> {
        let store = self.lock_store();
        let hits = store.search_wiki_pages(&query, limit as usize)?;
        let mut out = Vec::with_capacity(hits.len());
        for hit in hits {
            if let Some(topic) = store.get_topic(hit.topic_id)? {
                out.push(wiki_topic_to_summary(topic));
            }
        }
        Ok(out)
    }

    pub fn start_wiki_worker(&self) -> Result<(), SeoyuError> {
        let mut guard = self.wiki_worker.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            return Ok(());
        }
        let emitter = Arc::new(crate::wiki::worker::ForeignEmitter::new(Arc::clone(
            &self.wiki_observer,
        )));
        let handle = crate::wiki::worker::start_worker(
            Arc::clone(&self.store),
            emitter,
            Arc::clone(&self.wiki_wake),
        )
        .map_err(|e| SeoyuError::Other(format!("spawn wiki worker: {e}")))?;
        *guard = Some(handle);
        Ok(())
    }

    pub fn set_wiki_observer(&self, observer: Option<Arc<dyn WikiObserver>>) {
        let mut slot = self.wiki_observer.lock().unwrap_or_else(|e| e.into_inner());
        *slot = observer;
    }

    pub fn wiki_run_pending_now(&self) {
        self.wiki_wake.store(true, Ordering::Relaxed);
    }

    pub fn stop_wiki_worker(&self) {
        let handle = {
            let mut guard = self.wiki_worker.lock().unwrap_or_else(|e| e.into_inner());
            guard.take()
        };
        if let Some(h) = handle {
            h.stop();
            h.join();
        }
    }

    pub fn wiki_digest_today(&self) -> Result<WikiDigest, SeoyuError> {
        let store = self.lock_store();
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (day_start, ymd) = local_day_start(now_secs);
        let (topic_count, message_count) = store.wiki_counts_since(day_start)?;
        let hot_topics = store
            .get_trending_topics(3, 0, None)?
            .into_iter()
            .map(wiki_topic_to_summary)
            .collect();
        Ok(WikiDigest {
            date_ymd: ymd,
            topic_count,
            message_count,
            hot_topics,
        })
    }

    pub fn wiki_topic_messages(
        &self,
        topic_id: i64,
        limit: u32,
    ) -> Result<Vec<SearchHit>, SeoyuError> {
        let store = self.lock_store();
        let rows = store.get_topic_messages(topic_id, limit as usize)?;
        Ok(rows.into_iter().map(topic_row_to_hit).collect())
    }

    pub fn wiki_categories(&self) -> Result<Vec<WikiCategory>, SeoyuError> {
        let store = self.lock_store();
        let cats = store.get_categories_with_counts()?;
        Ok(cats
            .into_iter()
            .map(|c| WikiCategory {
                id: c.id,
                name: c.name,
                name_ko: c.name_ko,
                topic_count: c.topic_count,
            })
            .collect())
    }
}

impl Seoyu {
    fn lock_store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// ---------- internal helpers (not exposed via UniFFI) ----------

fn to_search_page(result: CoreSearchResult) -> SearchPage {
    let items = result
        .items
        .into_iter()
        .map(|item| {
            let (starts, ends): (Vec<u32>, Vec<u32>) = item
                .highlights
                .iter()
                .map(|h| (h.start as u32, h.end as u32))
                .unzip();
            SearchHit {
                chat_id: item.chat_id,
                message_id: item.message_id,
                timestamp: item.timestamp,
                text: item.text,
                link: item.link,
                chat_title: item.chat_title,
                highlight_starts: starts,
                highlight_ends: ends,
            }
        })
        .collect();
    SearchPage {
        items,
        next_cursor: result.next_cursor.map(|c| SearchCursor {
            rank: c.rank,
            timestamp: c.timestamp,
            chat_id: c.chat_id,
            message_id: c.message_id,
        }),
    }
}

fn to_index_outcome(outcome: CoreIndexOutcome) -> IndexOutcome {
    IndexOutcome {
        inserted: outcome.inserted,
        updated: outcome.updated,
    }
}

fn wiki_topic_to_summary(t: crate::store::wiki_topic::WikiTopic) -> WikiTopicSummary {
    WikiTopicSummary {
        id: t.topic_id,
        title: t.title,
        title_ko: t.title_ko,
        category: t.category_name.unwrap_or_else(|| "Uncategorized".into()),
        message_count: t.message_count,
        trending_score: t.trending_score,
    }
}

impl Drop for Seoyu {
    fn drop(&mut self) {
        self.set_wiki_observer(None);
        self.stop_wiki_worker();
        // Cancel any in-flight asks. The worker thread holds Arcs to
        // store + handler so it survives `Seoyu` dropping; without this
        // it would run to completion firing callbacks into a Swift
        // context the user has already abandoned. SIGTERM short-circuits
        // the codex subprocess; the thread sees `cancelled` next poll
        // and writes `ask_history.status='cancelled'`.
        let map = self.active_asks.lock().unwrap_or_else(|e| e.into_inner());
        for state in map.values() {
            state.cancelled.store(true, Ordering::Release);
            let pid = state.pid.load(Ordering::Acquire);
            if pid > 0 {
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGTERM);
                }
            }
        }
    }
}

fn local_day_start(now_secs: i64) -> (i64, String) {
    use std::mem::MaybeUninit;
    unsafe {
        let t: libc::time_t = now_secs as libc::time_t;
        let mut local: MaybeUninit<libc::tm> = MaybeUninit::uninit();
        if libc::localtime_r(&t, local.as_mut_ptr()).is_null() {
            return (now_secs - now_secs.rem_euclid(86_400), "1970-01-01".into());
        }
        let local = local.assume_init();
        let ymd = format!(
            "{:04}-{:02}-{:02}",
            local.tm_year + 1900,
            local.tm_mon + 1,
            local.tm_mday,
        );
        let day_start = now_secs
            - (local.tm_hour as i64) * 3600
            - (local.tm_min as i64) * 60
            - (local.tm_sec as i64);
        (day_start, ymd)
    }
}

/// Build presentation-indexed evidence summaries (1..=N) from the
/// retrieved evidence rows. The LLM never sees evidence_id — it only
/// sees `source_id`, which lets the host strip cites pointing to ids
/// the LLM hallucinated.
fn build_evidence_summaries(rows: &[AskEvidence]) -> Vec<EvidenceSummary> {
    rows.iter()
        .enumerate()
        .map(|(i, e)| EvidenceSummary {
            source_id: (i + 1) as u32,
            evidence_id: e.evidence_id,
            page_id: e.page_id,
            page_title: e.page_title.clone(),
            chat_id: e.chat_id,
            chat_title: e.chat_title.clone(),
            msg_id: e.msg_id,
            sender_id: e.sender_id,
            ts: e.ts,
            excerpt: e.excerpt.clone(),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn run_ask_job(
    store: Arc<Mutex<Store>>,
    active: Arc<Mutex<HashMap<i64, Arc<AskRunState>>>>,
    handler: Arc<dyn AskStreamHandler>,
    state: Arc<AskRunState>,
    ask_id: i64,
    query: String,
    model: String,
    page_ctx: Vec<AskPage>,
    evidence_summaries: Vec<EvidenceSummary>,
    thin_below_three: bool,
) {
    // Wrap the handler in AskDispatch so every callback hops to the
    // macOS main queue (spec §7) before invoking the Swift impl. The
    // worker keeps the strong Arc for the duration; cancel is the
    // explicit `wiki_cancel_ask(id)` API (no implicit-drop signal at
    // this UniFFI binding shape).
    let dispatch = AskDispatch::from_arc(handler);
    let finalize_status = ask_run_inner(
        &store,
        &dispatch,
        &state,
        ask_id,
        &query,
        &model,
        &page_ctx,
        &evidence_summaries,
        thin_below_three,
    );
    // Drop the active-asks slot regardless of outcome.
    {
        let mut map = active.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(&ask_id);
    }
    // Always emit one terminal callback through the main-queue hop.
    match finalize_status {
        AskOutcome::Done => dispatch.on_finished(ask_id),
        AskOutcome::Cancelled => dispatch.on_cancelled(ask_id),
        AskOutcome::Failed(msg) => dispatch.on_error(ask_id, msg),
    }
}

enum AskOutcome {
    Done,
    Cancelled,
    Failed(String),
}

/// Parse one codex agent_message text as NDJSON and dispatch each
/// segment + cite through the handler. Returns Err(msg) on parse
/// failure so the caller can record it. Extracted so tests can
/// exercise the LLM dispatch path without invoking codex (codex
/// review). The state argument stays in the signature because the
/// test handler may want to inspect it; current implementation
/// reads no fields.
#[allow(clippy::too_many_arguments)]
fn dispatch_agent_message(
    text: &str,
    handler: &AskDispatch,
    _state: &AskRunState,
    evidence_summaries: &[EvidenceSummary],
    evidence_count: u32,
    answer_md: &mut String,
    cited_ids: &mut std::collections::BTreeSet<u32>,
    segments_dispatched: &mut u32,
    model_thin_evidence: &mut bool,
) -> Result<(), String> {
    let parsed = parse_ask_stream(text).map_err(|e| format!("malformed stream: {e}"))?;
    *model_thin_evidence = *model_thin_evidence || parsed.thin_evidence;
    for seg in parsed.segments {
        let cleaned = strip_citation_markers(&seg.md);
        // Spec §6.6 "before any character is shown": validate cites
        // first, then dispatch on_delta then on_source.
        let valid_cites = validate_cites(&seg.cites, evidence_count);
        handler.on_delta(seg.seg, cleaned.clone());
        for tag in &valid_cites {
            if let Some(ev) = evidence_summaries.get(*tag as usize - 1) {
                handler.on_source(seg.seg, *tag, ev.clone());
                cited_ids.insert(*tag);
            }
        }
        if !answer_md.is_empty() {
            answer_md.push_str("\n\n");
        }
        answer_md.push_str(&cleaned);
        *segments_dispatched += 1;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn ask_run_inner(
    store: &Arc<Mutex<Store>>,
    handler: &AskDispatch,
    state: &Arc<AskRunState>,
    ask_id: i64,
    query: &str,
    model: &str,
    page_ctx: &[AskPage],
    evidence_summaries: &[EvidenceSummary],
    thin_below_three: bool,
) -> AskOutcome {
    // Compute thin_evidence hint per spec §6.6: <2 distinct chats AND
    // <3 distinct senders → mark thin and let the model flag uncertainty.
    let mut chats = std::collections::HashSet::new();
    let mut senders = std::collections::HashSet::new();
    for e in evidence_summaries {
        chats.insert(e.chat_id);
        senders.insert(e.sender_id);
    }
    let thin_hint = chats.len() < 2 && senders.len() < 3;

    if thin_below_three {
        // Spec §6.6: <3 distinct evidence → skip LLM, surface fallback +
        // raw FTS results below. Emit one delta segment, attach every
        // available evidence row as a source, persist + finish.
        // Cancellation may have arrived between wiki_ask returning the
        // ask_id and this thread getting scheduled — check before any
        // callback fires (codex review).
        if state.cancelled.load(Ordering::Acquire) {
            finalize(store, ask_id, "cancelled", "", "[]");
            return AskOutcome::Cancelled;
        }
        let msg = "Not enough in your chats yet to answer confidently. Showing the best evidence I found.".to_string();
        handler.on_delta(0, msg.clone());
        // Track which sources actually fired, so a mid-loop cancel
        // persists ONLY what the user could have seen — not the full
        // pre-cancel evidence list (codex review). `shown` grows after
        // each successful on_source.
        let mut shown: Vec<EvidenceSummary> = Vec::with_capacity(evidence_summaries.len());
        for ev in evidence_summaries {
            // Re-check cancel between sources so a long evidence list
            // doesn't keep firing callbacks after cancel.
            if state.cancelled.load(Ordering::Acquire) {
                // Persist what's been shown so far per spec §6.6:
                // "Partial answers shown so far are persisted with the
                // cancelled status."
                let cited_json = cited_sources_json(&shown);
                finalize(store, ask_id, "cancelled", &msg, &cited_json);
                return AskOutcome::Cancelled;
            }
            handler.on_source(0, ev.source_id, ev.clone());
            shown.push(ev.clone());
        }
        // Spec §6.3 retention: bump cited counter so these evidence
        // rows survive the next rewrite-time pruning. Same as the LLM
        // path — the user surfaced these rows as the answer.
        let cited_evidence_ids: Vec<i64> =
            evidence_summaries.iter().map(|e| e.evidence_id).collect();
        if !cited_evidence_ids.is_empty() {
            let s = store.lock().unwrap_or_else(|e| e.into_inner());
            let _ = s.bump_cited(&cited_evidence_ids);
        }
        let cited_json = cited_sources_json(evidence_summaries);
        finalize(store, ask_id, "done", &msg, &cited_json);
        return AskOutcome::Done;
    }

    // Build LLM input. Page summaries intentionally NOT passed —
    // codex review: a page's summary_md is synthesized from its
    // evidence rows and can leak excluded/deleted-source content even
    // after the evidence-level filter. Pages stay in `page_ctx` for
    // potential UI use but never reach the LLM. Evidence rows already
    // carry `page_title`, which is enough topic context for the model.
    let _ = page_ctx;
    let excerpts: Vec<String> = evidence_summaries
        .iter()
        .map(|e| truncate_chars(&e.excerpt, 280).to_string())
        .collect();
    let evidence_in: Vec<AskEvidenceIn> = evidence_summaries
        .iter()
        .zip(excerpts.iter())
        .map(|(e, ex)| AskEvidenceIn {
            source_id: e.source_id,
            page_title: e.page_title.as_str(),
            chat_title: e.chat_title.as_str(),
            ts: e.ts,
            excerpt: ex.as_str(),
        })
        .collect();
    let input = AskInput {
        query,
        thin_evidence: thin_hint,
        evidence: &evidence_in,
    };

    let evidence_count = evidence_summaries.len() as u32;
    // State accumulated across agent_message events. We dispatch each
    // segment's on_delta + on_source as soon as the agent_message
    // arrives — codex emits item.completed before turn.completed, so
    // callbacks fire ahead of the codex subprocess closing the turn.
    let mut answer_md = String::new();
    let mut cited_ids: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    let mut segments_dispatched: u32 = 0;
    let mut model_thin_evidence = false;
    let mut parse_err: Option<String> = None;

    let res = LlmClient::new().run_ask_stream(&input, model, state, |text| {
        if parse_err.is_some() {
            return;
        }
        if let Err(msg) = dispatch_agent_message(
            text,
            handler,
            state,
            evidence_summaries,
            evidence_count,
            &mut answer_md,
            &mut cited_ids,
            &mut segments_dispatched,
            &mut model_thin_evidence,
        ) {
            parse_err = Some(msg);
        }
    });

    // Helper closure: snapshot whatever has been dispatched so far.
    // Used for cancel-with-partial-answer per spec §6.6.
    let snapshot_cited = |cited_ids: &std::collections::BTreeSet<u32>| -> Vec<EvidenceSummary> {
        cited_ids
            .iter()
            .filter_map(|id| evidence_summaries.get(*id as usize - 1).cloned())
            .collect()
    };

    match res {
        Ok(()) => {}
        Err(AskRunError::Cancelled) => {
            // Persist partial answer per spec §6.6: "Partial answers
            // shown so far are persisted with the cancelled status."
            let cited = snapshot_cited(&cited_ids);
            finalize(
                store,
                ask_id,
                "cancelled",
                &answer_md,
                &cited_sources_json(&cited),
            );
            return AskOutcome::Cancelled;
        }
        Err(AskRunError::Timeout(secs)) => {
            finalize(store, ask_id, "failed", "", "[]");
            return AskOutcome::Failed(format!("ask timed out after {secs}s"));
        }
        Err(AskRunError::Exec(e)) => {
            finalize(store, ask_id, "failed", "", "[]");
            return AskOutcome::Failed(format!("codex: {e}"));
        }
    }

    if let Some(msg) = parse_err {
        finalize(store, ask_id, "failed", "", "[]");
        return AskOutcome::Failed(msg);
    }
    if state.cancelled.load(Ordering::Acquire) {
        let cited = snapshot_cited(&cited_ids);
        finalize(
            store,
            ask_id,
            "cancelled",
            &answer_md,
            &cited_sources_json(&cited),
        );
        return AskOutcome::Cancelled;
    }

    if segments_dispatched == 0 {
        // Codex review: model emitting only `done` with no segments
        // must not become a successful blank answer. Two sub-cases:
        //   - thin_evidence=true → model legitimately said "I can't
        //     answer". Surface a fallback delta + finish as `done`,
        //     so the UI shows a message instead of dead silence.
        //   - thin_evidence=false → model claimed an answer but
        //     produced nothing → failure.
        if model_thin_evidence {
            let msg = "I couldn't find enough relevant context to answer confidently.".to_string();
            handler.on_delta(0, msg.clone());
            finalize(store, ask_id, "done", &msg, "[]");
            return AskOutcome::Done;
        }
        finalize(store, ask_id, "failed", "", "[]");
        return AskOutcome::Failed("model returned no answer segments".into());
    }

    let cited = snapshot_cited(&cited_ids);
    // Spec §6.3 retention: bump the per-row `cited` counter so the
    // next rewrite tick keeps these evidence rows even if they fall
    // outside the salience-based pruning window.
    let cited_evidence_ids: Vec<i64> = cited.iter().map(|e| e.evidence_id).collect();
    if !cited_evidence_ids.is_empty() {
        let s = store.lock().unwrap_or_else(|e| e.into_inner());
        let _ = s.bump_cited(&cited_evidence_ids);
    }
    finalize(
        store,
        ask_id,
        "done",
        &answer_md,
        &cited_sources_json(&cited),
    );
    AskOutcome::Done
}

fn finalize(
    store: &Arc<Mutex<Store>>,
    ask_id: i64,
    status: &str,
    answer_md: &str,
    cited_json: &str,
) {
    let now = crate::wiki::norm::unix_now();
    let s = store.lock().unwrap_or_else(|e| e.into_inner());
    let _ = s.ask_history_finalize(ask_id, status, answer_md, cited_json, now);
}

/// Persist the actual evidence rows the answer cited (spec line 964) —
/// not `[n]` labels. Schema is plain JSON list of source records.
fn cited_sources_json(items: &[EvidenceSummary]) -> String {
    let arr: Vec<serde_json::Value> = items
        .iter()
        .map(|e| {
            serde_json::json!({
                "source_id": e.source_id,
                "evidence_id": e.evidence_id,
                "page_id": e.page_id,
                "page_title": e.page_title,
                "chat_id": e.chat_id,
                "chat_title": e.chat_title,
                "msg_id": e.msg_id,
                "sender_id": e.sender_id,
                "ts": e.ts,
                "excerpt": e.excerpt,
            })
        })
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
}

fn truncate_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &s[..byte_idx],
        None => s,
    }
}

fn topic_row_to_hit(row: crate::store::wiki_topic::TopicMessageRow) -> SearchHit {
    SearchHit {
        chat_id: row.chat_id,
        message_id: row.message_id,
        timestamp: row.timestamp,
        text: row.text,
        link: row.link,
        chat_title: row.chat_title,
        highlight_starts: Vec::new(),
        highlight_ends: Vec::new(),
    }
}

#[cfg(test)]
mod ask_tests {
    use super::*;

    /// Records every callback in delivery order so tests can assert
    /// ordering + final terminal callback.
    #[derive(Default)]
    struct RecordingHandler {
        events: Mutex<Vec<String>>,
    }
    impl AskStreamHandler for RecordingHandler {
        fn on_delta(&self, seg: u32, text: String) {
            self.events
                .lock()
                .unwrap()
                .push(format!("delta {seg} {text}"));
        }
        fn on_source(&self, seg: u32, tag: u32, source: EvidenceSummary) {
            self.events
                .lock()
                .unwrap()
                .push(format!("source {seg} {tag} ev{}", source.evidence_id));
        }
        fn on_finished(&self, ask_id: i64) {
            self.events
                .lock()
                .unwrap()
                .push(format!("finished {ask_id}"));
        }
        fn on_cancelled(&self, ask_id: i64) {
            self.events
                .lock()
                .unwrap()
                .push(format!("cancelled {ask_id}"));
        }
        fn on_error(&self, ask_id: i64, message: String) {
            self.events
                .lock()
                .unwrap()
                .push(format!("error {ask_id} {message}"));
        }
    }

    fn ev(source_id: u32, evidence_id: i64, chat_id: i64, sender_id: i64) -> EvidenceSummary {
        EvidenceSummary {
            source_id,
            evidence_id,
            page_id: 1,
            page_title: "P".into(),
            chat_id,
            chat_title: "C".into(),
            msg_id: evidence_id,
            sender_id,
            ts: 1_000,
            excerpt: "snippet".into(),
        }
    }

    #[test]
    fn thin_path_emits_fallback_delta_and_sources_then_finalizes_done() {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let recorder = Arc::new(RecordingHandler::default());
        let handler: Arc<dyn AskStreamHandler> = Arc::clone(&recorder) as Arc<dyn AskStreamHandler>;
        let state = Arc::new(AskRunState::default());
        let active = Arc::new(Mutex::new(HashMap::new()));
        let evidence = vec![ev(1, 11, 1, 7), ev(2, 12, 1, 8)];

        // Seed an ask_history row so finalize has a target.
        let ask_id = {
            let s = store.lock().unwrap();
            s.ask_history_insert("q?", "test-model", 0).unwrap()
        };
        active.lock().unwrap().insert(ask_id, Arc::clone(&state));

        // Drive the job. thin_below_three=true → no LLM call needed,
        // safe to run synchronously in the test thread.
        run_ask_job(
            Arc::clone(&store),
            Arc::clone(&active),
            handler,
            state,
            ask_id,
            "q?".into(),
            "test-model".into(),
            Vec::new(),
            evidence,
            true,
        );
        let _ = recorder; // recorder kept alive via Arc; events asserted below.

        // Active map cleared.
        assert!(active.lock().unwrap().is_empty());

        // ask_history row finalized to 'done' with non-empty cited list.
        let s = store.lock().unwrap();
        let mut q = s
            .conn()
            .prepare("SELECT status, answer_md, cited_sources FROM ask_history WHERE id = ?")
            .unwrap();
        q.bind((1, ask_id)).unwrap();
        q.next().unwrap();
        assert_eq!(q.read::<String, _>(0).unwrap(), "done");
        assert!(q
            .read::<String, _>(1)
            .unwrap()
            .contains("Not enough in your chats yet"));
        let cited = q.read::<String, _>(2).unwrap();
        // Both evidence rows persisted as cited (thin path surfaces the
        // raw FTS rows below the fallback message).
        assert!(cited.contains("\"evidence_id\":11"));
        assert!(cited.contains("\"evidence_id\":12"));
    }

    #[test]
    fn thin_path_callback_order_delta_before_sources_then_finished() {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let recorder = Arc::new(RecordingHandler::default());
        let handler: Arc<dyn AskStreamHandler> = Arc::clone(&recorder) as Arc<dyn AskStreamHandler>;
        let state = Arc::new(AskRunState::default());
        let active = Arc::new(Mutex::new(HashMap::new()));
        let evidence = vec![ev(1, 11, 1, 7), ev(2, 12, 1, 8)];

        let ask_id = {
            let s = store.lock().unwrap();
            s.ask_history_insert("q?", "test-model", 0).unwrap()
        };
        active.lock().unwrap().insert(ask_id, Arc::clone(&state));

        run_ask_job(
            store,
            active,
            handler,
            state,
            ask_id,
            "q?".into(),
            "test-model".into(),
            Vec::new(),
            evidence,
            true,
        );

        let events = recorder.events.lock().unwrap().clone();
        assert_eq!(
            events.len(),
            4,
            "delta + 2 sources + finished, got {events:?}"
        );
        assert!(events[0].starts_with("delta 0 "));
        assert!(events[1].starts_with("source 0 1 "));
        assert!(events[2].starts_with("source 0 2 "));
        assert!(events[3].starts_with("finished "));
    }

    #[test]
    fn thin_path_respects_cancel_before_dispatch() {
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let recorder = Arc::new(RecordingHandler::default());
        let handler: Arc<dyn AskStreamHandler> = Arc::clone(&recorder) as Arc<dyn AskStreamHandler>;
        let state = Arc::new(AskRunState::default());
        // Cancel BEFORE the worker starts. Codex review: thin path
        // must check cancel before any callback fires.
        state.cancelled.store(true, Ordering::Release);
        let active = Arc::new(Mutex::new(HashMap::new()));
        let evidence = vec![ev(1, 11, 1, 7), ev(2, 12, 1, 8)];

        let ask_id = {
            let s = store.lock().unwrap();
            s.ask_history_insert("q?", "test-model", 0).unwrap()
        };
        active.lock().unwrap().insert(ask_id, Arc::clone(&state));

        run_ask_job(
            store.clone(),
            active,
            handler,
            state,
            ask_id,
            "q?".into(),
            "test-model".into(),
            Vec::new(),
            evidence,
            true,
        );

        let events = recorder.events.lock().unwrap().clone();
        // No delta, no source — only the terminal on_cancelled.
        assert_eq!(events.len(), 1, "cancel-only, got {events:?}");
        assert!(events[0].starts_with("cancelled "));
        // ask_history persisted as `cancelled` (not `done`).
        let s = store.lock().unwrap();
        let mut q = s
            .conn()
            .prepare("SELECT status FROM ask_history WHERE id = ?")
            .unwrap();
        q.bind((1, ask_id)).unwrap();
        q.next().unwrap();
        assert_eq!(q.read::<String, _>(0).unwrap(), "cancelled");
    }

    // ---- dispatch_agent_message: tests the LLM dispatch path
    // without a live codex subprocess (codex review).

    fn dispatch_setup() -> (
        Arc<RecordingHandler>,
        AskDispatch,
        Arc<AskRunState>,
        Vec<EvidenceSummary>,
    ) {
        let recorder = Arc::new(RecordingHandler::default());
        let handler_arc: Arc<dyn AskStreamHandler> =
            Arc::clone(&recorder) as Arc<dyn AskStreamHandler>;
        let dispatch = AskDispatch::from_arc(handler_arc);
        let state = Arc::new(AskRunState::default());
        let evidence = vec![ev(1, 11, 1, 7), ev(2, 12, 2, 8), ev(3, 13, 3, 9)];
        (recorder, dispatch, state, evidence)
    }

    #[test]
    fn dispatch_agent_message_dispatches_segments_in_order() {
        let (recorder, dispatch, state, evidence) = dispatch_setup();
        let mut answer = String::new();
        let mut cited = std::collections::BTreeSet::new();
        let mut count = 0u32;
        let mut thin = false;
        let text = r#"{"type":"segment","seg":0,"md":"first paragraph","cites":[1]}
{"type":"segment","seg":1,"md":"second paragraph","cites":[2,3]}
{"type":"done","thin_evidence":false}"#;
        let r = dispatch_agent_message(
            text,
            &dispatch,
            &state,
            &evidence,
            evidence.len() as u32,
            &mut answer,
            &mut cited,
            &mut count,
            &mut thin,
        );
        assert!(r.is_ok());
        assert_eq!(count, 2);
        assert_eq!(answer, "first paragraph\n\nsecond paragraph");
        assert_eq!(cited.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3]);
        let events = recorder.events.lock().unwrap().clone();
        // delta 0, source 0/1, delta 1, source 1/2, source 1/3.
        assert_eq!(events.len(), 5);
        assert!(events[0].starts_with("delta 0 first paragraph"));
        assert!(events[1].starts_with("source 0 1 "));
        assert!(events[2].starts_with("delta 1 second paragraph"));
        assert!(events[3].starts_with("source 1 2 "));
        assert!(events[4].starts_with("source 1 3 "));
    }

    #[test]
    fn dispatch_agent_message_strips_unknown_cites() {
        let (recorder, dispatch, state, evidence) = dispatch_setup();
        let mut answer = String::new();
        let mut cited = std::collections::BTreeSet::new();
        let mut count = 0u32;
        let mut thin = false;
        // cite 99 doesn't exist — must not fire on_source.
        let text = r#"{"type":"segment","seg":0,"md":"a","cites":[1,99,2]}
{"type":"done","thin_evidence":false}"#;
        let r = dispatch_agent_message(
            text,
            &dispatch,
            &state,
            &evidence,
            evidence.len() as u32,
            &mut answer,
            &mut cited,
            &mut count,
            &mut thin,
        );
        assert!(r.is_ok());
        let events = recorder.events.lock().unwrap().clone();
        assert!(events.iter().all(|e| !e.contains(" 99 ")));
        assert_eq!(cited.iter().copied().collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn dispatch_agent_message_strips_inline_citation_markers() {
        let (recorder, dispatch, state, evidence) = dispatch_setup();
        let mut answer = String::new();
        let mut cited = std::collections::BTreeSet::new();
        let mut count = 0u32;
        let mut thin = false;
        // Model emitted `[1]` inline despite the prompt prohibiting it.
        let text = r#"{"type":"segment","seg":0,"md":"BTC up [1] today","cites":[1]}
{"type":"done","thin_evidence":false}"#;
        let r = dispatch_agent_message(
            text,
            &dispatch,
            &state,
            &evidence,
            evidence.len() as u32,
            &mut answer,
            &mut cited,
            &mut count,
            &mut thin,
        );
        assert!(r.is_ok());
        // delta text must not contain `[1]`.
        let events = recorder.events.lock().unwrap().clone();
        assert!(events[0].starts_with("delta 0 BTC up  today"));
    }

    #[test]
    fn dispatch_agent_message_returns_parse_error() {
        let (_recorder, dispatch, state, evidence) = dispatch_setup();
        let mut answer = String::new();
        let mut cited = std::collections::BTreeSet::new();
        let mut count = 0u32;
        let mut thin = false;
        let r = dispatch_agent_message(
            "not json",
            &dispatch,
            &state,
            &evidence,
            evidence.len() as u32,
            &mut answer,
            &mut cited,
            &mut count,
            &mut thin,
        );
        assert!(r.is_err());
        assert_eq!(count, 0);
    }

    #[test]
    fn dispatch_agent_message_thin_evidence_done_only() {
        let (recorder, dispatch, state, evidence) = dispatch_setup();
        let mut answer = String::new();
        let mut cited = std::collections::BTreeSet::new();
        let mut count = 0u32;
        let mut thin = false;
        let text = r#"{"type":"done","thin_evidence":true}"#;
        let r = dispatch_agent_message(
            text,
            &dispatch,
            &state,
            &evidence,
            evidence.len() as u32,
            &mut answer,
            &mut cited,
            &mut count,
            &mut thin,
        );
        assert!(r.is_ok());
        assert_eq!(count, 0);
        assert!(thin);
        assert_eq!(recorder.events.lock().unwrap().len(), 0);
    }
}
