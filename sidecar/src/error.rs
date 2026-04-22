use std::fmt;

#[derive(Debug)]
pub enum AppError {
    Store(sqlite::Error),
    Other(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Store(e) => write!(f, "Store error: {}", e),
            AppError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl From<sqlite::Error> for AppError {
    fn from(e: sqlite::Error) -> Self {
        AppError::Store(e)
    }
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        AppError::Other(s)
    }
}

impl From<AppError> for String {
    fn from(e: AppError) -> Self {
        e.to_string()
    }
}
