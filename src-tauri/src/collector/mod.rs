pub mod auth;
pub mod link;
pub mod messages;

use std::path::PathBuf;
use std::sync::Arc;

use grammers_client::Client;
use grammers_mtsender::SenderPool;
use grammers_session::storages::SqliteSession;

pub fn session_path() -> PathBuf {
    dirs::data_dir()
        .expect("could not determine data directory")
        .join("telegram-korean-search")
        .join("telegram.session")
}

/// Create a connected Telegram client.
/// Returns the client and a runner join handle. The runner must be kept alive.
pub async fn connect(api_id: i32) -> Result<(Client, tokio::task::JoinHandle<()>), CollectorError> {
    let path = session_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(CollectorError::Io)?;
    }

    let session = Arc::new(
        SqliteSession::open(path.to_str().ok_or(CollectorError::InvalidPath)?)
            .map_err(|e| CollectorError::Session(e.to_string()))?,
    );

    let pool = SenderPool::new(Arc::clone(&session), api_id);
    let client = Client::new(&pool);

    // Destructure to take ownership of the runner.
    // Install a panic hook that suppresses grammers-session panics (e.g. stale session
    // causing AUTH_KEY_UNREGISTERED → session SQLite write failure). These panics are
    // expected and handled by the stale session recovery in connect_telegram.
    let SenderPool { runner, .. } = pool;
    install_grammers_panic_hook();
    let runner_handle = tokio::spawn(async move {
        runner.run().await;
    });

    Ok((client, runner_handle))
}

/// Replace the default panic hook with one that suppresses panics from grammers-session
/// (e.g. SQLite errors from stale sessions). Other panics are forwarded to the default hook.
fn install_grammers_panic_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let from_grammers = info.location().is_some_and(|loc| {
                loc.file().contains("grammers-session") || loc.file().contains("grammers_session")
            });
            if from_grammers {
                log::warn!("Telegram session error (recovering automatically)");
            } else {
                default_hook(info);
            }
        }));
    });
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
