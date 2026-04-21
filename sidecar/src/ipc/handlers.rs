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
};
use crate::search::{engine, SearchResult};
use crate::store::message::{strip_whitespace, MessageRow};
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
        Method::WikiTrending(_) | Method::WikiTopicDetail(_) | Method::WikiSearch(_) => {
            Outcome::Err {
                error: RpcError {
                    code: RpcError::METHOD_NOT_FOUND,
                    message: "wiki methods not yet wired up".into(),
                },
            }
        }
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
