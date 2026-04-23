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

use std::sync::{Arc, Mutex};

use crate::search::{engine, SearchResult as CoreSearchResult};
use crate::store::chat::ChatRow;
use crate::store::message::{strip_whitespace, Cursor, MessageRow};
use crate::store::Store;
use crate::wiki::worker::{start_worker, LogEmitter, WorkerHandle};

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

// ---------- The Swift-facing `Seoyu` object ----------

/// Root handle the Swift shell keeps for the lifetime of the app.
/// One instance per database file is expected; UniFFI will drop it
/// when Swift releases its last reference.
#[derive(uniffi::Object)]
pub struct Seoyu {
    store: Arc<Mutex<Store>>,
    wiki_worker: Mutex<Option<WorkerHandle>>,
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

    /// Mirror a batch of messages into the local store, populating
    /// every FTS5 auxiliary index (plain, nospace, jamo, chosung).
    /// Returns the number of inputs accepted (duplicates are
    /// silently skipped by `INSERT OR IGNORE`).
    pub fn index_messages(&self, messages: Vec<IndexedMessage>) -> Result<u64, SeoyuError> {
        if messages.is_empty() {
            return Ok(0);
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
            })
            .collect();
        let count = rows.len() as u64;
        let store = self.lock_store();
        store.insert_messages_batch(&rows)?;
        Ok(count)
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
        let handle = start_worker(Arc::clone(&self.store), Arc::new(LogEmitter))
            .map_err(|e| SeoyuError::Other(format!("spawn wiki worker: {e}")))?;
        *guard = Some(handle);
        Ok(())
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
            timestamp: c.timestamp,
            chat_id: c.chat_id,
            message_id: c.message_id,
        }),
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
        self.stop_wiki_worker();
    }
}
