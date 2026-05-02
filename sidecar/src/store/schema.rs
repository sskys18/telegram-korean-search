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

    // Phase 6: Korean-aware auxiliary indexes (jamo, nospace)
    migrate_korean_indexes(conn)?;

    // Phase 7: Clean up the chosung column/table for installs that
    // already ran an earlier v6 (which created them). Fresh v6 in
    // this codebase no longer creates chosung in the first place.
    migrate_drop_chosung(conn)?;

    // Phase 8: Collapse Korean search indexes into one external-content
    // FTS5 table over plain, nospace, and jamo-normalized text.
    migrate_message_index_v8(conn)?;

    // Phase 9: Wiki v2 tables (per docs/specs/2026-04-24-reindex-and-wiki-v2-design.md §5).
    // v1 wiki tables stay untouched at this phase. v9 tables that would
    // collide with v1 names (wiki_pages, wiki_classify_queue) get a
    // `_v2` suffix; phase-13 drop_v1_wiki swaps them in. End state is
    // identical to spec rename-then-drop. This deviation keeps the v1
    // wiki module code (10 files) compiling until phase 6 rebuild.
    migrate_to_v9(conn)?;

    Ok(())
}

fn migrate_to_v9(conn: &Connection) -> Result<(), sqlite::Error> {
    // Fully idempotent: all statements use IF NOT EXISTS / column_exists
    // guards. Re-running is a no-op. This lets us extend v9 in place
    // (cloud-spec additions per docs/specs/2026-04-27-cloud-wiki-architecture.md
    // §Schema) without bumping schema_version.
    conn.execute("BEGIN")?;
    let result = (|| -> Result<(), sqlite::Error> {
        // Cloud-spec additions to messages: msg_version, deleted_at,
        // cloud_acked_version, sender_id. Land in the v9 banner per cloud
        // spec line 114. Behavior (msg_version bumps, soft-delete reads,
        // tombstone sweep) is deferred to the cloud worker phase.
        if !column_exists(conn, "messages", "msg_version")? {
            conn.execute("ALTER TABLE messages ADD COLUMN msg_version INTEGER NOT NULL DEFAULT 1")?;
        }
        if !column_exists(conn, "messages", "deleted_at")? {
            conn.execute("ALTER TABLE messages ADD COLUMN deleted_at INTEGER")?;
        }
        if !column_exists(conn, "messages", "cloud_acked_version")? {
            conn.execute("ALTER TABLE messages ADD COLUMN cloud_acked_version INTEGER")?;
        }
        if !column_exists(conn, "messages", "sender_id")? {
            conn.execute("ALTER TABLE messages ADD COLUMN sender_id INTEGER")?;
        }

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS cloud_outbox (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                client_op_id TEXT NOT NULL UNIQUE,
                op           TEXT NOT NULL CHECK (op IN
                                 ('msg_upsert','msg_delete','chat_meta','chat_purge')),
                chat_id      INTEGER NOT NULL,
                message_id   INTEGER,
                msg_version  INTEGER,
                payload      BLOB NOT NULL,
                created_at   INTEGER NOT NULL,
                attempts     INTEGER NOT NULL DEFAULT 0,
                last_error   TEXT
            );
            CREATE INDEX IF NOT EXISTS ix_outbox_chat ON cloud_outbox (chat_id);

            CREATE TABLE IF NOT EXISTS postbox_recon_watermark (
                chat_id        INTEGER PRIMARY KEY,
                max_msg_id     INTEGER NOT NULL,
                last_full_diff INTEGER NOT NULL DEFAULT 0
            );
            ",
        )?;

        conn.execute(
            "
            CREATE TABLE IF NOT EXISTS wiki_pages_v2 (
                id                          INTEGER PRIMARY KEY,
                kind                        TEXT NOT NULL
                                                CHECK (kind IN ('topic','event','entity')),
                title                       TEXT NOT NULL,
                title_norm                  TEXT NOT NULL,
                summary_md                  TEXT NOT NULL DEFAULT '',
                summary_rev                 INTEGER NOT NULL DEFAULT 0,
                state                       TEXT NOT NULL DEFAULT 'active'
                                                CHECK (state IN ('active','resolved','frozen','hidden')),
                pinned                      INTEGER NOT NULL DEFAULT 0,
                facts                       TEXT,
                facts_version               INTEGER NOT NULL DEFAULT 1,
                evidence_count              INTEGER NOT NULL DEFAULT 0,
                last_rewrite_evidence_count INTEGER NOT NULL DEFAULT 0,
                last_rewrite_max_evidence_id INTEGER NOT NULL DEFAULT 0,
                last_evidence_at            INTEGER,
                last_rewrite_at             INTEGER,
                created_at                  INTEGER NOT NULL,
                updated_at                  INTEGER NOT NULL
            );

            CREATE UNIQUE INDEX IF NOT EXISTS ux_pages_v2_title_norm
                ON wiki_pages_v2 (title_norm);
            CREATE INDEX IF NOT EXISTS ix_pages_v2_active_evidence
                ON wiki_pages_v2 (state, last_evidence_at DESC)
                WHERE state = 'active';
            CREATE INDEX IF NOT EXISTS ix_pages_v2_kind_state
                ON wiki_pages_v2 (kind, state);

            CREATE TABLE IF NOT EXISTS wiki_page_aliases (
                page_id    INTEGER NOT NULL
                               REFERENCES wiki_pages_v2(id) ON DELETE CASCADE,
                alias_norm TEXT NOT NULL,
                alias_raw  TEXT NOT NULL,
                PRIMARY KEY (page_id, alias_norm)
            );
            CREATE INDEX IF NOT EXISTS ix_aliases_norm
                ON wiki_page_aliases (alias_norm);

            CREATE TABLE IF NOT EXISTS wiki_evidence (
                id           INTEGER PRIMARY KEY,
                page_id      INTEGER NOT NULL
                                 REFERENCES wiki_pages_v2(id) ON DELETE CASCADE,
                msg_id       INTEGER NOT NULL,
                chat_id      INTEGER NOT NULL,
                sender_id    INTEGER NOT NULL,
                ts           INTEGER NOT NULL,
                excerpt      TEXT NOT NULL,
                excerpt_jamo TEXT NOT NULL DEFAULT '',
                source_hash  BLOB NOT NULL,
                salience     REAL NOT NULL DEFAULT 0.5,
                cited        INTEGER NOT NULL DEFAULT 0,
                created_at   INTEGER NOT NULL,
                UNIQUE (page_id, msg_id, chat_id)
            );
            CREATE INDEX IF NOT EXISTS ix_evidence_source_hash
                ON wiki_evidence (source_hash);
            CREATE INDEX IF NOT EXISTS ix_evidence_page_ts
                ON wiki_evidence (page_id, ts DESC);
            CREATE INDEX IF NOT EXISTS ix_evidence_chat_ts
                ON wiki_evidence (chat_id, ts DESC);
            CREATE INDEX IF NOT EXISTS ix_evidence_ts
                ON wiki_evidence (ts DESC);
            CREATE INDEX IF NOT EXISTS ix_evidence_msg
                ON wiki_evidence (msg_id, chat_id);

            CREATE TABLE IF NOT EXISTS wiki_classify_queue_v2 (
                msg_id          INTEGER NOT NULL,
                chat_id         INTEGER NOT NULL,
                status          TEXT NOT NULL DEFAULT 'pending'
                                    CHECK (status IN ('pending','processing','failed','done')),
                attempts        INTEGER NOT NULL DEFAULT 0,
                last_error      TEXT,
                hint            TEXT,
                hint_page_id    INTEGER REFERENCES wiki_pages_v2(id) ON DELETE SET NULL,
                text_hash       BLOB NOT NULL,
                enqueued_at     INTEGER NOT NULL,
                claimed_at      INTEGER,
                next_attempt_at INTEGER,
                PRIMARY KEY (msg_id, chat_id)
            );
            CREATE INDEX IF NOT EXISTS ix_classify_v2_ready
                ON wiki_classify_queue_v2 (status, next_attempt_at)
                WHERE status = 'pending';

            CREATE TABLE IF NOT EXISTS wiki_rewrite_queue (
                page_id         INTEGER PRIMARY KEY
                                    REFERENCES wiki_pages_v2(id) ON DELETE CASCADE,
                status          TEXT NOT NULL DEFAULT 'pending'
                                    CHECK (status IN ('pending','processing','failed','done')),
                attempts        INTEGER NOT NULL DEFAULT 0,
                last_error      TEXT,
                enqueued_at     INTEGER NOT NULL,
                claimed_at      INTEGER,
                next_attempt_at INTEGER
            );

            CREATE TABLE IF NOT EXISTS trending_cache (
                window         TEXT NOT NULL CHECK (window IN ('1h','24h','7d')),
                page_id        INTEGER NOT NULL
                                   REFERENCES wiki_pages_v2(id) ON DELETE CASCADE,
                rank           INTEGER NOT NULL,
                hook           TEXT NOT NULL,
                reason_code    TEXT NOT NULL,
                reason_metrics TEXT NOT NULL,
                sparkline      TEXT NOT NULL,
                computed_at    INTEGER NOT NULL,
                PRIMARY KEY (window, page_id),
                UNIQUE (window, rank)
            );
            CREATE INDEX IF NOT EXISTS ix_trending_window_rank
                ON trending_cache (window, rank);

            CREATE TABLE IF NOT EXISTS trending_watermark (
                window           TEXT PRIMARY KEY CHECK (window IN ('1h','24h','7d')),
                last_evidence_id INTEGER NOT NULL DEFAULT 0,
                last_computed_at INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS ask_history (
                id            INTEGER PRIMARY KEY,
                query         TEXT NOT NULL,
                answer_md     TEXT NOT NULL,
                cited_sources TEXT NOT NULL,
                model         TEXT NOT NULL,
                status        TEXT NOT NULL
                                  CHECK (status IN ('streaming','done','cancelled','failed')),
                created_at    INTEGER NOT NULL,
                finished_at   INTEGER
            );

            CREATE TABLE IF NOT EXISTS wiki_settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS wiki_last_open (
                chat_id      INTEGER PRIMARY KEY,
                last_open_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS wiki_pages_index (
                page_id      INTEGER PRIMARY KEY
                                 REFERENCES wiki_pages_v2(id) ON DELETE CASCADE,
                title        TEXT NOT NULL,
                aliases      TEXT NOT NULL DEFAULT '',
                summary_md   TEXT NOT NULL DEFAULT '',
                title_jamo   TEXT NOT NULL DEFAULT '',
                aliases_jamo TEXT NOT NULL DEFAULT '',
                summary_jamo TEXT NOT NULL DEFAULT ''
            );
            ",
        )?;

        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(
                title, aliases, summary_md,
                title_jamo, aliases_jamo, summary_jamo,
                content='wiki_pages_index',
                content_rowid='page_id',
                tokenize='trigram case_sensitive 0'
            )",
        )?;

        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS evidence_fts USING fts5(
                excerpt, excerpt_jamo,
                content='wiki_evidence',
                content_rowid='id',
                tokenize='trigram case_sensitive 0'
            )",
        )?;

        // Phase 7 add: monotonic id watermark for rewrite delta
        // selection. Time-based watermark loses same-second insertions
        // when classify lands new evidence after the rewrite's select
        // snapshot but before its apply. Idempotent for existing v9 DBs.
        if !column_exists(conn, "wiki_pages_v2", "last_rewrite_max_evidence_id")? {
            conn.execute(
                "ALTER TABLE wiki_pages_v2
                    ADD COLUMN last_rewrite_max_evidence_id INTEGER NOT NULL DEFAULT 0",
            )?;
        }

        seed_wiki_settings(conn)?;

        conn.execute(
            "INSERT OR REPLACE INTO app_meta (key, value) VALUES ('schema_version', '9')",
        )?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute("COMMIT")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute("ROLLBACK");
            Err(e)
        }
    }
}

