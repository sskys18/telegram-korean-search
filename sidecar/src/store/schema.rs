use sqlite::Connection;

pub fn run_migrations(conn: &Connection) -> Result<(), sqlite::Error> {
    // Phase 1: Create base tables (idempotent)
    conn.execute(
        "
        CREATE TABLE IF NOT EXISTS chats (
            chat_id       INTEGER PRIMARY KEY,
            title         TEXT NOT NULL,
            chat_type     TEXT NOT NULL CHECK (chat_type IN ('group', 'supergroup', 'channel', 'dm')),
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

    // Phase 3: Add 'dm' chat_type
    migrate_add_dm_chat_type(conn)?;

    // Phase 4: Wiki feature tables
    migrate_to_wiki_tables(conn)?;

    // Phase 5: Merge duplicate wiki categories
    migrate_merge_duplicate_categories(conn)?;

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
    conn.execute("INSERT OR REPLACE INTO app_meta (key, value) VALUES ('schema_version', '2')")?;

    Ok(())
}

fn migrate_add_dm_chat_type(conn: &Connection) -> Result<(), sqlite::Error> {
    if get_schema_version(conn) >= 3 {
        return Ok(());
    }

    // SQLite doesn't support ALTER CONSTRAINT, so recreate the table.
    // Temporarily disable foreign keys so we can drop the referenced table.
    // PRAGMA foreign_keys cannot be changed inside a transaction.
    conn.execute("PRAGMA foreign_keys = OFF")?;

    // Drop leftover temp table from any previously interrupted migration
    conn.execute("DROP TABLE IF EXISTS chats_new")?;
    conn.execute(
        "
        CREATE TABLE chats_new (
            chat_id       INTEGER PRIMARY KEY,
            title         TEXT NOT NULL,
            chat_type     TEXT NOT NULL CHECK (chat_type IN ('group', 'supergroup', 'channel', 'dm')),
            username      TEXT,
            access_hash   INTEGER,
            is_excluded   INTEGER NOT NULL DEFAULT 0,
            created_at    TEXT NOT NULL DEFAULT (datetime('now'))
        );

        INSERT INTO chats_new (chat_id, title, chat_type, username, access_hash, is_excluded, created_at)
            SELECT chat_id, title, chat_type, username, access_hash, is_excluded, created_at FROM chats;

        DROP TABLE chats;
        ALTER TABLE chats_new RENAME TO chats;
        ",
    )?;

    conn.execute("INSERT OR REPLACE INTO app_meta (key, value) VALUES ('schema_version', '3')")?;

    // Re-enable foreign keys
    conn.execute("PRAGMA foreign_keys = ON")?;

    Ok(())
}

fn migrate_to_wiki_tables(conn: &Connection) -> Result<(), sqlite::Error> {
    if get_schema_version(conn) >= 4 {
        return Ok(());
    }

    conn.execute(
        "
        CREATE TABLE IF NOT EXISTS wiki_categories (
            category_id  INTEGER PRIMARY KEY AUTOINCREMENT,
            name         TEXT NOT NULL UNIQUE,
            name_ko      TEXT,
            sort_order   INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS wiki_topics (
            topic_id        INTEGER PRIMARY KEY AUTOINCREMENT,
            title           TEXT NOT NULL UNIQUE,
            title_ko        TEXT,
            category_id     INTEGER REFERENCES wiki_categories(category_id),
            trending_score  REAL NOT NULL DEFAULT 0.0,
            message_count   INTEGER NOT NULL DEFAULT 0,
            channel_count   INTEGER NOT NULL DEFAULT 0,
            first_seen_at   INTEGER,
            last_seen_at    INTEGER,
            last_summary_at INTEGER,
            created_at      TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_wiki_topics_trending
            ON wiki_topics (trending_score DESC);
        CREATE INDEX IF NOT EXISTS idx_wiki_topics_category
            ON wiki_topics (category_id);
        CREATE INDEX IF NOT EXISTS idx_wiki_topics_last_seen
            ON wiki_topics (last_seen_at DESC);

        CREATE TABLE IF NOT EXISTS wiki_topic_aliases (
            alias_id   INTEGER PRIMARY KEY AUTOINCREMENT,
            topic_id   INTEGER NOT NULL REFERENCES wiki_topics(topic_id) ON DELETE CASCADE,
            alias      TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE INDEX IF NOT EXISTS idx_wiki_aliases_topic
            ON wiki_topic_aliases (topic_id);

        CREATE TABLE IF NOT EXISTS wiki_topic_messages (
            topic_id          INTEGER NOT NULL REFERENCES wiki_topics(topic_id) ON DELETE CASCADE,
            chat_id           INTEGER NOT NULL,
            message_id        INTEGER NOT NULL,
            relevance         REAL NOT NULL DEFAULT 1.0,
            assigned_category TEXT,
            PRIMARY KEY (topic_id, chat_id, message_id),
            FOREIGN KEY (chat_id, message_id) REFERENCES messages(chat_id, message_id)
        );

        CREATE INDEX IF NOT EXISTS idx_topic_messages_msg
            ON wiki_topic_messages (chat_id, message_id);

        CREATE TABLE IF NOT EXISTS wiki_pages (
            page_id      INTEGER PRIMARY KEY AUTOINCREMENT,
            topic_id     INTEGER NOT NULL REFERENCES wiki_topics(topic_id) ON DELETE CASCADE,
            content_ko   TEXT NOT NULL,
            content_en   TEXT NOT NULL,
            source_count INTEGER,
            source_hash  TEXT,
            version      INTEGER NOT NULL DEFAULT 1,
            created_at   TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE (topic_id, version)
        );

        CREATE INDEX IF NOT EXISTS idx_wiki_pages_topic
            ON wiki_pages (topic_id, version DESC);

        CREATE TABLE IF NOT EXISTS wiki_page_sources (
            page_id        INTEGER NOT NULL REFERENCES wiki_pages(page_id) ON DELETE CASCADE,
            citation_index INTEGER NOT NULL,
            chat_id        INTEGER NOT NULL,
            message_id     INTEGER NOT NULL,
            PRIMARY KEY (page_id, citation_index)
        );

        CREATE TABLE IF NOT EXISTS wiki_classify_queue (
            chat_id      INTEGER NOT NULL,
            message_id   INTEGER NOT NULL,
            status       TEXT NOT NULL DEFAULT 'pending'
                         CHECK (status IN ('pending', 'processing', 'done', 'failed', 'skipped')),
            attempts     INTEGER NOT NULL DEFAULT 0,
            error        TEXT,
            claimed_at   TEXT,
            created_at   TEXT NOT NULL DEFAULT (datetime('now')),
            processed_at TEXT,
            PRIMARY KEY (chat_id, message_id)
        );

        CREATE INDEX IF NOT EXISTS idx_queue_status
            ON wiki_classify_queue (status, created_at);

        CREATE TABLE IF NOT EXISTS topic_stats_daily (
            topic_id  INTEGER NOT NULL REFERENCES wiki_topics(topic_id) ON DELETE CASCADE,
            date      TEXT NOT NULL,
            msg_count INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (topic_id, date)
        );

        CREATE TABLE IF NOT EXISTS topic_channel_membership (
            topic_id INTEGER NOT NULL REFERENCES wiki_topics(topic_id) ON DELETE CASCADE,
            date     TEXT NOT NULL,
            chat_id  INTEGER NOT NULL,
            PRIMARY KEY (topic_id, date, chat_id)
        );
        ",
    )?;

    conn.execute(
        "CREATE VIRTUAL TABLE IF NOT EXISTS wiki_pages_fts USING fts5(
            content_ko, content_en,
            content='wiki_pages',
            tokenize='trigram case_sensitive 0'
        )",
    )?;

    // No seed categories — categories are auto-created by the LLM classifier.
    // Known aliases are handled in wiki_category.rs resolve_category().

    conn.execute("INSERT OR REPLACE INTO app_meta (key, value) VALUES ('schema_version', '4')")?;

    Ok(())
}

/// Migration v5: Merge duplicate wiki categories using the expanded KNOWN_ALIASES table.
/// For each alias group, picks the canonical category (lowest id) and reassigns all topics.
fn migrate_merge_duplicate_categories(conn: &Connection) -> Result<(), sqlite::Error> {
    if get_schema_version(conn) >= 5 {
        return Ok(());
    }

    // Collect all categories
    let mut stmt =
        conn.prepare("SELECT category_id, name FROM wiki_categories ORDER BY category_id")?;
    let mut categories: Vec<(i64, String)> = Vec::new();
    while let sqlite::State::Row = stmt.next()? {
        categories.push((stmt.read::<i64, _>(0)?, stmt.read::<String, _>(1)?));
    }
    drop(stmt);

    if categories.is_empty() {
        conn.execute(
            "INSERT OR REPLACE INTO app_meta (key, value) VALUES ('schema_version', '5')",
        )?;
        return Ok(());
    }

    // Build merge map: for each category, find its canonical target
    // Two categories merge if they share a KNOWN_ALIASES group OR have same lowercased name
    let mut merge_to: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    // canonical_name -> (lowest_category_id)
    let mut canonical_map: std::collections::HashMap<String, i64> =
        std::collections::HashMap::new();

    for &(cat_id, ref name) in &categories {
        let normalized = name.trim().to_lowercase();

        // Check known aliases for a canonical name
        let canonical = crate::store::wiki_category::find_canonical_name_pub(&normalized)
            .map(|s| s.to_lowercase())
            .unwrap_or_else(|| normalized.clone());

        if let Some(&existing_id) = canonical_map.get(&canonical) {
            if existing_id != cat_id {
                merge_to.insert(cat_id, existing_id);
            }
        } else {
            canonical_map.insert(canonical, cat_id);
        }
    }

    // Also merge exact case-insensitive duplicates that aren't in KNOWN_ALIASES
    let mut name_map: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for &(cat_id, ref name) in &categories {
        if merge_to.contains_key(&cat_id) {
            continue;
        }
        let key = name.trim().to_lowercase();
        if let Some(&existing_id) = name_map.get(&key) {
            if existing_id != cat_id {
                merge_to.insert(cat_id, existing_id);
            }
        } else {
            name_map.insert(key, cat_id);
        }
    }

    let merge_count = merge_to.len();
    conn.execute("BEGIN")?;

    if merge_count > 0 {
        log::info!(
            "Wiki migration v5: merging {} duplicate categories",
            merge_count
        );

        for (&from_id, &to_id) in &merge_to {
            conn.execute(format!(
                "UPDATE wiki_topics SET category_id = {} WHERE category_id = {}",
                to_id, from_id
            ))?;
            conn.execute(format!(
                "DELETE FROM wiki_categories WHERE category_id = {}",
                from_id
            ))?;
        }
    }

    conn.execute(
        "DELETE FROM wiki_categories WHERE category_id NOT IN (
            SELECT DISTINCT category_id FROM wiki_topics WHERE category_id IS NOT NULL
        )",
    )?;

    conn.execute("INSERT OR REPLACE INTO app_meta (key, value) VALUES ('schema_version', '5')")?;
    conn.execute("COMMIT")?;

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
    fn test_dm_chat_type_accepted() {
        let store = Store::open_in_memory().unwrap();
        store
            .conn()
            .execute(
                "INSERT INTO chats (chat_id, title, chat_type) VALUES (12345, 'John Doe', 'dm')",
            )
            .unwrap();
        let mut stmt = store
            .conn()
            .prepare("SELECT chat_type FROM chats WHERE chat_id = 12345")
            .unwrap();
        assert!(matches!(stmt.next(), Ok(sqlite::State::Row)));
        assert_eq!(stmt.read::<String, _>(0).unwrap(), "dm");
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

    #[test]
    fn test_wiki_tables_created() {
        let store = Store::open_in_memory().unwrap();
        let mut tables = Vec::new();
        let mut stmt = store
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        while let Ok(sqlite::State::Row) = stmt.next() {
            tables.push(stmt.read::<String, _>("name").unwrap());
        }

        assert!(tables.contains(&"wiki_categories".to_string()));
        assert!(tables.contains(&"wiki_topics".to_string()));
        assert!(tables.contains(&"wiki_topic_aliases".to_string()));
        assert!(tables.contains(&"wiki_topic_messages".to_string()));
        assert!(tables.contains(&"wiki_pages".to_string()));
        assert!(tables.contains(&"wiki_page_sources".to_string()));
        assert!(tables.contains(&"wiki_classify_queue".to_string()));
        assert!(tables.contains(&"topic_stats_daily".to_string()));
        assert!(tables.contains(&"topic_channel_membership".to_string()));
    }

    #[test]
    fn test_wiki_categories_table_empty() {
        let store = Store::open_in_memory().unwrap();
        let mut count = 0_i64;
        let mut stmt = store
            .conn()
            .prepare("SELECT COUNT(*) FROM wiki_categories")
            .unwrap();
        if let Ok(sqlite::State::Row) = stmt.next() {
            count = stmt.read::<i64, _>(0).unwrap();
        }
        // No seed categories — auto-created by LLM classifier
        assert_eq!(count, 0);
    }
}
