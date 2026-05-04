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
    AskInput, AskPageIn, AskRunError, AskRunState, LlmClient,
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
/// serialized on the worker thread; the Swift side wraps body code in
/// `DispatchQueue.main.async` per spec §7 threading contract.
///
/// Per-segment delivery order: `on_delta` first, then 0+ `on_source`
/// for that segment's validated cites. Segments with no valid cites
/// still emit `on_delta`. Terminal callback is exactly one of
/// `on_finished` / `on_cancelled` / `on_error`.
#[uniffi::export(with_foreign)]
pub trait AskStreamHandler: Send + Sync {
    fn on_delta(&self, segment_index: u32, text: String);
    fn on_source(&self, segment_index: u32, tag: u32, source: EvidenceSummary);
    fn on_finished(&self, ask_id: i64);
    fn on_cancelled(&self, ask_id: i64);
    fn on_error(&self, ask_id: i64, message: String);
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
    let finalize_status = ask_run_inner(
        &store,
        &handler,
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
    // Always emit one terminal callback. A panic above would skip this
    // → callers see no terminal event. The inner fn is panic-safe by
    // construction (no unwrap on user-supplied data); if that assumption
    // breaks, the missing callback is the visible symptom and worth
    // surfacing.
    match finalize_status {
        AskOutcome::Done => handler.on_finished(ask_id),
        AskOutcome::Cancelled => handler.on_cancelled(ask_id),
        AskOutcome::Failed(msg) => handler.on_error(ask_id, msg),
    }
}

enum AskOutcome {
    Done,
    Cancelled,
    Failed(String),
}

#[allow(clippy::too_many_arguments)]
fn ask_run_inner(
    store: &Arc<Mutex<Store>>,
    handler: &Arc<dyn AskStreamHandler>,
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
        let msg = "Not enough in your chats yet to answer confidently. Showing the best evidence I found.".to_string();
        handler.on_delta(0, msg.clone());
        for ev in evidence_summaries {
            handler.on_source(0, ev.source_id, ev.clone());
        }
        let cited_json = cited_sources_json(evidence_summaries);
        finalize(store, ask_id, "done", &msg, &cited_json);
        return AskOutcome::Done;
    }

    // Build LLM input. Truncate excerpts so a single oversize row
    // can't blow the prompt budget.
    let pages_in: Vec<AskPageIn> = page_ctx
        .iter()
        .map(|p| AskPageIn {
            kind: p.kind.as_str(),
            title: p.title.as_str(),
            summary_md: truncate_chars(&p.summary_md, 600),
        })
        .collect();
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
        pages: &pages_in,
        evidence: &evidence_in,
    };

    let raw = match LlmClient::new().run_ask(&input, model, state) {
        Ok(s) => s,
        Err(AskRunError::Cancelled) => {
            finalize(store, ask_id, "cancelled", "", "[]");
            return AskOutcome::Cancelled;
        }
        Err(AskRunError::Timeout(secs)) => {
            let msg = format!("ask timed out after {secs}s");
            finalize(store, ask_id, "failed", "", "[]");
            return AskOutcome::Failed(msg);
        }
        Err(AskRunError::Exec(e)) => {
            finalize(store, ask_id, "failed", "", "[]");
            return AskOutcome::Failed(format!("codex: {e}"));
        }
    };

    if state.cancelled.load(Ordering::Acquire) {
        finalize(store, ask_id, "cancelled", "", "[]");
        return AskOutcome::Cancelled;
    }

    let parsed = match parse_ask_stream(&raw) {
        Ok(p) => p,
        Err(e) => {
            finalize(store, ask_id, "failed", "", "[]");
            return AskOutcome::Failed(format!("malformed stream: {e}"));
        }
    };

    let evidence_count = evidence_summaries.len() as u32;
    let mut answer_md = String::new();
    let mut cited_ids: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();

    for seg in &parsed.segments {
        let cleaned = strip_citation_markers(&seg.md);
        // Per spec wording "before any character is shown": validate cites
        // first, then dispatch on_delta and on_source in order.
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
    }

    let cited: Vec<EvidenceSummary> = cited_ids
        .iter()
        .filter_map(|id| evidence_summaries.get(*id as usize - 1).cloned())
        .collect();
    finalize(
        store,
        ask_id,
        "done",
        &answer_md,
        &cited_sources_json(&cited),
    );
    let _ = parsed.thin_evidence; // surfaced via the on_delta text already.
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
}