fn seed_wiki_settings(conn: &Connection) -> Result<(), sqlite::Error> {
    const DEFAULTS: &[(&str, &str)] = &[
        ("max_codex_calls_per_hour_total", "500"),
        ("model_classify", "gpt-5.5-nano"),
        ("model_rewrite", "gpt-5.5"),
        ("model_trending", "gpt-5.5"),
        ("model_ask", "gpt-5.5-fast"),
        ("classify_batch_size", "20"),
        ("rewrite_per_hour_cap", "30"),
        ("trend_refresh_min_interval_sec", "300"),
        ("trend_window_min_refresh_sec", "3600"),
        ("min_classify_chars", "12"),
        ("max_classify_attempts", "3"),
        ("max_rewrite_attempts", "3"),
        ("max_ask_attempts", "2"),
        ("retention_evidence_per_page", "200"),
        ("fuzzy_title_dedup", "0"),
        ("pause_codex", "0"),
        ("pause_on_low_battery", "1"),
        ("low_battery_threshold_percent", "20"),
        ("v2_backfill_complete", "0"),
        ("v2_backfill_allow_failed_tolerance", "100"),
        ("schema_v9_marker", "1"),
    ];
    for (key, value) in DEFAULTS {
        let mut stmt =
            conn.prepare("INSERT OR IGNORE INTO wiki_settings (key, value) VALUES (?, ?)")?;
        stmt.bind((1, *key))?;
        stmt.bind((2, *value))?;
        stmt.next()?;
    }
    Ok(())
}

