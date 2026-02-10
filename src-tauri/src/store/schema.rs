use rusqlite::Connection;

pub fn run_migrations(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
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

        CREATE TABLE IF NOT EXISTS index_terms (
            term_id       INTEGER PRIMARY KEY AUTOINCREMENT,
            term          TEXT NOT NULL UNIQUE,
            source_type   TEXT NOT NULL CHECK (source_type IN ('token', 'ngram', 'stripped_ngram'))
        );

        CREATE INDEX IF NOT EXISTS idx_terms_term
            ON index_terms (term);

        CREATE TABLE IF NOT EXISTS postings (
            term_id       INTEGER NOT NULL,
            chat_id       INTEGER NOT NULL,
            message_id    INTEGER NOT NULL,
            timestamp     INTEGER NOT NULL,
            PRIMARY KEY (term_id, timestamp DESC, chat_id, message_id),
            FOREIGN KEY (term_id) REFERENCES index_terms(term_id),
            FOREIGN KEY (chat_id, message_id) REFERENCES messages(chat_id, message_id)
        );

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
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::store::Store;

    #[test]
    fn test_all_tables_created() {
        let store = Store::open_in_memory().unwrap();
        let tables: Vec<String> = store
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"chats".to_string()));
        assert!(tables.contains(&"messages".to_string()));
        assert!(tables.contains(&"index_terms".to_string()));
        assert!(tables.contains(&"postings".to_string()));
        assert!(tables.contains(&"sync_state".to_string()));
        assert!(tables.contains(&"app_meta".to_string()));
    }
}
