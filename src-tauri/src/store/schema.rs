use sqlite::Connection;

pub fn run_migrations(conn: &Connection) -> Result<(), sqlite::Error> {
    // Phase 1: Create base tables (idempotent)
    conn.execute(
        "
        CREATE TABLE IF NOT EXISTS chats (
            chat_id       INTEGER PRIMARY KEY,
            title         TEXT NOT NULL,
            chat_type     TEXT NOT NULL CHECK (chat_type IN ('group', 'supergroup', 'channel')),
            username      TEXT,
            access_hash   INTEGER,
            is_excluded   INTEGER NOT NULL DEFAULT 0,
            created_at    TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS messages (
            message_id    INTEGER NOT NULL,
            chat_id       INTEGER NOT NULL,
            timestamp     INTEGER NOT NULL,
            text_plain    TEXT NOT NULL,
            text_stripped TEXT NOT NULL,
            link          TEXT,
            PRIMARY KEY (chat_id, message_id),
            FOREIGN KEY (chat_id) REFERENCES chats(chat_id)
        );

        CREATE INDEX IF NOT EXISTS idx_messages_timestamp
            ON messages (timestamp DESC);
        CREATE INDEX IF NOT EXISTS idx_messages_chat_timestamp
            ON messages (chat_id, timestamp DESC);

        CREATE TABLE IF NOT EXISTS sync_state (
            chat_id           INTEGER PRIMARY KEY,
            last_message_id   INTEGER NOT NULL DEFAULT 0,
            oldest_message_id INTEGER,
            initial_done      INTEGER NOT NULL DEFAULT 0,
            last_sync_at      TEXT,
            FOREIGN KEY (chat_id) REFERENCES chats(chat_id)
        );

        CREATE TABLE IF NOT EXISTS app_meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        ",
    )?;

    // Phase 2: Versioned migration — FTS5 trigram
    migrate_to_fts5(conn)?;

    Ok(())
}

fn get_schema_version(conn: &Connection) -> i64 {
    let mut stmt = match conn.prepare("SELECT value FROM app_meta WHERE key = 'schema_version'") {
        Ok(s) => s,
        Err(_) => return 1,
    };
    if let Ok(sqlite::State::Row) = stmt.next() {
        stmt.read::<String, _>(0)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1)
    } else {
        1
    }
}

fn migrate_to_fts5(conn: &Connection) -> Result<(), sqlite::Error> {
    if get_schema_version(conn) >= 2 {
        return Ok(());
    }

    conn.execute(
        "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
            text_plain,
            content='messages',
            tokenize='trigram case_sensitive 0'
        )",
    )?;

    // Rebuild FTS5 index from any existing messages (idempotent)
    conn.execute("INSERT INTO messages_fts(messages_fts) VALUES('rebuild')")?;

    // Drop old manual index tables
    conn.execute("DROP TABLE IF EXISTS postings")?;
    conn.execute("DROP TABLE IF EXISTS index_terms")?;

    // Mark migration complete
    conn.execute(
        "INSERT OR REPLACE INTO app_meta (key, value) VALUES ('schema_version', '2')",
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::store::Store;

    #[test]
    fn test_all_tables_created() {
        let store = Store::open_in_memory().unwrap();
        let mut tables = Vec::new();
        let mut stmt = store
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        while let Ok(sqlite::State::Row) = stmt.next() {
            tables.push(stmt.read::<String, _>("name").unwrap());
        }

        assert!(tables.contains(&"chats".to_string()));
        assert!(tables.contains(&"messages".to_string()));
        assert!(tables.contains(&"sync_state".to_string()));
        assert!(tables.contains(&"app_meta".to_string()));
    }

    #[test]
    fn test_fts5_table_created() {
        let store = Store::open_in_memory().unwrap();
        let mut stmt = store
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name = 'messages_fts'")
            .unwrap();
        assert!(matches!(stmt.next(), Ok(sqlite::State::Row)));
    }

    #[test]
    fn test_old_index_tables_dropped() {
        let store = Store::open_in_memory().unwrap();
        let mut tables = Vec::new();
        let mut stmt = store
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        while let Ok(sqlite::State::Row) = stmt.next() {
            tables.push(stmt.read::<String, _>("name").unwrap());
        }

        assert!(!tables.contains(&"index_terms".to_string()));
        assert!(!tables.contains(&"postings".to_string()));
    }
}
