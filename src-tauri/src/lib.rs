pub mod collector;
pub mod commands;
pub mod error;
pub mod logging;
pub mod search;
pub mod security;
pub mod store;

use grammers_client::types::LoginToken;
use grammers_client::Client;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use store::message::Cursor;
use store::Store;
use tauri::State;
use tokio::sync::Mutex as TokioMutex;

pub struct AppState {
    pub store: Mutex<Store>,
    pub client: TokioMutex<Option<Client>>,
    pub login_token: TokioMutex<Option<LoginToken>>,
    pub password_token: TokioMutex<Option<Box<grammers_client::types::PasswordToken>>>,
    pub runner_handle: TokioMutex<Option<tokio::task::JoinHandle<()>>>,
}

#[derive(Debug, Serialize)]
pub struct DbStats {
    pub chats: i64,
    pub messages: i64,
}

#[tauri::command]
fn get_db_stats(state: State<AppState>) -> Result<DbStats, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    Ok(DbStats {
        chats: store.chat_count().map_err(|e| e.to_string())?,
        messages: store.message_count().map_err(|e| e.to_string())?,
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

#[tauri::command]
fn set_chat_excluded(state: State<AppState>, chat_id: i64, excluded: bool) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .set_chat_excluded(chat_id, excluded)
        .map_err(|e| e.to_string())
}

pub fn run() {
    // Initialize logging
    let log_dir = store::app_data_dir();
    if let Err(e) = logging::init(&log_dir) {
        eprintln!("Failed to initialize logging: {}", e);
    }

    log::info!(
        "Starting telegram-korean-search v{}",
        env!("CARGO_PKG_VERSION")
    );

    let db_path = store::default_db_path();
    let store = Store::open(&db_path).expect("failed to open database");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState {
            store: Mutex::new(store),
            client: TokioMutex::new(None),
            login_token: TokioMutex::new(None),
            password_token: TokioMutex::new(None),
            runner_handle: TokioMutex::new(None),
        })
        .setup(|app| {
            #[cfg(desktop)]
            {
                use tauri::Manager;
                use tauri_plugin_global_shortcut::{Code, Modifiers, ShortcutState};

                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_shortcuts(["super+shift+f"])?
                        .with_handler(|app, shortcut, event| {
                            if event.state == ShortcutState::Pressed
                                && shortcut.matches(Modifiers::SUPER | Modifiers::SHIFT, Code::KeyF)
                            {
                                if let Some(window) = app.get_webview_window("main") {
                                    if window.is_visible().unwrap_or(false) {
                                        let _ = window.hide();
                                    } else {
                                        let _ = window.show();
                                        let _ = window.set_focus();
                                    }
                                }
                            }
                        })
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_db_stats,
            search_messages,
            get_chats,
            set_chat_excluded,
            commands::get_api_credentials,
            commands::save_api_credentials,
            commands::connect_telegram,
            commands::request_login_code,
            commands::submit_login_code,
            commands::submit_password,
            commands::start_collection,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // Gracefully shut down the Telegram runner on app exit
                // to prevent stale session files.
                use tauri::Manager;
                let handle = {
                    let state = app.state::<AppState>();
                    state.runner_handle.try_lock().ok().and_then(|mut g| g.take())
                };
                if let Some(h) = handle {
                    h.abort();
                    log::info!("Telegram runner stopped on exit");
                }
            }
        });
}
