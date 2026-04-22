//! Method dispatch for the IPC layer.
//!
//! Each client request is translated here into a call on the owning
//! [`SidecarState`] and packaged back into a
//! [`ResponsePayload`]. Handlers are intentionally thin — they only
//! do shape conversion and lock handling. Real logic lives in
//! `store`, `search`, and `wiki`.

use std::sync::Arc;
use std::sync::Mutex;

use crate::ipc::protocol::{
    IndexBatchParams, IndexBatchResult, IndexMessageInput, Method, Notification, NotifyIn, Outcome,
    PongResult, Request, Response, ResponsePayload, RpcError, SearchParams, SearchScopeInput,
    WikiSearchParams, WikiTopicDetail, WikiTopicDetailParams, WikiTopicSummary, WikiTrendingParams,
};
use crate::search::{engine, SearchResult};
use crate::store::message::{strip_whitespace, MessageRow};
use crate::store::wiki_topic::WikiTopic;
use crate::store::Store;

/// Shared state for every handler. Cheap to clone.
#[derive(Clone)]
pub struct SidecarState {
    pub store: Arc<Mutex<Store>>,
}

impl SidecarState {
    pub fn new(store: Store) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }

    fn lock_store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Outcome of dispatching one message. `Shutdown` tells the server
/// loop to drop the connection and stop accepting new work.
pub enum Dispatch {
    Reply(Response),
    Silent,
    Shutdown,
}

pub fn dispatch_request(state: &SidecarState, req: Request) -> Dispatch {
    let id = req.id;
    let outcome = match req.call {
        Method::Ping => Outcome::Ok {
            result: ResponsePayload::Pong(PongResult {
                version: env!("CARGO_PKG_VERSION"),
            }),
        },
        Method::Shutdown => {
            return Dispatch::Shutdown;
        }
        Method::IndexMessagesBatch(params) => match index_messages_batch(state, params) {
            Ok(count) => Outcome::Ok {
                result: ResponsePayload::IndexBatch(IndexBatchResult { indexed: count }),
            },
            Err(e) => Outcome::Err {
                error: RpcError::internal(e.to_string()),
            },
        },
        Method::DeleteMessage(_params) => Outcome::Err {
            error: RpcError::internal("delete_message not yet implemented"),
        },
        Method::Search(params) => match run_search(state, params) {
            Ok(result) => Outcome::Ok {
                result: ResponsePayload::Search(result),
            },
            Err(e) => Outcome::Err {
                error: RpcError::internal(e.to_string()),
            },
        },
        Method::WikiTrending(params) => match wiki_trending(state, params) {
            Ok(list) => Outcome::Ok {
                result: ResponsePayload::WikiTrending(list),
            },
            Err(e) => Outcome::Err {
                error: RpcError::internal(e.to_string()),
            },
        },
        Method::WikiTopicDetail(params) => match wiki_topic_detail(state, params) {
            Ok(Some(detail)) => Outcome::Ok {
                result: ResponsePayload::WikiTopicDetail(detail),
            },
            Ok(None) => Outcome::Err {
                error: RpcError {
                    code: RpcError::INVALID_PARAMS,
                    message: "topic not found".into(),
                },
            },
            Err(e) => Outcome::Err {
                error: RpcError::internal(e.to_string()),
            },
        },
        Method::WikiSearch(params) => match wiki_search(state, params) {
            Ok(list) => Outcome::Ok {
                result: ResponsePayload::WikiSearch(list),
            },
            Err(e) => Outcome::Err {
                error: RpcError::internal(e.to_string()),
            },
        },
    };
    Dispatch::Reply(Response { id, outcome })
}

pub fn handle_notification(_state: &SidecarState, note: Notification) {
    match note.event {
        NotifyIn::ShellExiting => {
            log::info!("shell announced exit");
        }
    }
}

fn index_messages_batch(
    state: &SidecarState,
    params: IndexBatchParams,
) -> Result<u64, sqlite::Error> {
    let rows: Vec<MessageRow> = params.messages.into_iter().map(to_message_row).collect();
    let count = rows.len() as u64;
    if rows.is_empty() {
        return Ok(0);
    }
    let store = state.lock_store();
    store.insert_messages_batch(&rows)?;
    Ok(count)
}

fn to_message_row(msg: IndexMessageInput) -> MessageRow {
    let stripped = strip_whitespace(&msg.text);
    MessageRow {
        message_id: msg.message_id,
        chat_id: msg.chat_id,
        timestamp: msg.timestamp,
        text_plain: msg.text,
        text_stripped: stripped,
        link: None,
    }
}

fn topic_to_summary(t: &WikiTopic) -> WikiTopicSummary {
    WikiTopicSummary {
        id: t.topic_id,
        title: t.title.clone(),
        title_ko: t.title_ko.clone(),
        category: t
            .category_name
            .clone()
            .unwrap_or_else(|| "Uncategorized".into()),
        message_count: t.message_count,
        trending_score: t.trending_score,
    }
}

fn wiki_trending(
    state: &SidecarState,
    params: WikiTrendingParams,
) -> Result<Vec<WikiTopicSummary>, sqlite::Error> {
    let store = state.lock_store();
    // Category filter is passed as a name by the shell; resolve to id
    // before hitting the store. Missing categories just return no
    // topics.
    let category_id = match params.category.as_deref() {
        Some(name) => {
            let all = store.get_all_categories()?;
            match all.iter().find(|c| c.name.eq_ignore_ascii_case(name)) {
                Some(c) => Some(c.category_id),
                None => return Ok(Vec::new()),
            }
        }
        None => None,
    };
    let topics = store.get_trending_topics(params.limit, params.offset, category_id)?;
    Ok(topics.iter().map(topic_to_summary).collect())
}

fn wiki_topic_detail(
    state: &SidecarState,
    params: WikiTopicDetailParams,
) -> Result<Option<WikiTopicDetail>, sqlite::Error> {
    let store = state.lock_store();
    let topic = match store.get_topic(params.topic_id)? {
        Some(t) => t,
        None => return Ok(None),
    };
    let summary = topic_to_summary(&topic);
    let page = store.get_latest_page(params.topic_id)?;
    Ok(Some(WikiTopicDetail {
        summary,
        article_md: page.as_ref().map(|p| p.content_en.clone()),
        article_md_ko: page.as_ref().map(|p| p.content_ko.clone()),
    }))
}

fn wiki_search(
    state: &SidecarState,
    params: WikiSearchParams,
) -> Result<Vec<WikiTopicSummary>, sqlite::Error> {
    let store = state.lock_store();
    let hits = store.search_wiki_pages(&params.query, params.limit)?;
    let mut out = Vec::with_capacity(hits.len());
    for hit in hits {
        if let Some(topic) = store.get_topic(hit.topic_id)? {
            out.push(topic_to_summary(&topic));
        }
    }
    Ok(out)
}

fn run_search(state: &SidecarState, params: SearchParams) -> Result<SearchResult, sqlite::Error> {
    let scope = match params.scope {
        SearchScopeInput::All => engine::SearchScope::All,
        SearchScopeInput::Chat(id) => engine::SearchScope::Chat(id),
    };
    let store = state.lock_store();
    engine::search(
        &store,
        &params.query,
        &scope,
        params.cursor.as_ref(),
        params.limit,
    )
}