fn migrate_message_index_v8(conn: &Connection) -> Result<(), sqlite::Error> {
    if get_schema_version(conn) >= 8 {
        return Ok(());
    }

    if !column_exists(conn, "messages", "text_jamo")? {
        conn.execute("ALTER TABLE messages ADD COLUMN text_jamo TEXT NOT NULL DEFAULT ''")?;
    }

    backfill_message_search_columns(conn)?;

    conn.execute("DROP TABLE IF EXISTS messages_fts")?;
    conn.execute("DROP TABLE IF EXISTS messages_fts_nospace")?;
    conn.execute("DROP TABLE IF EXISTS messages_fts_jamo")?;

    conn.execute(
        "CREATE VIRTUAL TABLE messages_fts USING fts5(
            text_plain, text_stripped, text_jamo,
            content='messages',
            content_rowid='rowid',
            tokenize='trigram case_sensitive 0'
        )",
    )?;

    conn.execute("INSERT INTO messages_fts(messages_fts) VALUES('rebuild')")?;
    conn.execute("INSERT OR REPLACE INTO app_meta (key, value) VALUES ('schema_version', '8')")?;

    Ok(())
}

fn migrate_drop_chosung(conn: &Connection) -> Result<(), sqlite::Error> {
    if get_schema_version(conn) >= 7 {
        return Ok(());
    }

    conn.execute("DROP TABLE IF EXISTS messages_fts_chosung")?;

    if column_exists(conn, "messages", "text_chosung")? {
        // DROP COLUMN requires SQLite 3.35+. sqlcipher ships 3.46.1.
        conn.execute("ALTER TABLE messages DROP COLUMN text_chosung")?;
    }

    conn.execute("INSERT OR REPLACE INTO app_meta (key, value) VALUES ('schema_version', '7')")?;

    Ok(())
}

