pub mod crypto;
pub mod keychain;

use std::path::PathBuf;

const SESSION_FILENAME: &str = "session.bin";

pub fn default_session_path() -> PathBuf {
    dirs::data_dir()
        .expect("could not determine data directory")
        .join("telegram-korean-search")
        .join(SESSION_FILENAME)
}

/// Save encrypted session data to disk.
/// Creates the parent directory if it doesn't exist.
pub fn save_session(data: &[u8]) -> Result<(), SessionError> {
    let key = keychain::get_or_create_key()?;
    let encrypted = crypto::encrypt(&key, data)?;

    let path = default_session_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(SessionError::Io)?;
    }
    std::fs::write(&path, &encrypted).map_err(SessionError::Io)?;
    Ok(())
}

/// Load and decrypt session data from disk.
/// Returns `None` if the session file doesn't exist.
pub fn load_session() -> Result<Option<Vec<u8>>, SessionError> {
    let path = default_session_path();
    if !path.exists() {
        return Ok(None);
    }

    let key = keychain::get_or_create_key()?;
    let encrypted = std::fs::read(&path).map_err(SessionError::Io)?;
    let plaintext = crypto::decrypt(&key, &encrypted)?;
    Ok(Some(plaintext))
}

/// Delete the session file from disk.
pub fn delete_session() -> Result<(), SessionError> {
    let path = default_session_path();
    if path.exists() {
        std::fs::remove_file(&path).map_err(SessionError::Io)?;
    }
    Ok(())
}

#[derive(Debug)]
pub enum SessionError {
    Io(std::io::Error),
    Crypto(crypto::CryptoError),
    Keychain(keychain::KeychainError),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Io(e) => write!(f, "IO error: {}", e),
            SessionError::Crypto(e) => write!(f, "Crypto error: {}", e),
            SessionError::Keychain(e) => write!(f, "Keychain error: {}", e),
        }
    }
}

impl From<crypto::CryptoError> for SessionError {
    fn from(e: crypto::CryptoError) -> Self {
        SessionError::Crypto(e)
    }
}

impl From<keychain::KeychainError> for SessionError {
    fn from(e: keychain::KeychainError) -> Self {
        SessionError::Keychain(e)
    }
}
