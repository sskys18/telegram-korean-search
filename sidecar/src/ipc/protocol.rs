//! Request/response shapes for the IPC protocol.
//!
//! The shape is deliberately close to JSON-RPC 2.0 but not fully
//! compliant: we drop the `jsonrpc` version field and treat the
//! `method` field as an enum so it can be matched exhaustively on
//! the Rust side. The Swift client sends JSON with the same shape
//! and parses the response via `Codable`.
//!
//! A request with no `id` is a notification and MUST NOT produce a
//! response. Requests with an `id` MUST get exactly one response.
//! Server-initiated events (wiki progress, etc.) are pushed as
//! notifications on the same connection.

use serde::{Deserialize, Serialize};

use crate::search::SearchResult;
use crate::store::message::Cursor;

/// Incoming message from the Swift client.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Incoming {
    Request(Request),
    Notification(Notification),
}

#[derive(Debug, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub call: Method,
}

#[derive(Debug, Deserialize)]
pub struct Notification {
    #[serde(flatten)]
    pub event: NotifyIn,
}

/// Client → server methods. The discriminant is the `method` field
/// and method-specific parameters live under `params`.
#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Method {
    Ping,
    Shutdown,

    IndexMessagesBatch(IndexBatchParams),
    DeleteMessage(DeleteMessageParams),

    Search(SearchParams),

    WikiTrending(WikiTrendingParams),
    WikiTopicDetail(WikiTopicDetailParams),
    WikiSearch(WikiSearchParams),
}

/// Client → server notifications (fire-and-forget).
#[derive(Debug, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum NotifyIn {
    /// Hint the sidecar that the shell is closing. Best-effort; the
    /// sidecar should also watch its parent PID.
    ShellExiting,
}

/// Server-initiated push events. Wrapped in [`OutgoingFrame::Event`]
/// before being sent.
#[derive(Debug, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "snake_case")]
pub enum ServerEvent {
    WikiProgress {
        processed: u64,
        pending: u64,
        total: u64,
    },
    WikiError {
        message: String,
        recoverable: bool,
    },
}

/// What the sidecar writes back on the wire.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum OutgoingFrame {
    Response(Response),
    Event(Event),
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub id: u64,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Outcome {
    Ok { result: ResponsePayload },
    Err { error: RpcError },
}

#[derive(Debug, Serialize)]
pub struct Event {
    #[serde(flatten)]
    pub body: ServerEvent,
}

#[derive(Debug, Serialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl RpcError {
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL: i32 = -32603;

    pub fn method_not_found(method: impl Into<String>) -> Self {
        Self {
            code: Self::METHOD_NOT_FOUND,
            message: format!("method not found: {}", method.into()),
        }
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self {
            code: Self::INTERNAL,
            message: msg.into(),
        }
    }
}

/// Method-specific success payloads. Matched 1-to-1 with [`Method`].
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ResponsePayload {
    Pong(PongResult),
    ShutdownAck,
    IndexBatch(IndexBatchResult),
    DeleteAck,
    Search(SearchResult),
    WikiTrending(Vec<WikiTopicSummary>),
    WikiTopicDetail(WikiTopicDetail),
    WikiSearch(Vec<WikiTopicSummary>),
}

#[derive(Debug, Serialize)]
pub struct PongResult {
    pub version: &'static str,
}

// ---------- method-specific params and payloads ----------

#[derive(Debug, Deserialize)]
pub struct IndexBatchParams {
    pub messages: Vec<IndexMessageInput>,
}

/// A single message the Swift shell wants mirrored into the sidecar.
/// Mirrors [`crate::store::message::NewMessage`] on the Rust side
/// but stays decoupled so the wire format can evolve independently.
#[derive(Debug, Deserialize, Clone)]
pub struct IndexMessageInput {
    pub chat_id: i64,
    pub message_id: i64,
    pub sender_id: Option<i64>,
    pub sender_name: Option<String>,
    pub timestamp: i64,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct IndexBatchResult {
    pub inserted: u64,
    pub updated: u64,
}

#[derive(Debug, Deserialize)]
pub struct DeleteMessageParams {
    pub chat_id: i64,
    pub message_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub query: String,
    #[serde(default)]
    pub scope: SearchScopeInput,
    pub limit: Option<usize>,
    pub cursor: Option<Cursor>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(tag = "kind", content = "chat_id", rename_all = "snake_case")]
pub enum SearchScopeInput {
    #[default]
    All,
    Chat(i64),
}

#[derive(Debug, Deserialize)]
pub struct WikiTrendingParams {
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
    pub category: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WikiTopicDetailParams {
    pub topic_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct WikiSearchParams {
    pub query: String,
    #[serde(default = "default_wiki_limit")]
    pub limit: usize,
}

fn default_wiki_limit() -> usize {
    20
}

// ---------- placeholder shapes until wiki handlers are wired up ----------

#[derive(Debug, Serialize)]
pub struct WikiTopicSummary {
    pub id: i64,
    pub title: String,
    pub title_ko: Option<String>,
    pub category: String,
    pub message_count: i64,
    pub trending_score: f64,
}

#[derive(Debug, Serialize)]
pub struct WikiTopicDetail {
    pub summary: WikiTopicSummary,
    pub article_md: Option<String>,
    pub article_md_ko: Option<String>,
}
