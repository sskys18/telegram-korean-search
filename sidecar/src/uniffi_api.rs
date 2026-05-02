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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::search::{engine, SearchResult as CoreSearchResult};
use crate::store::chat::ChatRow;
use crate::store::message::{
    strip_whitespace, Cursor, IndexOutcome as CoreIndexOutcome, MessageRef as CoreMessageRef,
    MessageRow,
};
use crate::store::Store;
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
