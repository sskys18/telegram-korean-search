use std::fmt;

/// Unified error type for Tauri command responses.
#[derive(Debug)]
pub enum AppError {
    Store(sqlite::Error),
    Collector(crate::collector::CollectorError),
    Other(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Store(e) => write!(f, "Store error: {}", e),
            AppError::Collector(e) => write!(f, "Collector error: {}", e),
            AppError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl From<sqlite::Error> for AppError {
    fn from(e: sqlite::Error) -> Self {
        AppError::Store(e)
    }
}

impl From<crate::collector::CollectorError> for AppError {
    fn from(e: crate::collector::CollectorError) -> Self {
        AppError::Collector(e)
    }
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError::Other(s)
    }
}

// Tauri requires commands to return Result<T, String> or implement Serialize on error.
// We convert via Display for simplicity.
impl From<AppError> for String {
    fn from(e: AppError) -> Self {
        e.to_string()
    }
}
