use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::collector;
use crate::AppState;

#[derive(Serialize)]
pub struct ApiCredentials {
    pub api_id: i32,
    pub api_hash: String,
}

#[derive(Serialize)]
pub struct ConnectResult {
    pub authorized: bool,
}

#[derive(Serialize)]
pub struct SignInResponse {
    pub success: bool,
    pub requires_2fa: bool,
    pub hint: Option<String>,
}

/// Read saved API credentials from the database.
#[tauri::command]
pub fn get_api_credentials(state: State<AppState>) -> Result<Option<ApiCredentials>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let api_id = store.get_meta("tg_api_id").map_err(|e| e.to_string())?;
    let api_hash = store.get_meta("tg_api_hash").map_err(|e| e.to_string())?;
    match (api_id, api_hash) {
        (Some(id_str), Some(hash)) => {
            let id: i32 = id_str
                .parse()
                .map_err(|_| "invalid api_id in database".to_string())?;
            Ok(Some(ApiCredentials {
                api_id: id,
                api_hash: hash,
            }))
        }
        _ => Ok(None),
    }
}

/// Save API credentials to the database.
#[tauri::command]
pub fn save_api_credentials(
    state: State<AppState>,
    api_id: i32,
    api_hash: String,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .set_meta("tg_api_id", &api_id.to_string())
        .map_err(|e| e.to_string())?;
    store
        .set_meta("tg_api_hash", &api_hash)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Connect to Telegram using saved credentials.
/// Stores the client in AppState and checks if already authorized.
/// If the session is stale (AUTH_KEY_UNREGISTERED), deletes it and reconnects fresh.
#[tauri::command]
pub async fn connect_telegram(state: State<'_, AppState>) -> Result<ConnectResult, String> {
    // Read api_id and auth flag from DB
    let (api_id, was_authenticated) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let id_str = store
            .get_meta("tg_api_id")
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "API credentials not configured".to_string())?;
        let api_id = id_str
            .parse::<i32>()
            .map_err(|_| "invalid api_id in database".to_string())?;
        let authenticated = store
            .get_meta("tg_authenticated")
            .map_err(|e| e.to_string())?
            .map_or(false, |v| v == "1");
        (api_id, authenticated)
    };

    let session_path = collector::session_path();

    // Abort any existing runner before connecting
    if let Some(old) = state.runner_handle.lock().await.take() {
        old.abort();
    }
    // Clear the old client
    *state.client.lock().await = None;

    // Only try to reuse an existing session if login was previously completed.
    // Otherwise, delete any leftover session file to avoid stale auth key issues.
    if was_authenticated && session_path.exists() {
        let (client, runner) = collector::connect(api_id)
            .await
            .map_err(|e| e.to_string())?;

        let auth_check = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            collector::auth::is_authorized(&client),
        )
        .await;

        match auth_check {
            Ok(Ok(true)) => {
                *state.client.lock().await = Some(client);
                *state.runner_handle.lock().await = Some(runner);
                return Ok(ConnectResult { authorized: true });
            }
            _ => {
                // Session expired — clean up and reconnect fresh.
                runner.abort();
                let _ = std::fs::remove_file(&session_path);
                // Clear the authenticated flag since session is no longer valid.
                let store = state.store.lock().map_err(|e| e.to_string())?;
                let _ = store.delete_meta("tg_authenticated");
                log::info!("Session expired, reconnecting fresh");
            }
        }
    } else if session_path.exists() {
        // Session file exists but user never completed login — just delete it.
        let _ = std::fs::remove_file(&session_path);
    }

    // Fresh connection
    let (client, runner) = collector::connect(api_id)
        .await
        .map_err(|e| e.to_string())?;

    *state.client.lock().await = Some(client);
    *state.runner_handle.lock().await = Some(runner);

    Ok(ConnectResult { authorized: false })
}

/// Request a login code for the given phone number.
#[tauri::command]
pub async fn request_login_code(state: State<'_, AppState>, phone: String) -> Result<(), String> {
    let client_guard = state.client.lock().await;
    let client = client_guard
        .as_ref()
        .ok_or_else(|| "Client not connected".to_string())?;

    // Read api_hash from DB for the login code request
    let api_hash = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .get_meta("tg_api_hash")
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "API credentials not configured".to_string())?
    };

    let token = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        collector::auth::request_login_code(client, &phone, &api_hash),
    )
    .await
    .map_err(|_| "Connection timed out. Please try again.".to_string())?
    .map_err(|e| e.to_string())?;
    *state.login_token.lock().await = Some(token);
    Ok(())
}

