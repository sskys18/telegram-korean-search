pub mod collector;
pub mod indexer;
pub mod search;
pub mod security;
pub mod store;

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use store::message::Cursor;
use store::Store;
use tauri::State;

pub struct AppState {
    pub store: Mutex<Store>,
}

#[derive(Debug, Serialize)]
pub struct DbStats {
    pub chats: i64,
    pub messages: i64,
    pub terms: i64,
    pub postings: i64,
}

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}!", name)
}

#[tauri::command]
fn get_db_stats(state: State<AppState>) -> Result<DbStats, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    Ok(DbStats {
        chats: store.chat_count().map_err(|e| e.to_string())?,
        messages: store.message_count().map_err(|e| e.to_string())?,
        terms: store.term_count().map_err(|e| e.to_string())?,
        postings: store.posting_count().map_err(|e| e.to_string())?,
    })
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    query: String,
    chat_id: Option<i64>,
    cursor: Option<Cursor>,
    limit: Option<usize>,
}

#[tauri::command]
fn search_messages(
    state: State<AppState>,
    params: SearchQuery,
) -> Result<search::SearchResult, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let scope = match params.chat_id {
        Some(id) => search::engine::SearchScope::Chat(id),
        None => search::engine::SearchScope::All,
    };
    search::engine::search(
        &store,
        &params.query,
        &scope,
        params.cursor.as_ref(),
        params.limit,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_chats(state: State<AppState>) -> Result<Vec<store::chat::ChatRow>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.get_all_chats().map_err(|e| e.to_string())
}

pub fn run() {
    let db_path = store::default_db_path();
    let store = Store::open(&db_path).expect("failed to open database");

    tauri::Builder::default()
        .manage(AppState {
            store: Mutex::new(store),
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            get_db_stats,
            search_messages,
            get_chats,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
