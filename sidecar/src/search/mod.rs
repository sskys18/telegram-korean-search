pub mod engine;
pub mod highlight;

use serde::{Deserialize, Serialize};

use crate::store::message::Cursor;
use highlight::HighlightRange;

/// A single search result item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchItem {
    pub message_id: i64,
    pub chat_id: i64,
    pub timestamp: i64,
    pub text: String,
    pub link: Option<String>,
    pub chat_title: String,
    pub highlights: Vec<HighlightRange>,
}

/// Paginated search results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub items: Vec<SearchItem>,
    pub next_cursor: Option<Cursor>,
}