/// Submit the login code. Returns whether 2FA is needed.
#[tauri::command]
pub async fn submit_login_code(
    state: State<'_, AppState>,
    code: String,
) -> Result<SignInResponse, String> {
    let client_guard = state.client.lock().await;
    let client = client_guard
        .as_ref()
        .ok_or_else(|| "Client not connected".to_string())?;

    let token = state
        .login_token
        .lock()
        .await
        .take()
        .ok_or_else(|| "No login token. Call request_login_code first.".to_string())?;

    let result = collector::auth::sign_in(client, &token, &code)
        .await
        .map_err(|e| e.to_string())?;

    match result {
        collector::auth::SignInResult::Success => {
            // Mark as authenticated so we can reuse the session on next launch.
            let store = state.store.lock().map_err(|e| e.to_string())?;
            let _ = store.set_meta("tg_authenticated", "1");
            Ok(SignInResponse {
                success: true,
                requires_2fa: false,
                hint: None,
            })
        }
        collector::auth::SignInResult::TwoFactorRequired {
            password_token,
            hint,
        } => {
            *state.password_token.lock().await = Some(password_token);
            Ok(SignInResponse {
                success: false,
                requires_2fa: true,
                hint: Some(hint),
            })
        }
    }
}

/// Submit 2FA password.
#[tauri::command]
pub async fn submit_password(state: State<'_, AppState>, password: String) -> Result<(), String> {
    let client_guard = state.client.lock().await;
    let client = client_guard
        .as_ref()
        .ok_or_else(|| "Client not connected".to_string())?;

    let token = state
        .password_token
        .lock()
        .await
        .take()
        .ok_or_else(|| "No password token. Complete sign_in first.".to_string())?;

    collector::auth::check_password(client, *token, &password)
        .await
        .map_err(|e| e.to_string())?;

    // Mark as authenticated so we can reuse the session on next launch.
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let _ = store.set_meta("tg_authenticated", "1");
    Ok(())
}

/// Start initial message collection in a background thread.
/// Emits progress events: "collection-progress", "collection-complete", "collection-error".
#[tauri::command]
pub async fn start_collection(app: AppHandle) -> Result<(), String> {
    let client = app
        .state::<AppState>()
        .client
        .lock()
        .await
        .as_ref()
        .ok_or_else(|| "Client not connected".to_string())?
        .clone();

    std::thread::spawn(move || {
        run_collection(app, client);
    });

    Ok(())
}

// Runs on a dedicated thread with a single-threaded tokio runtime.
// The store lock is acquired only for brief DB writes, never during network I/O,
// so other Tauri commands (search, get_chats) remain responsive.
fn run_collection(app: AppHandle, client: grammers_client::Client) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let state = app.state::<AppState>();

        // Phase 1: Fetch chats from network (no store lock needed)
        let _ = app.emit(
            "collection-progress",
            serde_json::json!({
                "phase": "chats",
                "detail": "Fetching chat list..."
            }),
        );

        let chat_rows = match collector::messages::fetch_chats(&client).await {
            Ok(rows) => rows,
            Err(e) => {
                log::error!("Chat fetch failed: {}", e);
                let _ = app.emit("collection-error", e.to_string());
                return;
            }
        };

        // Brief lock: save chats and read active list
        let (chats, chat_count) = {
            let store = state.store.lock().unwrap();
            for row in &chat_rows {
                if let Err(e) = store.upsert_chat(row) {
                    log::warn!("Failed to save chat {}: {}", row.title, e);
                }
            }
            let active = store.get_active_chats().unwrap_or_default();
            let count = chat_rows.len();
            (active, count)
        };

        // Phase 2: Fetch messages per chat (FTS5 indexing is automatic)
        for (i, chat) in chats.iter().enumerate() {
            let _ = app.emit(
                "collection-progress",
                serde_json::json!({
                    "phase": "messages",
                    "chat_title": &chat.title,
                    "chats_done": i,
                    "chats_total": chat_count,
                }),
            );

            // Network I/O: no store lock held
            match collector::messages::fetch_messages(&client, chat, None).await {
                Ok(rows) => {
                    let count = rows.len();
                    if !rows.is_empty() {
                        // Brief lock: save messages
                        let store = state.store.lock().unwrap();
                        if let Err(e) = store.insert_messages_batch(&rows) {
                            log::warn!("Failed to save messages for {}: {}", chat.title, e);
                        }
                    }
                    log::info!("Fetched {} messages for {}", count, chat.title);
                }
                Err(e) => log::warn!("Failed to fetch messages for {}: {}", chat.title, e),
            }

            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }

        let _ = app.emit(
            "collection-complete",
            serde_json::json!({ "chats": chat_count }),
        );
    });
}