fn migrate_korean_indexes(conn: &Connection) -> Result<(), sqlite::Error> {
    if get_schema_version(conn) >= 6 {
        return Ok(());
    }

    // Add two computed-at-insert columns. text_stripped already
    // existed; we just start indexing it now. NOT NULL + DEFAULT ''
    // keeps old INSERTs that do not mention these columns working.
    if !column_exists(conn, "messages", "text_jamo")? {
        conn.execute("ALTER TABLE messages ADD COLUMN text_jamo TEXT NOT NULL DEFAULT ''")?;
    }

    // Backfill the new column from the existing text_plain in batches
    // of 500 so we never hold a huge prepared statement in memory.
    // Skip rows that already have a value (migration replay after a
    // crash).
    backfill_korean_columns(conn)?;

    conn.execute(
        "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts_jamo USING fts5(
            text_jamo,
            content='messages',
            tokenize='trigram case_sensitive 0'
        )",
    )?;
    conn.execute(
        "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts_nospace USING fts5(
            text_stripped,
            content='messages',
            tokenize='trigram case_sensitive 0'
        )",
    )?;

    // External-content FTS5 reads the column from `messages` on
    // rebuild, so the backfilled values are what gets indexed.
    conn.execute("INSERT INTO messages_fts_jamo(messages_fts_jamo) VALUES('rebuild')")?;
    conn.execute("INSERT INTO messages_fts_nospace(messages_fts_nospace) VALUES('rebuild')")?;

    conn.execute("INSERT OR REPLACE INTO app_meta (key, value) VALUES ('schema_version', '6')")?;

    Ok(())
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool, sqlite::Error> {
    let mut stmt = conn.prepare(format!("PRAGMA table_info({table})"))?;
    while let sqlite::State::Row = stmt.next()? {
        if stmt.read::<String, _>("name")? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn backfill_korean_columns(conn: &Connection) -> Result<(), sqlite::Error> {
    const BATCH: usize = 5000;
    loop {
        let mut rows: Vec<(i64, String)> = Vec::with_capacity(BATCH);
        {
            let mut stmt = conn.prepare(
                "SELECT rowid, text_plain FROM messages
                 WHERE text_jamo = ''
                 LIMIT ?",
            )?;
            stmt.bind((1, BATCH as i64))?;
            while let sqlite::State::Row = stmt.next()? {
                rows.push((stmt.read::<i64, _>(0)?, stmt.read::<String, _>(1)?));
            }
        }
        if rows.is_empty() {
            return Ok(());
        }

        conn.execute("BEGIN")?;
        for (rowid, text) in &rows {
            let jamo = crate::search::hangul::decompose_jamo(text);
            let mut stmt = conn.prepare("UPDATE messages SET text_jamo = ? WHERE rowid = ?")?;
            stmt.bind((1, jamo.as_str()))?;
            stmt.bind((2, *rowid))?;
            stmt.next()?;
        }
        conn.execute("COMMIT")?;

        if rows.len() < BATCH {
            return Ok(());
        }
    }
}

fn backfill_message_search_columns(conn: &Connection) -> Result<(), sqlite::Error> {
    const BATCH: usize = 5000;
    loop {
        let mut rows: Vec<(i64, String)> = Vec::with_capacity(BATCH);
        {
            let mut stmt = conn.prepare(
                "SELECT rowid, text_plain FROM messages
                 WHERE text_stripped = '' OR text_jamo = ''
                 LIMIT ?",
            )?;
            stmt.bind((1, BATCH as i64))?;
            while let sqlite::State::Row = stmt.next()? {
                rows.push((stmt.read::<i64, _>(0)?, stmt.read::<String, _>(1)?));
            }
        }
        if rows.is_empty() {
            return Ok(());
        }

        conn.execute("BEGIN")?;
        for (rowid, text) in &rows {
            let stripped = crate::store::message::strip_whitespace(text);
            let jamo = crate::search::hangul::decompose_jamo(text);
            let mut stmt = conn
                .prepare("UPDATE messages SET text_stripped = ?, text_jamo = ? WHERE rowid = ?")?;
            stmt.bind((1, stripped.as_str()))?;
            stmt.bind((2, jamo.as_str()))?;
            stmt.bind((3, *rowid))?;
            stmt.next()?;
        }
        conn.execute("COMMIT")?;

        if rows.len() < BATCH {
            return Ok(());
        }
    }
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
    fn test_v8_single_message_fts_table_created() {
        let store = Store::open_in_memory().unwrap();
        let mut tables = Vec::new();
        let mut stmt = store
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        while let Ok(sqlite::State::Row) = stmt.next() {
            tables.push(stmt.read::<String, _>("name").unwrap());
        }

        assert!(tables.contains(&"messages_fts".to_string()));
        assert!(!tables.contains(&"messages_fts_nospace".to_string()));
        assert!(!tables.contains(&"messages_fts_jamo".to_string()));
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
    fn test_v9_tables_created() {
        let store = Store::open_in_memory().unwrap();
        let mut tables = Vec::new();
        let mut stmt = store
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','view') ORDER BY name")
            .unwrap();
        while let Ok(sqlite::State::Row) = stmt.next() {
            tables.push(stmt.read::<String, _>("name").unwrap());
        }

        for name in [
            "wiki_pages_v2",
            "wiki_page_aliases",
            "wiki_evidence",
            "wiki_classify_queue_v2",
            "wiki_rewrite_queue",
            "trending_cache",
            "trending_watermark",
            "ask_history",
            "wiki_settings",
            "wiki_last_open",
            "wiki_pages_index",
            "pages_fts",
            "evidence_fts",
        ] {
            assert!(tables.contains(&name.to_string()), "missing table: {name}");
        }
    }

    #[test]
    fn test_schema_version_is_9() {
        let store = Store::open_in_memory().unwrap();
        let mut stmt = store
            .conn()
            .prepare("SELECT value FROM app_meta WHERE key = 'schema_version'")
            .unwrap();
        assert!(matches!(stmt.next(), Ok(sqlite::State::Row)));
        assert_eq!(stmt.read::<String, _>(0).unwrap(), "9");
    }

    #[test]
    fn test_v9_settings_seeded() {
        let store = Store::open_in_memory().unwrap();
        let mut stmt = store
            .conn()
            .prepare("SELECT value FROM wiki_settings WHERE key = 'schema_v9_marker'")
            .unwrap();
        assert!(matches!(stmt.next(), Ok(sqlite::State::Row)));
        assert_eq!(stmt.read::<String, _>(0).unwrap(), "1");

        let mut count_stmt = store
            .conn()
            .prepare("SELECT COUNT(*) FROM wiki_settings")
            .unwrap();
        count_stmt.next().unwrap();
        let count: i64 = count_stmt.read(0).unwrap();
        assert_eq!(count, 21);
    }

    #[test]
    fn test_v9_message_columns_added() {
        let store = Store::open_in_memory().unwrap();
        for col in [
            "msg_version",
            "deleted_at",
            "cloud_acked_version",
            "sender_id",
        ] {
            let mut stmt = store
                .conn()
                .prepare("SELECT 1 FROM pragma_table_info('messages') WHERE name = ?")
                .unwrap();
            stmt.bind((1, col)).unwrap();
            assert!(
                matches!(stmt.next(), Ok(sqlite::State::Row)),
                "column missing: {col}"
            );
        }
    }

    #[test]
    fn test_v9_cloud_tables_created() {
        let store = Store::open_in_memory().unwrap();
        let mut tables = Vec::new();
        let mut stmt = store
            .conn()
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        while let Ok(sqlite::State::Row) = stmt.next() {
            tables.push(stmt.read::<String, _>("name").unwrap());
        }
        assert!(tables.contains(&"cloud_outbox".to_string()));
        assert!(tables.contains(&"postbox_recon_watermark".to_string()));
    }

    #[test]
    fn test_v9_idempotent() {
        let store = Store::open_in_memory().unwrap();
        // Re-running migrations is a no-op.
        super::run_migrations(store.conn()).unwrap();
        super::run_migrations(store.conn()).unwrap();

        let mut stmt = store
            .conn()
            .prepare("SELECT value FROM app_meta WHERE key = 'schema_version'")
            .unwrap();
        assert!(matches!(stmt.next(), Ok(sqlite::State::Row)));
        assert_eq!(stmt.read::<String, _>(0).unwrap(), "9");
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
