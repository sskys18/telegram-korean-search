pub mod app_meta;
pub mod chat;
pub mod index_store;
pub mod message;
pub mod schema;
pub mod sync_state;

use rusqlite::Connection;
use std::path::PathBuf;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(db_path: &PathBuf) -> Result<Self, rusqlite::Error> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(db_path)?;
        Self::configure(&conn)?;
        schema::run_migrations(&conn)?;
        Ok(Store { conn })
    }

    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        schema::run_migrations(&conn)?;
        Ok(Store { conn })
    }

    fn configure(conn: &Connection) -> Result<(), rusqlite::Error> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -64000;
             PRAGMA foreign_keys = ON;",
        )?;
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}

pub fn default_db_path() -> PathBuf {
    let mut path = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("telegram-korean-search");
    path.push("tg-korean-search.db");
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let store = Store::open_in_memory().unwrap();
        let mode: String = store
            .conn()
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        // In-memory databases use "memory" journal mode, not WAL
        assert!(mode == "wal" || mode == "memory");
    }

    #[test]
    fn test_migrations_idempotent() {
        let store = Store::open_in_memory().unwrap();
        // Running migrations again should not error
        schema::run_migrations(store.conn()).unwrap();
        schema::run_migrations(store.conn()).unwrap();
    }

    #[test]
    fn test_default_db_path() {
        let path = default_db_path();
        assert!(path.to_string_lossy().contains("telegram-korean-search"));
        assert!(path.to_string_lossy().contains("tg-korean-search.db"));
    }
}
