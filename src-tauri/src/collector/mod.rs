pub mod auth;
pub mod link;
pub mod messages;

use std::path::PathBuf;
use std::sync::Arc;

use grammers_client::Client;
use grammers_mtsender::SenderPool;
use grammers_session::storages::SqliteSession;

/// Default Telegram API credentials (bundled).
/// Can be overridden via TG_API_ID / TG_API_HASH environment variables.
const DEFAULT_API_ID: i32 = 0; // Replace with real API ID before release
const DEFAULT_API_HASH: &str = ""; // Replace with real API hash before release

pub fn api_id() -> i32 {
    std::env::var("TG_API_ID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_API_ID)
}

pub fn api_hash() -> String {
    std::env::var("TG_API_HASH")
        .ok()
        .unwrap_or_else(|| DEFAULT_API_HASH.to_string())
}

pub fn session_path() -> PathBuf {
    dirs::data_dir()
        .expect("could not determine data directory")
        .join("telegram-korean-search")
        .join("telegram.session")
}

/// Create a connected Telegram client.
/// Returns the client and a runner join handle. The runner must be kept alive.
pub async fn connect() -> Result<(Client, tokio::task::JoinHandle<()>), CollectorError> {
    let path = session_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(CollectorError::Io)?;
    }

    let session = Arc::new(
        SqliteSession::open(path.to_str().ok_or(CollectorError::InvalidPath)?)
            .map_err(|e| CollectorError::Session(e.to_string()))?,
    );

    let pool = SenderPool::new(Arc::clone(&session), api_id());
    let client = Client::new(&pool);

    // Destructure to take ownership of the runner
    let SenderPool { runner, .. } = pool;
    let runner_handle = tokio::spawn(async move {
        runner.run().await;
    });

    Ok((client, runner_handle))
}

#[derive(Debug)]
pub enum CollectorError {
    Io(std::io::Error),
    Session(String),
    Auth(String),
    Api(String),
    InvalidPath,
}

impl std::fmt::Display for CollectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CollectorError::Io(e) => write!(f, "IO error: {}", e),
            CollectorError::Session(e) => write!(f, "session error: {}", e),
            CollectorError::Auth(e) => write!(f, "auth error: {}", e),
            CollectorError::Api(e) => write!(f, "API error: {}", e),
            CollectorError::InvalidPath => write!(f, "invalid session path"),
        }
    }
}
