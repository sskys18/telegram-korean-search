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
#[tauri::command]
pub async fn connect_telegram(state: State<'_, AppState>) -> Result<ConnectResult, String> {
    // Read api_id from DB
    let api_id = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        let id_str = store
            .get_meta("tg_api_id")
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "API credentials not configured".to_string())?;
        id_str
            .parse::<i32>()
            .map_err(|_| "invalid api_id in database".to_string())?
    };

    let (client, runner) = collector::connect(api_id)
        .await
        .map_err(|e| e.to_string())?;

    let authorized = collector::auth::is_authorized(&client)
        .await
        .unwrap_or(false);

    // Store client and runner. If OnceCell already set, ignore (reconnect not supported).
    let _ = state.client.set(client);
    *state.runner_handle.lock().await = Some(runner);

    Ok(ConnectResult { authorized })
}

/// Request a login code for the given phone number.
#[tauri::command]
pub async fn request_login_code(state: State<'_, AppState>, phone: String) -> Result<(), String> {
    let client = state
        .client
        .get()
        .ok_or_else(|| "Client not connected".to_string())?;

    // Read api_hash from DB for the login code request
    let api_hash = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        store
            .get_meta("tg_api_hash")
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "API credentials not configured".to_string())?
    };

    let token = collector::auth::request_login_code(client, &phone, &api_hash)
        .await
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
    let client = state
        .client
        .get()
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
        collector::auth::SignInResult::Success => Ok(SignInResponse {
            success: true,
            requires_2fa: false,
            hint: None,
        }),
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
    let client = state
        .client
        .get()
        .ok_or_else(|| "Client not connected".to_string())?;

    let token = state
        .password_token
        .lock()
        .await
        .take()
        .ok_or_else(|| "No password token. Complete sign_in first.".to_string())?;

    collector::auth::check_password(client, *token, &password)
        .await
        .map_err(|e| e.to_string())
}

/// Start initial message collection in a background thread.
/// Emits progress events: "collection-progress", "collection-complete", "collection-error".
#[tauri::command]
pub async fn start_collection(app: AppHandle) -> Result<(), String> {
    let client = app
        .state::<AppState>()
        .client
        .get()
        .ok_or_else(|| "Client not connected".to_string())?
        .clone();

    std::thread::spawn(move || {
        run_collection(app, client);
    });

    Ok(())
}

// Runs on a dedicated thread with a single-threaded tokio runtime.
// The std::sync::MutexGuard<Store> is held across .await points, which is safe
// because block_on runs everything on this single thread (no Send required).
#[allow(clippy::await_holding_lock)]
fn run_collection(app: AppHandle, client: grammers_client::Client) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        let state = app.state::<AppState>();

        // Phase 1: Fetch chats
        let _ = app.emit(
            "collection-progress",
            serde_json::json!({
                "phase": "chats",
                "detail": "Fetching chat list..."
            }),
        );

        let store = state.store.lock().unwrap();
        let chat_count = match collector::messages::fetch_and_save_chats(&client, &store).await {
            Ok(count) => count,
            Err(e) => {
                log::error!("Chat fetch failed: {}", e);
                let _ = app.emit("collection-error", e.to_string());
                return;
            }
        };

        let chats = store.get_active_chats().unwrap_or_default();
        drop(store);

        // Phase 2: Fetch messages per chat
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

            let store = state.store.lock().unwrap();
            match collector::messages::fetch_messages_for_chat(&client, &store, chat, None).await {
                Ok(count) => log::info!("Fetched {} messages for {}", count, chat.title),
                Err(e) => log::warn!("Failed to fetch messages for {}: {}", chat.title, e),
            }
            drop(store);

            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }

        let _ = app.emit(
            "collection-complete",
            serde_json::json!({ "chats": chat_count }),
        );
    });
}
