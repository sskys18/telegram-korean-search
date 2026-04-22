# Wiki Feature Implementation Plan

**Spec**: `docs/specs/2026-04-06-wiki-feature-design.md` (v3, Codex-approved)
**Scope**: Full wiki feature — data layer, LLM client, background worker, Tauri commands, React frontend
**Approach**: 4 phases, each produces testable software independently

## File Map

### New Files (Rust Backend)
| File | Responsibility |
|------|---------------|
| `src-tauri/src/store/wiki_category.rs` | Category CRUD + normalization |
| `src-tauri/src/store/wiki_queue.rs` | Classify queue (enqueue, atomic dequeue, crash recovery) |
| `src-tauri/src/store/wiki_topic.rs` | Topic + alias + topic_messages CRUD, reconciliation |
| `src-tauri/src/store/wiki_page.rs` | Wiki pages + page_sources + FTS5 |
| `src-tauri/src/store/wiki_stats.rs` | Daily rollups + channel membership + trending queries |
| `src-tauri/src/wiki/mod.rs` | Module root |
| `src-tauri/src/wiki/llm.rs` | OpenAI HTTP client |
| `src-tauri/src/wiki/worker.rs` | Background classification worker |
| `src-tauri/src/wiki/trending.rs` | Trending score calculation |

### New Files (Frontend)
| File | Responsibility |
|------|---------------|
| `src/components/TabBar.tsx` | Search/Wiki tab switcher |
| `src/components/wiki/TrendingDashboard.tsx` | Landing page with topic list |
| `src/components/wiki/TopicCard.tsx` | Individual topic card |
| `src/components/wiki/WikiArticle.tsx` | Bilingual article with citations |
| `src/components/wiki/SourceMessages.tsx` | Collapsible source messages |
| `src/components/wiki/CategoryFilter.tsx` | Category pill selector |
| `src/components/wiki/WikiSearch.tsx` | Wiki search bar + results |
| `src/components/wiki/WikiSettings.tsx` | API key, status, controls |
| `src/hooks/useWiki.ts` | Topic browsing, selection, categories |
| `src/hooks/useWikiWorker.ts` | Worker status, progress, controls |
| `src/pages/WikiPage.tsx` | Main wiki container |

### Modified Files
| File | Change |
|------|--------|
| `src-tauri/src/store/schema.rs` | Add `migrate_to_wiki_tables()` (v4) |
| `src-tauri/src/store/mod.rs` | Register 5 new store modules |
| `src-tauri/src/lib.rs` | Add `wiki` module, register new Tauri commands, add wiki worker handle to AppState |
| `src-tauri/src/commands.rs` | Add wiki commands, enqueue in collection flow |
| `src-tauri/Cargo.toml` | Add `reqwest`, `sha2` |
| `src/App.tsx` | Add TabBar, WikiPage routing |
| `src/App.css` | Wiki styles |
| `src/api/tauri.ts` | Wiki command wrappers + event listeners |
| `src/types/index.ts` | Wiki types |
| `package.json` | Add `react-markdown` |

---

## Phase 1: Data Layer

### Task 1.1: Add dependencies to Cargo.toml

**File**: `src-tauri/Cargo.toml`

Add after the `flexi_logger` line:
```toml
reqwest = { version = "0.12", features = ["json"] }
sha2 = "0.10"
```

**Verify**: `cd src-tauri && cargo check`

---

### Task 1.2: Schema migration v4 — wiki tables

**File**: `src-tauri/src/store/schema.rs`

Add call in `run_migrations()` after `migrate_add_dm_chat_type(conn)?;`:
```rust
    // Phase 4: Wiki feature tables
    migrate_to_wiki_tables(conn)?;
```

Add the migration function:
```rust
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
        "
    )?;

    // Create FTS5 virtual table for wiki pages
    conn.execute(
        "CREATE VIRTUAL TABLE IF NOT EXISTS wiki_pages_fts USING fts5(
            content_ko, content_en,
            content='wiki_pages',
            tokenize='trigram case_sensitive 0'
        )"
    )?;

    // Seed categories
    conn.execute(
        "
        INSERT OR IGNORE INTO wiki_categories (name, name_ko, sort_order) VALUES
            ('DeFi', '디파이', 1),
            ('Trading', '트레이딩', 2),
            ('L1/L2', '레이어1/2', 3),
            ('NFT', 'NFT', 4),
            ('Airdrop', '에어드롭', 5),
            ('Regulation', '규제', 6),
            ('Macro', '매크로', 7),
            ('Scam Alert', '스캠 경고', 8),
            ('Other', '기타', 99);
        "
    )?;

    conn.execute("INSERT OR REPLACE INTO app_meta (key, value) VALUES ('schema_version', '4')")?;

    Ok(())
}
```

Add test in the `#[cfg(test)]` block:
```rust
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
    fn test_wiki_seed_categories() {
        let store = Store::open_in_memory().unwrap();
        let mut count = 0i64;
        let mut stmt = store
            .conn()
            .prepare("SELECT COUNT(*) FROM wiki_categories")
            .unwrap();
        if let Ok(sqlite::State::Row) = stmt.next() {
            count = stmt.read::<i64, _>(0).unwrap();
        }
        assert_eq!(count, 9); // 9 seed categories
    }
```

**Verify**: `cargo test -p telegram-korean-search`

---

### Task 1.3: Register new store modules

**File**: `src-tauri/src/store/mod.rs`

Add after `pub mod sync_state;`:
```rust
pub mod wiki_category;
pub mod wiki_page;
pub mod wiki_queue;
pub mod wiki_stats;
pub mod wiki_topic;
```

**Verify**: Create empty files first so it compiles, then fill them in subsequent tasks.

---

### Task 1.4: Store — wiki_category.rs

**File**: `src-tauri/src/store/wiki_category.rs`

```rust
use serde::{Deserialize, Serialize};

use super::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiCategory {
    pub category_id: i64,
    pub name: String,
    pub name_ko: Option<String>,
    pub sort_order: i64,
}

impl Store {
    pub fn get_all_categories(&self) -> Result<Vec<WikiCategory>, sqlite::Error> {
        let mut stmt = self
            .conn()
            .prepare("SELECT category_id, name, name_ko, sort_order FROM wiki_categories ORDER BY sort_order")?;
        let mut cats = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            cats.push(WikiCategory {
                category_id: stmt.read::<i64, _>("category_id")?,
                name: stmt.read::<String, _>("name")?,
                name_ko: stmt.read::<Option<String>, _>("name_ko")?,
                sort_order: stmt.read::<i64, _>("sort_order")?,
            });
        }
        Ok(cats)
    }

    pub fn get_category_by_id(&self, category_id: i64) -> Result<Option<WikiCategory>, sqlite::Error> {
        let mut stmt = self
            .conn()
            .prepare("SELECT category_id, name, name_ko, sort_order FROM wiki_categories WHERE category_id = ?")?;
        stmt.bind((1, category_id))?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(Some(WikiCategory {
                category_id: stmt.read::<i64, _>("category_id")?,
                name: stmt.read::<String, _>("name")?,
                name_ko: stmt.read::<Option<String>, _>("name_ko")?,
                sort_order: stmt.read::<i64, _>("sort_order")?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Normalize a free-form category name from LLM output to a canonical category_id.
    /// Returns the "Other" category if no match found.
    pub fn normalize_category(&self, raw: &str) -> Result<i64, sqlite::Error> {
        let normalized = raw.trim().to_lowercase();

        // Try exact match (case-insensitive)
        let mut stmt = self
            .conn()
            .prepare("SELECT category_id FROM wiki_categories WHERE LOWER(name) = ?")?;
        stmt.bind((1, normalized.as_str()))?;
        if let sqlite::State::Row = stmt.next()? {
            return Ok(stmt.read::<i64, _>(0)?);
        }

        // Try contains match
        let like_pattern = format!("%{}%", normalized);
        let mut stmt = self
            .conn()
            .prepare("SELECT category_id FROM wiki_categories WHERE LOWER(name) LIKE ? LIMIT 1")?;
        stmt.bind((1, like_pattern.as_str()))?;
        if let sqlite::State::Row = stmt.next()? {
            return Ok(stmt.read::<i64, _>(0)?);
        }

        // Fallback to "Other"
        let mut stmt = self
            .conn()
            .prepare("SELECT category_id FROM wiki_categories WHERE name = 'Other'")?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(stmt.read::<i64, _>(0)?)
        } else {
            // Should never happen if seeds ran, but be safe
            Ok(9)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::store::Store;

    #[test]
    fn test_get_all_categories() {
        let store = Store::open_in_memory().unwrap();
        let cats = store.get_all_categories().unwrap();
        assert_eq!(cats.len(), 9);
        assert_eq!(cats[0].name, "DeFi");
        assert_eq!(cats[0].name_ko, Some("디파이".to_string()));
    }

    #[test]
    fn test_normalize_category_exact() {
        let store = Store::open_in_memory().unwrap();
        let id = store.normalize_category("DeFi").unwrap();
        let cat = store.get_category_by_id(id).unwrap().unwrap();
        assert_eq!(cat.name, "DeFi");
    }

    #[test]
    fn test_normalize_category_case_insensitive() {
        let store = Store::open_in_memory().unwrap();
        let id = store.normalize_category("defi").unwrap();
        let cat = store.get_category_by_id(id).unwrap().unwrap();
        assert_eq!(cat.name, "DeFi");
    }

    #[test]
    fn test_normalize_category_fallback() {
        let store = Store::open_in_memory().unwrap();
        let id = store.normalize_category("something unknown").unwrap();
        let cat = store.get_category_by_id(id).unwrap().unwrap();
        assert_eq!(cat.name, "Other");
    }
}
```

**Verify**: `cargo test -p telegram-korean-search wiki_category`

---

### Task 1.5: Store — wiki_queue.rs

**File**: `src-tauri/src/store/wiki_queue.rs`

```rust
use serde::{Deserialize, Serialize};

use super::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub chat_id: i64,
    pub message_id: i64,
    pub status: String,
    pub attempts: i64,
    pub error: Option<String>,
    pub claimed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueStats {
    pub pending: i64,
    pub processing: i64,
    pub done: i64,
    pub failed: i64,
    pub skipped: i64,
}

impl Store {
    /// Enqueue message IDs for classification. Ignores duplicates.
    pub fn enqueue_for_classification(&self, items: &[(i64, i64)]) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "INSERT OR IGNORE INTO wiki_classify_queue (chat_id, message_id) VALUES (?, ?)",
        )?;
        for &(chat_id, message_id) in items {
            stmt.bind((1, chat_id))?;
            stmt.bind((2, message_id))?;
            stmt.next()?;
            stmt.reset()?;
        }
        Ok(())
    }

    /// Atomically claim up to `limit` pending items. Returns claimed items.
    pub fn dequeue_classify_batch(&self, limit: usize) -> Result<Vec<QueueItem>, sqlite::Error> {
        // Claim items atomically
        self.conn().execute(&format!(
            "UPDATE wiki_classify_queue
             SET status = 'processing', claimed_at = datetime('now'), attempts = attempts + 1
             WHERE rowid IN (
                 SELECT rowid FROM wiki_classify_queue
                 WHERE status = 'pending'
                 ORDER BY created_at
                 LIMIT {}
             )",
            limit
        ))?;

        // Read claimed items
        let mut stmt = self.conn().prepare(
            "SELECT chat_id, message_id, status, attempts, error, claimed_at
             FROM wiki_classify_queue WHERE status = 'processing'"
        )?;
        let mut items = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            items.push(QueueItem {
                chat_id: stmt.read::<i64, _>("chat_id")?,
                message_id: stmt.read::<i64, _>("message_id")?,
                status: stmt.read::<String, _>("status")?,
                attempts: stmt.read::<i64, _>("attempts")?,
                error: stmt.read::<Option<String>, _>("error")?,
                claimed_at: stmt.read::<Option<String>, _>("claimed_at")?,
            });
        }
        Ok(items)
    }

    /// Mark a queue item as done.
    pub fn mark_queue_done(&self, chat_id: i64, message_id: i64) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "UPDATE wiki_classify_queue SET status = 'done', processed_at = datetime('now')
             WHERE chat_id = ? AND message_id = ?",
        )?;
        stmt.bind((1, chat_id))?;
        stmt.bind((2, message_id))?;
        stmt.next()?;
        Ok(())
    }

    /// Mark a queue item as skipped (LLM said skip).
    pub fn mark_queue_skipped(&self, chat_id: i64, message_id: i64) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "UPDATE wiki_classify_queue SET status = 'skipped', processed_at = datetime('now')
             WHERE chat_id = ? AND message_id = ?",
        )?;
        stmt.bind((1, chat_id))?;
        stmt.bind((2, message_id))?;
        stmt.next()?;
        Ok(())
    }

    /// Mark a queue item as failed with error message.
    /// If attempts >= 3, keeps as failed. Otherwise resets to pending for retry.
    pub fn mark_queue_failed(
        &self,
        chat_id: i64,
        message_id: i64,
        error: &str,
    ) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "UPDATE wiki_classify_queue
             SET status = CASE WHEN attempts >= 3 THEN 'failed' ELSE 'pending' END,
                 error = ?,
                 processed_at = datetime('now')
             WHERE chat_id = ? AND message_id = ?",
        )?;
        stmt.bind((1, error))?;
        stmt.bind((2, chat_id))?;
        stmt.bind((3, message_id))?;
        stmt.next()?;
        Ok(())
    }

    /// Recover stale claims (items stuck in 'processing' for > 5 minutes).
    pub fn recover_stale_claims(&self) -> Result<usize, sqlite::Error> {
        self.conn().execute(
            "UPDATE wiki_classify_queue
             SET status = 'pending', claimed_at = NULL
             WHERE status = 'processing'
               AND claimed_at < datetime('now', '-5 minutes')",
        )?;
        Ok(self.conn().change_count())
    }

    /// Get queue statistics.
    pub fn get_queue_stats(&self) -> Result<QueueStats, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT
                SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END) as pending,
                SUM(CASE WHEN status = 'processing' THEN 1 ELSE 0 END) as processing,
                SUM(CASE WHEN status = 'done' THEN 1 ELSE 0 END) as done,
                SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) as failed,
                SUM(CASE WHEN status = 'skipped' THEN 1 ELSE 0 END) as skipped
             FROM wiki_classify_queue",
        )?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(QueueStats {
                pending: stmt.read::<Option<i64>, _>("pending")?.unwrap_or(0),
                processing: stmt.read::<Option<i64>, _>("processing")?.unwrap_or(0),
                done: stmt.read::<Option<i64>, _>("done")?.unwrap_or(0),
                failed: stmt.read::<Option<i64>, _>("failed")?.unwrap_or(0),
                skipped: stmt.read::<Option<i64>, _>("skipped")?.unwrap_or(0),
            })
        } else {
            Ok(QueueStats {
                pending: 0,
                processing: 0,
                done: 0,
                failed: 0,
                skipped: 0,
            })
        }
    }

    /// Clear all queue data (for reprocessing).
    pub fn clear_classify_queue(&self) -> Result<(), sqlite::Error> {
        self.conn().execute("DELETE FROM wiki_classify_queue")?;
        Ok(())
    }

    /// Re-enqueue all messages for classification.
    pub fn enqueue_all_messages(&self) -> Result<usize, sqlite::Error> {
        self.conn().execute(
            "INSERT OR IGNORE INTO wiki_classify_queue (chat_id, message_id)
             SELECT chat_id, message_id FROM messages",
        )?;
        Ok(self.conn().change_count())
    }
}

#[cfg(test)]
mod tests {
    use crate::store::Store;
    use crate::store::message::MessageRow;

    fn setup_store_with_messages() -> Store {
        let store = Store::open_in_memory().unwrap();
        // Insert a chat first
        store.conn().execute(
            "INSERT INTO chats (chat_id, title, chat_type) VALUES (1, 'Test', 'channel')"
        ).unwrap();
        // Insert messages
        let msgs = vec![
            MessageRow {
                message_id: 1, chat_id: 1, timestamp: 1000,
                text_plain: "hello".to_string(), text_stripped: "hello".to_string(), link: None,
            },
            MessageRow {
                message_id: 2, chat_id: 1, timestamp: 2000,
                text_plain: "world".to_string(), text_stripped: "world".to_string(), link: None,
            },
        ];
        store.insert_messages_batch(&msgs).unwrap();
        store
    }

    #[test]
    fn test_enqueue_and_dequeue() {
        let store = setup_store_with_messages();
        store.enqueue_for_classification(&[(1, 1), (1, 2)]).unwrap();

        let stats = store.get_queue_stats().unwrap();
        assert_eq!(stats.pending, 2);

        let batch = store.dequeue_classify_batch(1).unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].status, "processing");
    }

    #[test]
    fn test_enqueue_ignores_duplicates() {
        let store = setup_store_with_messages();
        store.enqueue_for_classification(&[(1, 1)]).unwrap();
        store.enqueue_for_classification(&[(1, 1)]).unwrap();
        let stats = store.get_queue_stats().unwrap();
        assert_eq!(stats.pending, 1);
    }

    #[test]
    fn test_mark_done() {
        let store = setup_store_with_messages();
        store.enqueue_for_classification(&[(1, 1)]).unwrap();
        store.dequeue_classify_batch(1).unwrap();
        store.mark_queue_done(1, 1).unwrap();
        let stats = store.get_queue_stats().unwrap();
        assert_eq!(stats.done, 1);
        assert_eq!(stats.pending, 0);
    }

    #[test]
    fn test_mark_failed_retries() {
        let store = setup_store_with_messages();
        store.enqueue_for_classification(&[(1, 1)]).unwrap();
        // First attempt
        store.dequeue_classify_batch(1).unwrap();
        store.mark_queue_failed(1, 1, "timeout").unwrap();
        let stats = store.get_queue_stats().unwrap();
        // attempts=1, < 3, so back to pending
        assert_eq!(stats.pending, 1);
    }

    #[test]
    fn test_enqueue_all_messages() {
        let store = setup_store_with_messages();
        let count = store.enqueue_all_messages().unwrap();
        assert_eq!(count, 2);
        let stats = store.get_queue_stats().unwrap();
        assert_eq!(stats.pending, 2);
    }
}
```

**Verify**: `cargo test -p telegram-korean-search wiki_queue`

---

### Task 1.6: Store — wiki_topic.rs

**File**: `src-tauri/src/store/wiki_topic.rs`

```rust
use serde::{Deserialize, Serialize};

use super::Store;
use super::message::MessageWithChat;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiTopic {
    pub topic_id: i64,
    pub title: String,
    pub title_ko: Option<String>,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub category_name_ko: Option<String>,
    pub trending_score: f64,
    pub message_count: i64,
    pub channel_count: i64,
    pub first_seen_at: Option<i64>,
    pub last_seen_at: Option<i64>,
    pub last_summary_at: Option<i64>,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewTopic {
    pub title: String,
    pub title_ko: Option<String>,
    pub category_id: i64,
}

#[derive(Debug, Clone)]
pub struct TopicMessageLink {
    pub topic_id: i64,
    pub chat_id: i64,
    pub message_id: i64,
    pub relevance: f64,
    pub assigned_category: String,
}

impl Store {
    /// Create a new topic and its first alias. Returns topic_id.
    pub fn create_topic(&self, topic: &NewTopic) -> Result<i64, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "INSERT INTO wiki_topics (title, title_ko, category_id) VALUES (?, ?, ?)",
        )?;
        stmt.bind((1, topic.title.as_str()))?;
        stmt.bind((2, topic.title_ko.as_deref()))?;
        stmt.bind((3, topic.category_id))?;
        stmt.next()?;

        let topic_id = self.last_insert_rowid();

        // Add the title as the first alias (normalized)
        let alias = normalize_topic_title(&topic.title);
        self.add_topic_alias(topic_id, &alias)?;

        Ok(topic_id)
    }

    fn last_insert_rowid(&self) -> i64 {
        let mut stmt = self.conn().prepare("SELECT last_insert_rowid()").unwrap();
        stmt.next().unwrap();
        stmt.read::<i64, _>(0).unwrap()
    }

    /// Look up a topic by normalized alias. Returns topic_id if found.
    pub fn find_topic_by_alias(&self, raw_title: &str) -> Result<Option<i64>, sqlite::Error> {
        let normalized = normalize_topic_title(raw_title);
        let mut stmt = self
            .conn()
            .prepare("SELECT topic_id FROM wiki_topic_aliases WHERE alias = ?")?;
        stmt.bind((1, normalized.as_str()))?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(Some(stmt.read::<i64, _>(0)?))
        } else {
            Ok(None)
        }
    }

    /// Get top-N similar aliases for dedup checking.
    pub fn get_similar_aliases(&self, raw_title: &str, limit: usize) -> Result<Vec<(i64, String)>, sqlite::Error> {
        let normalized = normalize_topic_title(raw_title);
        // Use LIKE with first 3 chars as a rough filter
        let prefix = if normalized.len() >= 3 {
            &normalized[..3]
        } else {
            &normalized
        };
        let pattern = format!("{}%", prefix);
        let mut stmt = self.conn().prepare(
            &format!(
                "SELECT topic_id, alias FROM wiki_topic_aliases WHERE alias LIKE ? LIMIT {}",
                limit
            ),
        )?;
        stmt.bind((1, pattern.as_str()))?;
        let mut results = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            results.push((
                stmt.read::<i64, _>("topic_id")?,
                stmt.read::<String, _>("alias")?,
            ));
        }
        Ok(results)
    }

    /// Add an alias for a topic. Ignores if alias already exists.
    pub fn add_topic_alias(&self, topic_id: i64, alias: &str) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "INSERT OR IGNORE INTO wiki_topic_aliases (topic_id, alias) VALUES (?, ?)",
        )?;
        stmt.bind((1, topic_id))?;
        stmt.bind((2, alias))?;
        stmt.next()?;
        Ok(())
    }

    /// Link a message to a topic.
    pub fn link_message_to_topic(&self, link: &TopicMessageLink) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "INSERT OR IGNORE INTO wiki_topic_messages
             (topic_id, chat_id, message_id, relevance, assigned_category)
             VALUES (?, ?, ?, ?, ?)",
        )?;
        stmt.bind((1, link.topic_id))?;
        stmt.bind((2, link.chat_id))?;
        stmt.bind((3, link.message_id))?;
        stmt.bind((4, link.relevance))?;
        stmt.bind((5, link.assigned_category.as_str()))?;
        stmt.next()?;

        // Update topic counters
        self.refresh_topic_counters(link.topic_id)?;

        Ok(())
    }

    /// Refresh message_count, channel_count, first/last_seen_at for a topic.
    fn refresh_topic_counters(&self, topic_id: i64) -> Result<(), sqlite::Error> {
        self.conn().execute(&format!(
            "UPDATE wiki_topics SET
                message_count = (SELECT COUNT(*) FROM wiki_topic_messages WHERE topic_id = {0}),
                channel_count = (SELECT COUNT(DISTINCT chat_id) FROM wiki_topic_messages WHERE topic_id = {0}),
                first_seen_at = (SELECT MIN(m.timestamp) FROM wiki_topic_messages tm
                    JOIN messages m ON m.chat_id = tm.chat_id AND m.message_id = tm.message_id
                    WHERE tm.topic_id = {0}),
                last_seen_at = (SELECT MAX(m.timestamp) FROM wiki_topic_messages tm
                    JOIN messages m ON m.chat_id = tm.chat_id AND m.message_id = tm.message_id
                    WHERE tm.topic_id = {0}),
                updated_at = datetime('now')
             WHERE topic_id = {0}",
            topic_id
        ))?;
        Ok(())
    }

    /// Set Korean title if not already set (first-non-null-wins).
    pub fn set_title_ko_if_absent(&self, topic_id: i64, title_ko: &str) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "UPDATE wiki_topics SET title_ko = ? WHERE topic_id = ? AND title_ko IS NULL",
        )?;
        stmt.bind((1, title_ko))?;
        stmt.bind((2, topic_id))?;
        stmt.next()?;
        Ok(())
    }

    /// Check if category should be updated based on majority vote.
    /// Returns Some(new_category_id) if update needed, None otherwise.
    pub fn check_category_reconciliation(&self, topic_id: i64) -> Result<Option<i64>, sqlite::Error> {
        // Only reconcile if topic has > 10 messages
        let mut stmt = self.conn().prepare(
            "SELECT message_count, category_id FROM wiki_topics WHERE topic_id = ?"
        )?;
        stmt.bind((1, topic_id))?;
        if let sqlite::State::Row = stmt.next()? {
            let count = stmt.read::<i64, _>("message_count")?;
            let current_cat = stmt.read::<Option<i64>, _>("category_id")?;
            if count <= 10 {
                return Ok(None);
            }

            // Find the most common assigned_category
            let mut stmt2 = self.conn().prepare(
                "SELECT assigned_category, COUNT(*) as cnt
                 FROM wiki_topic_messages WHERE topic_id = ?
                 GROUP BY assigned_category ORDER BY cnt DESC LIMIT 1"
            )?;
            stmt2.bind((1, topic_id))?;
            if let sqlite::State::Row = stmt2.next()? {
                let top_cat = stmt2.read::<String, _>("assigned_category")?;
                let top_cnt = stmt2.read::<i64, _>("cnt")?;
                let ratio = top_cnt as f64 / count as f64;

                if ratio > 0.6 {
                    let new_id = self.normalize_category(&top_cat)?;
                    if current_cat != Some(new_id) {
                        return Ok(Some(new_id));
                    }
                }
            }
            Ok(None)
        } else {
            Ok(None)
        }
    }

    /// Update topic category.
    pub fn update_topic_category(&self, topic_id: i64, category_id: i64) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "UPDATE wiki_topics SET category_id = ?, updated_at = datetime('now') WHERE topic_id = ?"
        )?;
        stmt.bind((1, category_id))?;
        stmt.bind((2, topic_id))?;
        stmt.next()?;
        Ok(())
    }

    /// Update trending score.
    pub fn update_trending_score(&self, topic_id: i64, score: f64) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "UPDATE wiki_topics SET trending_score = ? WHERE topic_id = ?"
        )?;
        stmt.bind((1, score))?;
        stmt.bind((2, topic_id))?;
        stmt.next()?;
        Ok(())
    }

    /// Get trending topics with category info.
    pub fn get_trending_topics(
        &self,
        limit: usize,
        offset: usize,
        category_id: Option<i64>,
    ) -> Result<Vec<WikiTopic>, sqlite::Error> {
        let sql = match category_id {
            Some(_) => format!(
                "SELECT t.*, c.name as category_name, c.name_ko as category_name_ko
                 FROM wiki_topics t
                 LEFT JOIN wiki_categories c ON t.category_id = c.category_id
                 WHERE t.category_id = ?
                 ORDER BY t.trending_score DESC
                 LIMIT {} OFFSET {}",
                limit, offset
            ),
            None => format!(
                "SELECT t.*, c.name as category_name, c.name_ko as category_name_ko
                 FROM wiki_topics t
                 LEFT JOIN wiki_categories c ON t.category_id = c.category_id
                 ORDER BY t.trending_score DESC
                 LIMIT {} OFFSET {}",
                limit, offset
            ),
        };
        let mut stmt = self.conn().prepare(&sql)?;
        if let Some(cat_id) = category_id {
            stmt.bind((1, cat_id))?;
        }
        let mut topics = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            topics.push(read_wiki_topic(&stmt)?);
        }
        Ok(topics)
    }

    /// Get a single topic with category info.
    pub fn get_topic(&self, topic_id: i64) -> Result<Option<WikiTopic>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT t.*, c.name as category_name, c.name_ko as category_name_ko
             FROM wiki_topics t
             LEFT JOIN wiki_categories c ON t.category_id = c.category_id
             WHERE t.topic_id = ?"
        )?;
        stmt.bind((1, topic_id))?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(Some(read_wiki_topic(&stmt)?))
        } else {
            Ok(None)
        }
    }

    /// Get source messages for a topic, ordered by relevance.
    pub fn get_topic_sources(
        &self,
        topic_id: i64,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MessageWithChat>, sqlite::Error> {
        let mut stmt = self.conn().prepare(&format!(
            "SELECT m.message_id, m.chat_id, m.timestamp, m.text_plain, m.link, ch.title as chat_title
             FROM wiki_topic_messages tm
             JOIN messages m ON m.chat_id = tm.chat_id AND m.message_id = tm.message_id
             JOIN chats ch ON ch.chat_id = m.chat_id
             WHERE tm.topic_id = ?
             ORDER BY tm.relevance DESC, m.timestamp DESC
             LIMIT {} OFFSET {}",
            limit, offset
        ))?;
        stmt.bind((1, topic_id))?;
        let mut msgs = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            msgs.push(MessageWithChat {
                message_id: stmt.read::<i64, _>("message_id")?,
                chat_id: stmt.read::<i64, _>("chat_id")?,
                timestamp: stmt.read::<i64, _>("timestamp")?,
                text_plain: stmt.read::<String, _>("text_plain")?,
                link: stmt.read::<Option<String>, _>("link")?,
                chat_title: stmt.read::<String, _>("chat_title")?,
            });
        }
        Ok(msgs)
    }

    /// Get all topic IDs that have new messages since their last summary.
    pub fn get_topics_needing_summary(&self) -> Result<Vec<i64>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT topic_id FROM wiki_topics
             WHERE last_summary_at IS NULL
                OR last_seen_at > last_summary_at"
        )?;
        let mut ids = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            ids.push(stmt.read::<i64, _>(0)?);
        }
        Ok(ids)
    }

    /// Search topics by title.
    pub fn search_topics(&self, query: &str, limit: usize) -> Result<Vec<WikiTopic>, sqlite::Error> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn().prepare(&format!(
            "SELECT t.*, c.name as category_name, c.name_ko as category_name_ko
             FROM wiki_topics t
             LEFT JOIN wiki_categories c ON t.category_id = c.category_id
             WHERE t.title LIKE ? OR t.title_ko LIKE ?
             ORDER BY t.trending_score DESC
             LIMIT {}",
            limit
        ))?;
        stmt.bind((1, pattern.as_str()))?;
        stmt.bind((2, pattern.as_str()))?;
        let mut topics = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            topics.push(read_wiki_topic(&stmt)?);
        }
        Ok(topics)
    }

    /// Clear all wiki topic data (for full reset).
    pub fn clear_wiki_topics(&self) -> Result<(), sqlite::Error> {
        self.conn().execute("DELETE FROM wiki_topic_messages")?;
        self.conn().execute("DELETE FROM wiki_topic_aliases")?;
        self.conn().execute("DELETE FROM wiki_topics")?;
        Ok(())
    }
}

fn read_wiki_topic(stmt: &sqlite::Statement) -> Result<WikiTopic, sqlite::Error> {
    Ok(WikiTopic {
        topic_id: stmt.read::<i64, _>("topic_id")?,
        title: stmt.read::<String, _>("title")?,
        title_ko: stmt.read::<Option<String>, _>("title_ko")?,
        category_id: stmt.read::<Option<i64>, _>("category_id")?,
        category_name: stmt.read::<Option<String>, _>("category_name")?,
        category_name_ko: stmt.read::<Option<String>, _>("category_name_ko")?,
        trending_score: stmt.read::<f64, _>("trending_score")?,
        message_count: stmt.read::<i64, _>("message_count")?,
        channel_count: stmt.read::<i64, _>("channel_count")?,
        first_seen_at: stmt.read::<Option<i64>, _>("first_seen_at")?,
        last_seen_at: stmt.read::<Option<i64>, _>("last_seen_at")?,
        last_summary_at: stmt.read::<Option<i64>, _>("last_summary_at")?,
        updated_at: stmt.read::<String, _>("updated_at")?,
    })
}

/// Normalize a topic title for alias matching.
/// Lowercase, strip whitespace, remove common suffixes.
pub fn normalize_topic_title(title: &str) -> String {
    let mut s = title.to_lowercase();
    s = s.chars().filter(|c| !c.is_whitespace()).collect();
    // Remove common noise suffixes
    for suffix in &["update", "news", "alert", "analysis"] {
        if s.ends_with(suffix) && s.len() > suffix.len() + 3 {
            s = s[..s.len() - suffix.len()].to_string();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn test_normalize_topic_title() {
        assert_eq!(normalize_topic_title("ETH Layer 2 Fees"), "ethlayer2fees");
        assert_eq!(normalize_topic_title("Bitcoin Price Update"), "bitcoinprice");
        assert_eq!(normalize_topic_title("BTC"), "btc"); // too short for suffix strip
    }

    #[test]
    fn test_create_and_find_topic() {
        let store = Store::open_in_memory().unwrap();
        let topic = NewTopic {
            title: "ETH Layer 2 Fees".to_string(),
            title_ko: Some("이더리움 L2 수수료".to_string()),
            category_id: 1,
        };
        let id = store.create_topic(&topic).unwrap();
        assert!(id > 0);

        let found = store.find_topic_by_alias("ETH Layer 2 Fees").unwrap();
        assert_eq!(found, Some(id));

        // Should also match with different casing/spacing
        let found2 = store.find_topic_by_alias("eth layer 2 fees").unwrap();
        assert_eq!(found2, Some(id));
    }

    #[test]
    fn test_get_topic_with_category() {
        let store = Store::open_in_memory().unwrap();
        let topic = NewTopic {
            title: "DeFi Test".to_string(),
            title_ko: None,
            category_id: 1, // DeFi
        };
        let id = store.create_topic(&topic).unwrap();
        let loaded = store.get_topic(id).unwrap().unwrap();
        assert_eq!(loaded.title, "DeFi Test");
        assert_eq!(loaded.category_name, Some("DeFi".to_string()));
    }
}
```

**Verify**: `cargo test -p telegram-korean-search wiki_topic`

---

### Task 1.7: Store — wiki_page.rs

**File**: `src-tauri/src/store/wiki_page.rs`

```rust
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPage {
    pub page_id: i64,
    pub topic_id: i64,
    pub content_ko: String,
    pub content_en: String,
    pub source_count: Option<i64>,
    pub source_hash: Option<String>,
    pub version: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSource {
    pub citation_index: i64,
    pub chat_id: i64,
    pub message_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPageSearchResult {
    pub topic_id: i64,
    pub topic_title: String,
    pub snippet: String,
}

impl Store {
    /// Insert a new wiki page version with its citation sources.
    pub fn insert_wiki_page(
        &self,
        topic_id: i64,
        content_ko: &str,
        content_en: &str,
        sources: &[(i64, i64)], // (chat_id, message_id) in citation order
    ) -> Result<i64, sqlite::Error> {
        // Calculate next version
        let version = {
            let mut stmt = self.conn().prepare(
                "SELECT COALESCE(MAX(version), 0) + 1 FROM wiki_pages WHERE topic_id = ?"
            )?;
            stmt.bind((1, topic_id))?;
            stmt.next()?;
            stmt.read::<i64, _>(0)?
        };

        let source_hash = compute_source_hash(sources);

        let mut stmt = self.conn().prepare(
            "INSERT INTO wiki_pages (topic_id, content_ko, content_en, source_count, source_hash, version)
             VALUES (?, ?, ?, ?, ?, ?)"
        )?;
        stmt.bind((1, topic_id))?;
        stmt.bind((2, content_ko))?;
        stmt.bind((3, content_en))?;
        stmt.bind((4, sources.len() as i64))?;
        stmt.bind((5, source_hash.as_str()))?;
        stmt.bind((6, version))?;
        stmt.next()?;

        let page_id = self.last_insert_rowid();

        // Insert FTS5 entry
        let mut fts_stmt = self.conn().prepare(
            "INSERT INTO wiki_pages_fts (rowid, content_ko, content_en) VALUES (?, ?, ?)"
        )?;
        fts_stmt.bind((1, page_id))?;
        fts_stmt.bind((2, content_ko))?;
        fts_stmt.bind((3, content_en))?;
        fts_stmt.next()?;

        // Insert citation sources
        let mut src_stmt = self.conn().prepare(
            "INSERT INTO wiki_page_sources (page_id, citation_index, chat_id, message_id)
             VALUES (?, ?, ?, ?)"
        )?;
        for (i, &(chat_id, message_id)) in sources.iter().enumerate() {
            src_stmt.bind((1, page_id))?;
            src_stmt.bind((2, (i + 1) as i64))?; // 1-indexed citations
            src_stmt.bind((3, chat_id))?;
            src_stmt.bind((4, message_id))?;
            src_stmt.next()?;
            src_stmt.reset()?;
        }

        // Update topic's last_summary_at
        self.conn().execute(&format!(
            "UPDATE wiki_topics SET last_summary_at = strftime('%s', 'now') WHERE topic_id = {}",
            topic_id
        ))?;

        Ok(page_id)
    }

    /// Get the latest wiki page for a topic.
    pub fn get_latest_page(&self, topic_id: i64) -> Result<Option<WikiPage>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT page_id, topic_id, content_ko, content_en, source_count, source_hash, version, created_at
             FROM wiki_pages WHERE topic_id = ? ORDER BY version DESC LIMIT 1"
        )?;
        stmt.bind((1, topic_id))?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(Some(WikiPage {
                page_id: stmt.read::<i64, _>("page_id")?,
                topic_id: stmt.read::<i64, _>("topic_id")?,
                content_ko: stmt.read::<String, _>("content_ko")?,
                content_en: stmt.read::<String, _>("content_en")?,
                source_count: stmt.read::<Option<i64>, _>("source_count")?,
                source_hash: stmt.read::<Option<String>, _>("source_hash")?,
                version: stmt.read::<i64, _>("version")?,
                created_at: stmt.read::<String, _>("created_at")?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Get citation sources for a page.
    pub fn get_page_sources(&self, page_id: i64) -> Result<Vec<PageSource>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT citation_index, chat_id, message_id FROM wiki_page_sources
             WHERE page_id = ? ORDER BY citation_index"
        )?;
        stmt.bind((1, page_id))?;
        let mut sources = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            sources.push(PageSource {
                citation_index: stmt.read::<i64, _>("citation_index")?,
                chat_id: stmt.read::<i64, _>("chat_id")?,
                message_id: stmt.read::<i64, _>("message_id")?,
            });
        }
        Ok(sources)
    }

    /// Check if a topic's source set has changed since last summary.
    /// Returns true if regeneration is needed.
    pub fn needs_regeneration(&self, topic_id: i64) -> Result<bool, sqlite::Error> {
        let page = self.get_latest_page(topic_id)?;
        match page {
            None => Ok(true), // No page exists yet
            Some(p) => {
                // Compute current source hash
                let mut stmt = self.conn().prepare(
                    "SELECT chat_id, message_id FROM wiki_topic_messages
                     WHERE topic_id = ? ORDER BY chat_id, message_id"
                )?;
                stmt.bind((1, topic_id))?;
                let mut sources = Vec::new();
                while let sqlite::State::Row = stmt.next()? {
                    sources.push((
                        stmt.read::<i64, _>("chat_id")?,
                        stmt.read::<i64, _>("message_id")?,
                    ));
                }
                let current_hash = compute_source_hash(&sources);
                Ok(p.source_hash.as_deref() != Some(current_hash.as_str()))
            }
        }
    }

    /// Search wiki pages via FTS5.
    pub fn search_wiki_pages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WikiPageSearchResult>, sqlite::Error> {
        if query.len() < 3 {
            return Ok(Vec::new()); // Trigram needs >= 3 chars
        }
        let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
        let mut stmt = self.conn().prepare(&format!(
            "SELECT wp.topic_id, wt.title, snippet(wiki_pages_fts, 0, '<b>', '</b>', '...', 32) as snippet
             FROM wiki_pages_fts fts
             JOIN wiki_pages wp ON wp.page_id = fts.rowid
             JOIN wiki_topics wt ON wt.topic_id = wp.topic_id
             WHERE wiki_pages_fts MATCH ?
             GROUP BY wp.topic_id
             LIMIT {}",
            limit
        ))?;
        stmt.bind((1, fts_query.as_str()))?;
        let mut results = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            results.push(WikiPageSearchResult {
                topic_id: stmt.read::<i64, _>("topic_id")?,
                topic_title: stmt.read::<String, _>("title")?,
                snippet: stmt.read::<String, _>("snippet")?,
            });
        }
        Ok(results)
    }

    /// Clear all wiki pages (for full reset).
    pub fn clear_wiki_pages(&self) -> Result<(), sqlite::Error> {
        self.conn().execute("DELETE FROM wiki_page_sources")?;
        self.conn().execute("DELETE FROM wiki_pages")?;
        self.conn().execute("INSERT INTO wiki_pages_fts(wiki_pages_fts) VALUES('rebuild')")?;
        Ok(())
    }
}

/// Compute a deterministic hash of source message IDs for cache invalidation.
pub fn compute_source_hash(sources: &[(i64, i64)]) -> String {
    let mut hasher = Sha256::new();
    for &(chat_id, message_id) in sources {
        hasher.update(chat_id.to_le_bytes());
        hasher.update(message_id.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use crate::store::Store;
    use crate::store::message::MessageRow;

    fn setup() -> Store {
        let store = Store::open_in_memory().unwrap();
        store.conn().execute(
            "INSERT INTO chats (chat_id, title, chat_type) VALUES (1, 'Test', 'channel')"
        ).unwrap();
        store.insert_messages_batch(&[
            MessageRow {
                message_id: 1, chat_id: 1, timestamp: 1000,
                text_plain: "test msg 1".to_string(), text_stripped: "testmsg1".to_string(), link: None,
            },
            MessageRow {
                message_id: 2, chat_id: 1, timestamp: 2000,
                text_plain: "test msg 2".to_string(), text_stripped: "testmsg2".to_string(), link: None,
            },
        ]).unwrap();
        store
    }

    #[test]
    fn test_insert_and_get_page() {
        let store = setup();
        let topic = crate::store::wiki_topic::NewTopic {
            title: "Test Topic".to_string(),
            title_ko: None,
            category_id: 1,
        };
        let topic_id = store.create_topic(&topic).unwrap();

        let page_id = store.insert_wiki_page(
            topic_id, "한국어 내용", "English content", &[(1, 1), (1, 2)]
        ).unwrap();
        assert!(page_id > 0);

        let page = store.get_latest_page(topic_id).unwrap().unwrap();
        assert_eq!(page.content_ko, "한국어 내용");
        assert_eq!(page.content_en, "English content");
        assert_eq!(page.version, 1);
        assert_eq!(page.source_count, Some(2));

        let sources = store.get_page_sources(page_id).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].citation_index, 1);
        assert_eq!(sources[1].citation_index, 2);
    }

    #[test]
    fn test_needs_regeneration() {
        let store = setup();
        let topic = crate::store::wiki_topic::NewTopic {
            title: "Regen Test".to_string(),
            title_ko: None,
            category_id: 1,
        };
        let topic_id = store.create_topic(&topic).unwrap();

        // No page exists
        assert!(store.needs_regeneration(topic_id).unwrap());

        // Link messages and create page
        let link = crate::store::wiki_topic::TopicMessageLink {
            topic_id, chat_id: 1, message_id: 1, relevance: 1.0,
            assigned_category: "DeFi".to_string(),
        };
        store.link_message_to_topic(&link).unwrap();
        store.insert_wiki_page(topic_id, "ko", "en", &[(1, 1)]).unwrap();

        // Same sources — no regen needed
        assert!(!store.needs_regeneration(topic_id).unwrap());

        // Add new message link — now needs regen
        let link2 = crate::store::wiki_topic::TopicMessageLink {
            topic_id, chat_id: 1, message_id: 2, relevance: 0.8,
            assigned_category: "DeFi".to_string(),
        };
        store.link_message_to_topic(&link2).unwrap();
        assert!(store.needs_regeneration(topic_id).unwrap());
    }
}
```

**Verify**: `cargo test -p telegram-korean-search wiki_page`

---

### Task 1.8: Store — wiki_stats.rs

**File**: `src-tauri/src/store/wiki_stats.rs`

```rust
use super::Store;

impl Store {
    /// Record a message classification in the daily rollup.
    /// Uses the message's timestamp (not current time) for the date.
    pub fn record_topic_stat(&self, topic_id: i64, message_timestamp: i64, chat_id: i64) -> Result<(), sqlite::Error> {
        // UPSERT into topic_stats_daily
        let mut stmt = self.conn().prepare(
            "INSERT INTO topic_stats_daily (topic_id, date, msg_count)
             VALUES (?, date(?, 'unixepoch'), 1)
             ON CONFLICT(topic_id, date) DO UPDATE SET msg_count = msg_count + 1"
        )?;
        stmt.bind((1, topic_id))?;
        stmt.bind((2, message_timestamp))?;
        stmt.next()?;

        // Record channel membership
        let mut stmt2 = self.conn().prepare(
            "INSERT OR IGNORE INTO topic_channel_membership (topic_id, date, chat_id)
             VALUES (?, date(?, 'unixepoch'), ?)"
        )?;
        stmt2.bind((1, topic_id))?;
        stmt2.bind((2, message_timestamp))?;
        stmt2.bind((3, chat_id))?;
        stmt2.next()?;

        Ok(())
    }

    /// Get message count for a topic in the last N days.
    pub fn get_topic_msg_count_days(&self, topic_id: i64, days: i64) -> Result<i64, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT COALESCE(SUM(msg_count), 0) FROM topic_stats_daily
             WHERE topic_id = ? AND date >= date('now', ? || ' days')"
        )?;
        stmt.bind((1, topic_id))?;
        stmt.bind((2, format!("-{}", days).as_str()))?;
        stmt.next()?;
        Ok(stmt.read::<i64, _>(0)?)
    }

    /// Get unique channel count for a topic in the last N days.
    pub fn get_topic_channel_count_days(&self, topic_id: i64, days: i64) -> Result<i64, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT COUNT(DISTINCT chat_id) FROM topic_channel_membership
             WHERE topic_id = ? AND date >= date('now', ? || ' days')"
        )?;
        stmt.bind((1, topic_id))?;
        stmt.bind((2, format!("-{}", days).as_str()))?;
        stmt.next()?;
        Ok(stmt.read::<i64, _>(0)?)
    }

    /// Get total active (non-excluded) channel count.
    pub fn get_total_active_channels(&self) -> Result<i64, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT COUNT(*) FROM chats WHERE is_excluded = 0"
        )?;
        stmt.next()?;
        Ok(stmt.read::<i64, _>(0)?)
    }

    /// Get all topic IDs that have activity in the last N days.
    pub fn get_active_topic_ids(&self, days: i64) -> Result<Vec<i64>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT DISTINCT topic_id FROM topic_stats_daily
             WHERE date >= date('now', ? || ' days')"
        )?;
        stmt.bind((1, format!("-{}", days).as_str()))?;
        let mut ids = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            ids.push(stmt.read::<i64, _>(0)?);
        }
        Ok(ids)
    }

    /// Clear all stats (for full reset).
    pub fn clear_wiki_stats(&self) -> Result<(), sqlite::Error> {
        self.conn().execute("DELETE FROM topic_stats_daily")?;
        self.conn().execute("DELETE FROM topic_channel_membership")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::store::Store;

    #[test]
    fn test_record_and_query_stats() {
        let store = Store::open_in_memory().unwrap();
        // Create a topic
        store.conn().execute(
            "INSERT INTO wiki_topics (title, category_id) VALUES ('Test', 1)"
        ).unwrap();
        let topic_id = 1;

        // Record stats with a recent timestamp
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        store.record_topic_stat(topic_id, now, 100).unwrap();
        store.record_topic_stat(topic_id, now, 200).unwrap(); // different channel
        store.record_topic_stat(topic_id, now, 100).unwrap(); // same channel again

        let msg_count = store.get_topic_msg_count_days(topic_id, 1).unwrap();
        assert_eq!(msg_count, 3);

        let chan_count = store.get_topic_channel_count_days(topic_id, 1).unwrap();
        assert_eq!(chan_count, 2); // 100 and 200
    }

    #[test]
    fn test_total_active_channels() {
        let store = Store::open_in_memory().unwrap();
        store.conn().execute(
            "INSERT INTO chats (chat_id, title, chat_type) VALUES (1, 'A', 'channel')"
        ).unwrap();
        store.conn().execute(
            "INSERT INTO chats (chat_id, title, chat_type, is_excluded) VALUES (2, 'B', 'channel', 1)"
        ).unwrap();
        let count = store.get_total_active_channels().unwrap();
        assert_eq!(count, 1);
    }
}
```

**Verify**: `cargo test -p telegram-korean-search wiki_stats`

---

### Task 1.9: Phase 1 verification

Run full test suite and checks:

```bash
cd src-tauri
cargo fmt
cargo clippy -- -D warnings
cargo test
```

All tests must pass. All new tables created. All CRUD operations working.

**Commit**: `git add -A && git commit -m "feat: wiki data layer — schema v4, store CRUD for topics, pages, queue, stats, categories"`

---

## Phase 2: LLM Client & Worker

### Task 2.1: Wiki module skeleton

**File**: `src-tauri/src/wiki/mod.rs`
```rust
pub mod llm;
pub mod trending;
pub mod worker;
```

**File**: `src-tauri/src/lib.rs` — add after `pub mod store;`:
```rust
pub mod wiki;
```

---

### Task 2.2: LLM client

**File**: `src-tauri/src/wiki/llm.rs`

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    api_key: String,
    model: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    response_format: ResponseFormat,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    r#type: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChatMessage,
}

// Classification response from LLM
#[derive(Debug, Clone, Deserialize)]
pub struct ClassifyResponse {
    pub skip: bool,
    #[serde(default)]
    pub topics: Vec<ClassifiedTopic>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClassifiedTopic {
    pub topic: String,
    pub topic_ko: Option<String>,
    pub category: String,
    pub relevance: f64,
}

// Dedup response
#[derive(Debug, Deserialize)]
pub struct DedupResponse {
    pub same: bool,
    pub confidence: f64,
}

#[derive(Debug)]
pub enum LlmError {
    Http(reqwest::Error),
    Parse(String),
    Api(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Http(e) => write!(f, "HTTP error: {}", e),
            LlmError::Parse(e) => write!(f, "Parse error: {}", e),
            LlmError::Api(e) => write!(f, "API error: {}", e),
        }
    }
}

impl LlmClient {
    pub fn new(api_key: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            model: "gpt-4o-mini".to_string(),
        }
    }

    /// Validate the API key by making a lightweight request.
    pub async fn validate_key(&self) -> Result<bool, LlmError> {
        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: "Reply with just: ok".to_string(),
            }],
            temperature: 0.0,
            response_format: ResponseFormat { r#type: "text".to_string() },
        };
        match self.call_api(&req).await {
            Ok(_) => Ok(true),
            Err(LlmError::Api(msg)) if msg.contains("401") || msg.contains("invalid") => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Classify a single message into topics.
    pub async fn classify_message(
        &self,
        chat_title: &str,
        timestamp: i64,
        text: &str,
    ) -> Result<ClassifyResponse, LlmError> {
        let system = r#"You are a crypto/finance message classifier for a Telegram archive.
Classify the message into one or more topics. Return ONLY valid JSON.

Rules:
- topics: array of 1-3 topics this message relates to
- Each topic: concise English title (e.g., "ETH Layer 2 Fees", "Solana Outage")
- topic_ko: Korean title if inferrable, else null
- category: one of [DeFi, Trading, L1/L2, NFT, Airdrop, Regulation, Macro, Scam Alert, Other]
- relevance: 0.0-1.0 how relevant the message is to each topic
- skip: true if message is greeting, spam, bot command, emoji-only, or has no informational value

Response format:
{"skip": false, "topics": [{"topic": "...", "topic_ko": "...", "category": "...", "relevance": 0.8}]}
If skip=true, topics array should be empty: {"skip": true, "topics": []}"#;

        let truncated = if text.len() > 500 { &text[..500] } else { text };
        let user = format!("[Channel: {}] [{}]\n{}", chat_title, timestamp, truncated);

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage { role: "system".to_string(), content: system.to_string() },
                ChatMessage { role: "user".to_string(), content: user },
            ],
            temperature: 0.1,
            response_format: ResponseFormat { r#type: "json_object".to_string() },
        };

        let response_text = self.call_api(&req).await?;
        serde_json::from_str::<ClassifyResponse>(&response_text)
            .map_err(|e| LlmError::Parse(format!("Failed to parse classify response: {} — raw: {}", e, response_text)))
    }

    /// Generate a bilingual wiki summary for a topic.
    pub async fn generate_summary(
        &self,
        title: &str,
        category: &str,
        source_messages: &[(usize, i64, &str, &str)], // (index, timestamp, chat_title, text)
    ) -> Result<(String, String), LlmError> {
        let system = r#"Write a bilingual wiki article about a crypto/finance topic based on Telegram messages.
Every factual claim MUST cite its source using [N] notation matching the message index.

Structure:
## 요약
(Korean summary: 2-3 paragraphs, factual, cite sources as [1], [2], etc.)

### 핵심 포인트
- (Korean bullet points with citations)

### 타임라인
- (Korean chronological events with citations)

---

## Summary
(English version of the same content with same citations)

### Key Points
- (English bullet points with citations)

### Timeline
- (English chronological events with citations)

Rules:
- EVERY factual claim must have a [N] citation to a source message
- If sources disagree, note the disagreement
- If information is unverified or speculative, mark it as such
- Skip duplicate forwarded messages
- If fewer than 3 unique source messages, output "Insufficient sources for wiki article"
- Keep it concise"#;

        let mut user = format!("Topic: {}\nCategory: {}\n\nSource messages ({} total):\n",
            title, category, source_messages.len());
        for &(idx, ts, chat_title, text) in source_messages {
            let truncated = if text.len() > 300 { &text[..300] } else { text };
            user.push_str(&format!("[{}] [{}] [{}]: {}\n", idx, ts, chat_title, truncated));
        }

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                ChatMessage { role: "system".to_string(), content: system.to_string() },
                ChatMessage { role: "user".to_string(), content: user },
            ],
            temperature: 0.3,
            response_format: ResponseFormat { r#type: "text".to_string() },
        };

        let response = self.call_api(&req).await?;

        // Split into Korean and English sections
        let (ko, en) = split_bilingual(&response);
        Ok((ko, en))
    }

    /// Check if two topic titles refer to the same topic.
    pub async fn check_topic_dedup(
        &self,
        new_title: &str,
        existing_title: &str,
    ) -> Result<DedupResponse, LlmError> {
        let user = format!(
            "Are these the same crypto topic?\nNew: \"{}\"\nExisting: \"{}\"\nAnswer JSON: {{\"same\": true/false, \"confidence\": 0.0-1.0}}",
            new_title, existing_title
        );

        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![ChatMessage { role: "user".to_string(), content: user }],
            temperature: 0.0,
            response_format: ResponseFormat { r#type: "json_object".to_string() },
        };

        let response_text = self.call_api(&req).await?;
        serde_json::from_str::<DedupResponse>(&response_text)
            .map_err(|e| LlmError::Parse(format!("Dedup parse error: {} — raw: {}", e, response_text)))
    }

    async fn call_api(&self, req: &ChatRequest) -> Result<String, LlmError> {
        let resp = self
            .http
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(req)
            .send()
            .await
            .map_err(LlmError::Http)?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(LlmError::Api(format!("{}: {}", status, body)));
        }

        let chat_resp: ChatResponse = resp.json().await.map_err(LlmError::Http)?;
        chat_resp
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| LlmError::Api("No choices in response".to_string()))
    }
}

/// Split a bilingual article into Korean and English parts.
/// Splits on the "---" separator or "## Summary" header.
fn split_bilingual(text: &str) -> (String, String) {
    // Try splitting on "---" separator
    if let Some(pos) = text.find("\n---\n") {
        let ko = text[..pos].trim().to_string();
        let en = text[pos + 5..].trim().to_string();
        return (ko, en);
    }
    // Try splitting on "## Summary"
    if let Some(pos) = text.find("## Summary") {
        let ko = text[..pos].trim().to_string();
        let en = text[pos..].trim().to_string();
        return (ko, en);
    }
    // Fallback: entire text as both
    (text.to_string(), text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_bilingual() {
        let text = "## 요약\n한국어 내용\n\n---\n\n## Summary\nEnglish content";
        let (ko, en) = split_bilingual(text);
        assert!(ko.contains("한국어"));
        assert!(en.contains("English"));
    }

    #[test]
    fn test_split_bilingual_no_separator() {
        let text = "## 요약\n한국어\n\n## Summary\nEnglish";
        let (ko, en) = split_bilingual(text);
        assert!(ko.contains("한국어"));
        assert!(en.contains("English"));
    }
}
```

**Verify**: `cargo test -p telegram-korean-search llm`

---

### Task 2.3: Trending score calculation

**File**: `src-tauri/src/wiki/trending.rs`

```rust
/// Calculate trending score for a topic.
///
/// Formula: velocity × recency × log2(message_count + 1) × channel_diversity
///
/// - velocity: ratio of recent activity to weekly average
/// - recency: exponential decay based on hours since last message
/// - channel_diversity: fraction of channels mentioning this topic
pub fn calculate_trending_score(
    message_count: i64,
    last_seen_at: i64,
    msgs_24h: i64,
    msgs_7d: i64,
    unique_channels_7d: i64,
    total_active_channels: i64,
    now: i64,
) -> f64 {
    if message_count == 0 {
        return 0.0;
    }

    let hours_since_last = ((now - last_seen_at) as f64) / 3600.0;
    let recency = (-0.1 * hours_since_last).exp();

    let daily_avg_7d = (msgs_7d as f64) / 7.0;
    let velocity = (msgs_24h as f64) / daily_avg_7d.max(1.0);

    let total_channels = (total_active_channels as f64).max(1.0);
    let channel_div = (unique_channels_7d as f64) / total_channels;

    let base = (message_count as f64 + 1.0).log2();

    velocity * recency * base * channel_div
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trending_score_basic() {
        let now = 1700000000;
        let score = calculate_trending_score(
            100,            // message_count
            now - 3600,     // last_seen 1h ago
            20,             // 20 msgs in 24h
            50,             // 50 msgs in 7d
            5,              // 5 unique channels
            20,             // 20 total channels
            now,
        );
        assert!(score > 0.0);
    }

    #[test]
    fn test_trending_score_zero_messages() {
        let score = calculate_trending_score(0, 0, 0, 0, 0, 10, 1700000000);
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_trending_score_recency_decay() {
        let now = 1700000000;
        let recent = calculate_trending_score(100, now - 3600, 10, 30, 5, 20, now);
        let old = calculate_trending_score(100, now - 86400 * 7, 10, 30, 5, 20, now);
        assert!(recent > old, "Recent topics should score higher");
    }

    #[test]
    fn test_trending_score_velocity_boost() {
        let now = 1700000000;
        let spiking = calculate_trending_score(100, now - 3600, 50, 50, 5, 20, now);
        let steady = calculate_trending_score(100, now - 3600, 7, 50, 5, 20, now);
        assert!(spiking > steady, "Spiking topics should score higher");
    }

    #[test]
    fn test_trending_score_channel_diversity() {
        let now = 1700000000;
        let diverse = calculate_trending_score(100, now - 3600, 10, 30, 15, 20, now);
        let narrow = calculate_trending_score(100, now - 3600, 10, 30, 1, 20, now);
        assert!(diverse > narrow, "More diverse topics should score higher");
    }
}
```

**Verify**: `cargo test -p telegram-korean-search trending`

---

### Task 2.4: Background worker

**File**: `src-tauri/src/wiki/worker.rs`

This is the core classification worker. It runs on a dedicated thread, polls the queue, calls the LLM, and writes results to DB. Full implementation with:
- Stale claim recovery on startup
- Batch dequeue (5 at a time)
- Per-message classification with retry
- Topic alias matching + creation
- Stats rollup on each classification
- Trending score recalculation every 50 messages
- Graceful shutdown via AtomicBool
- Progress event emission

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};

use crate::store::wiki_topic::{NewTopic, TopicMessageLink, normalize_topic_title};
use crate::wiki::llm::{ClassifiedTopic, LlmClient};
use crate::wiki::trending::calculate_trending_score;
use crate::AppState;

/// Start the wiki classification worker on a dedicated thread.
/// Returns a handle that can be used to stop the worker.
pub fn start_worker(app: AppHandle, api_key: String) -> Arc<AtomicBool> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = Arc::clone(&shutdown);

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            run_worker(app, api_key, shutdown_clone).await;
        });
    });

    shutdown
}

async fn run_worker(app: AppHandle, api_key: String, shutdown: Arc<AtomicBool>) {
    let state = app.state::<AppState>();
    let llm = LlmClient::new(api_key);
    let mut processed_count: usize = 0;

    // Recover stale claims on startup
    {
        let store = state.store.lock().unwrap();
        let recovered = store.recover_stale_claims().unwrap_or(0);
        if recovered > 0 {
            log::info!("Wiki worker: recovered {} stale queue items", recovered);
        }
    }

    // Cache total active channels (refresh periodically)
    let mut total_channels = {
        let store = state.store.lock().unwrap();
        store.get_total_active_channels().unwrap_or(1)
    };

    loop {
        if shutdown.load(Ordering::Relaxed) {
            log::info!("Wiki worker: shutdown requested");
            let _ = app.emit("wiki-worker-stopped", serde_json::json!({"reason": "shutdown"}));
            break;
        }

        // Dequeue a batch
        let items = {
            let store = state.store.lock().unwrap();
            store.dequeue_classify_batch(5).unwrap_or_default()
        };

        if items.is_empty() {
            // Nothing to process — sleep and retry
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        }

        for item in &items {
            if shutdown.load(Ordering::Relaxed) {
                break;
            }

            // Read message text from DB
            let msg_data = {
                let store = state.store.lock().unwrap();
                store.get_message(item.chat_id, item.message_id).ok().flatten()
            };

            let msg = match msg_data {
                Some(m) => m,
                None => {
                    let store = state.store.lock().unwrap();
                    let _ = store.mark_queue_skipped(item.chat_id, item.message_id);
                    continue;
                }
            };

            // Get chat title
            let chat_title = {
                let store = state.store.lock().unwrap();
                store.get_chat(item.chat_id)
                    .ok()
                    .flatten()
                    .map(|c| c.title)
                    .unwrap_or_else(|| "Unknown".to_string())
            };

            // Skip empty messages
            if msg.text_plain.trim().is_empty() {
                let store = state.store.lock().unwrap();
                let _ = store.mark_queue_skipped(item.chat_id, item.message_id);
                continue;
            }

            // Classify via LLM with retry
            let classify_result = retry_classify(&llm, &chat_title, msg.timestamp, &msg.text_plain).await;

            match classify_result {
                Ok(response) => {
                    if response.skip || response.topics.is_empty() {
                        let store = state.store.lock().unwrap();
                        let _ = store.mark_queue_skipped(item.chat_id, item.message_id);
                    } else {
                        // Process each classified topic
                        let store = state.store.lock().unwrap();
                        for classified in &response.topics {
                            if let Err(e) = process_classified_topic(
                                &store, &classified, item.chat_id, item.message_id, msg.timestamp,
                            ) {
                                log::warn!("Failed to process topic '{}': {}", classified.topic, e);
                            }
                        }
                        let _ = store.mark_queue_done(item.chat_id, item.message_id);
                    }
                }
                Err(e) => {
                    log::warn!("Classification failed for ({}, {}): {}", item.chat_id, item.message_id, e);
                    let store = state.store.lock().unwrap();
                    let _ = store.mark_queue_failed(item.chat_id, item.message_id, &e.to_string());
                    let _ = app.emit("wiki-worker-error", serde_json::json!({
                        "message": e.to_string(),
                        "recoverable": true,
                    }));
                }
            }

            processed_count += 1;

            // Refresh total channels every 100 messages
            if processed_count % 100 == 0 {
                let store = state.store.lock().unwrap();
                total_channels = store.get_total_active_channels().unwrap_or(1);
            }

            // Recalculate trending scores every 50 messages
            if processed_count % 50 == 0 {
                recalculate_trending(&state, total_channels);
            }

            // Emit progress
            let stats = {
                let store = state.store.lock().unwrap();
                store.get_queue_stats().unwrap_or(crate::store::wiki_queue::QueueStats {
                    pending: 0, processing: 0, done: 0, failed: 0, skipped: 0,
                })
            };
            let _ = app.emit("wiki-worker-progress", serde_json::json!({
                "processed": stats.done + stats.skipped,
                "total": stats.done + stats.skipped + stats.pending + stats.failed,
                "queue_remaining": stats.pending,
            }));

            // Rate limit: 200ms between calls
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    // Final trending recalculation
    recalculate_trending(&state, total_channels);
}

/// Classify with up to 3 retries and exponential backoff.
async fn retry_classify(
    llm: &LlmClient,
    chat_title: &str,
    timestamp: i64,
    text: &str,
) -> Result<crate::wiki::llm::ClassifyResponse, crate::wiki::llm::LlmError> {
    let mut last_err = None;
    for attempt in 0..3 {
        match llm.classify_message(chat_title, timestamp, text).await {
            Ok(resp) => return Ok(resp),
            Err(e) => {
                log::warn!("Classify attempt {} failed: {}", attempt + 1, e);
                last_err = Some(e);
                let delay = std::time::Duration::from_millis(500 * 2u64.pow(attempt));
                tokio::time::sleep(delay).await;
            }
        }
    }
    Err(last_err.unwrap())
}

/// Process a single classified topic: find or create, link message, update stats.
fn process_classified_topic(
    store: &crate::store::Store,
    classified: &ClassifiedTopic,
    chat_id: i64,
    message_id: i64,
    message_timestamp: i64,
) -> Result<(), sqlite::Error> {
    // Try to find existing topic by alias
    let topic_id = match store.find_topic_by_alias(&classified.topic)? {
        Some(id) => {
            // Set Korean title if not yet set
            if let Some(ref ko) = classified.topic_ko {
                store.set_title_ko_if_absent(id, ko)?;
            }
            // Add new alias variant
            let alias = normalize_topic_title(&classified.topic);
            store.add_topic_alias(id, &alias)?;
            id
        }
        None => {
            // Create new topic
            let category_id = store.normalize_category(&classified.category)?;
            let new_topic = NewTopic {
                title: classified.topic.clone(),
                title_ko: classified.topic_ko.clone(),
                category_id,
            };
            store.create_topic(&new_topic)?
        }
    };

    // Link message to topic
    let link = TopicMessageLink {
        topic_id,
        chat_id,
        message_id,
        relevance: classified.relevance,
        assigned_category: classified.category.clone(),
    };
    store.link_message_to_topic(&link)?;

    // Update daily stats
    store.record_topic_stat(topic_id, message_timestamp, chat_id)?;

    // Check category reconciliation
    if let Some(new_cat_id) = store.check_category_reconciliation(topic_id)? {
        store.update_topic_category(topic_id, new_cat_id)?;
    }

    Ok(())
}

/// Recalculate trending scores for all recently active topics.
fn recalculate_trending(state: &tauri::State<AppState>, total_channels: i64) {
    let store = state.store.lock().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let topic_ids = store.get_active_topic_ids(30).unwrap_or_default();

    for topic_id in topic_ids {
        let topic = match store.get_topic(topic_id) {
            Ok(Some(t)) => t,
            _ => continue,
        };
        let msgs_24h = store.get_topic_msg_count_days(topic_id, 1).unwrap_or(0);
        let msgs_7d = store.get_topic_msg_count_days(topic_id, 7).unwrap_or(0);
        let channels_7d = store.get_topic_channel_count_days(topic_id, 7).unwrap_or(0);

        let score = calculate_trending_score(
            topic.message_count,
            topic.last_seen_at.unwrap_or(0),
            msgs_24h,
            msgs_7d,
            channels_7d,
            total_channels,
            now,
        );

        let _ = store.update_trending_score(topic_id, score);
    }
}
```

**Verify**: `cargo check -p telegram-korean-search` (worker requires runtime context, tested via integration)

---

### Task 2.5: Phase 2 verification

```bash
cd src-tauri
cargo fmt
cargo clippy -- -D warnings
cargo test
```

**Commit**: `git add -A && git commit -m "feat: wiki LLM client, background worker, trending calculation"`

---

## Phase 3: Tauri Commands & Collection Integration

### Task 3.1: Add wiki worker handle to AppState

**File**: `src-tauri/src/lib.rs`

Add to AppState struct:
```rust
pub wiki_worker_shutdown: TokioMutex<Option<Arc<std::sync::atomic::AtomicBool>>>,
```

Add to the `.manage(AppState { ... })` block:
```rust
wiki_worker_shutdown: TokioMutex::new(None),
```

Add import at top:
```rust
use std::sync::Arc;
```

---

### Task 3.2: Wiki Tauri commands

**File**: `src-tauri/src/commands.rs`

Add the following wiki commands after the existing `start_collection` command. Add required imports at top.

Commands to add:
- `save_openai_api_key` — store in macOS Keychain
- `get_openai_api_key` — retrieve (masked)
- `validate_openai_api_key` — test API call
- `start_wiki_worker` — start background worker
- `stop_wiki_worker` — graceful shutdown
- `get_wiki_status` — queue stats + running state
- `reprocess_wiki` — clear and re-enqueue all
- `clear_wiki_data` — full wiki reset
- `get_trending_topics` — browse trending
- `get_wiki_categories` — category list
- `get_topic_detail` — topic + page + sources
- `get_topic_sources` — paginated source messages
- `search_wiki` — search topics + FTS5 pages
- `generate_topic_summary` — on-demand summary generation for a topic

Each follows the existing pattern: `State<AppState>`, `state.store.lock().map_err(|e| e.to_string())?`, `Result<T, String>`.

---

### Task 3.3: Register wiki commands in invoke_handler

**File**: `src-tauri/src/lib.rs`

Add all new command names to the `invoke_handler(tauri::generate_handler![...])` block.

---

### Task 3.4: Collection integration — enqueue after sync

**File**: `src-tauri/src/commands.rs`

In `run_collection()`, after the message batch is successfully saved to DB (inside the `Ok(rows)` match arm), add:

```rust
// Enqueue for wiki classification
if !rows.is_empty() {
    let items: Vec<(i64, i64)> = rows.iter().map(|r| (r.chat_id, r.message_id)).collect();
    if let Err(e) = store.enqueue_for_classification(&items) {
        log::warn!("Failed to enqueue for wiki: {}", e);
    }
}
```

After collection completes, check if wiki worker should auto-start:
```rust
// Auto-start wiki worker if API key exists and worker not running
{
    let store = state.store.lock().unwrap();
    if store.get_meta("openai_api_key_exists").ok().flatten().is_some() {
        // Check via keychain
    }
}
```

---

### Task 3.5: Graceful shutdown for wiki worker on app exit

**File**: `src-tauri/src/lib.rs`

In the `.run(|app, event|` handler, add wiki worker shutdown before the Telegram runner shutdown.

---

### Task 3.6: Phase 3 verification

```bash
cd src-tauri
cargo fmt
cargo clippy -- -D warnings
cargo test
```

**Commit**: `git add -A && git commit -m "feat: wiki Tauri commands, collection integration, auto-start worker"`

---

## Phase 4: Frontend

### Task 4.1: Install react-markdown

```bash
bun add react-markdown
```

---

### Task 4.2: TypeScript types

**File**: `src/types/index.ts`

Add wiki types after existing types (WikiTopic, WikiPage, WikiTopicDetail, WikiCategory, WikiSearchResult, WikiProgress, QueueStats, PageSource).

---

### Task 4.3: Tauri API wrappers

**File**: `src/api/tauri.ts`

Add invoke wrappers for all wiki commands + event listeners for `wiki-worker-progress`, `wiki-worker-error`, `wiki-worker-stopped`.

---

### Task 4.4: TabBar component

**File**: `src/components/TabBar.tsx`

Simple two-tab header. Props: `activeTab`, `onTabChange`. Styled to match existing dark theme.

---

### Task 4.5: App.tsx — add tab routing

**File**: `src/App.tsx`

Add state for `activeTab`. When `step === "ready"`, render TabBar + conditional SearchPage/WikiPage.

---

### Task 4.6: useWikiWorker hook

**File**: `src/hooks/useWikiWorker.ts`

Manages: API key state, worker running state, progress events, start/stop controls. Listens to `wiki-worker-progress`, `wiki-worker-error`, `wiki-worker-stopped` events.

---

### Task 4.7: useWiki hook

**File**: `src/hooks/useWiki.ts`

Manages: trending topics, selected topic, category filter, wiki search. Loads categories on mount, loads trending on mount, handles topic selection with detail + page loading.

---

### Task 4.8: WikiPage container

**File**: `src/pages/WikiPage.tsx`

Routes between: TrendingDashboard (default), WikiArticle (when topic selected). Uses useWiki and useWikiWorker hooks.

---

### Task 4.9: TrendingDashboard + TopicCard + CategoryFilter

**Files**: `src/components/wiki/TrendingDashboard.tsx`, `TopicCard.tsx`, `CategoryFilter.tsx`

TrendingDashboard renders category pills, search bar, and scrollable list of TopicCards. TopicCard shows: title, category badge, trending score %, message count, channel count, last updated.

---

### Task 4.10: WikiArticle + SourceMessages

**Files**: `src/components/wiki/WikiArticle.tsx`, `SourceMessages.tsx`

WikiArticle renders bilingual markdown via react-markdown. Back button. Collapsible SourceMessages section at bottom with numbered citations matching the article.

---

### Task 4.11: WikiSettings

**File**: `src/components/wiki/WikiSettings.tsx`

Collapsible settings panel: API key input (masked), validate button, queue stats, worker start/stop, reprocess/clear buttons.

---

### Task 4.12: WikiSearch

**File**: `src/components/wiki/WikiSearch.tsx`

Search input with debounce. Searches both topic titles and FTS5 page content. Shows combined results.

---

### Task 4.13: CSS styles

**File**: `src/App.css`

Add wiki-specific styles following existing dark theme patterns (#1e1e1e bg, #0078d4 accent, #e0e0e0 text).

---

### Task 4.14: Phase 4 verification

```bash
bun install
cargo tauri dev
```

Manual test:
1. App loads, TabBar visible with Search and Wiki tabs
2. Wiki tab shows settings panel (no API key yet)
3. Enter OpenAI API key, validate
4. Collect some messages
5. Wiki worker auto-starts, progress shown
6. Topics appear on trending dashboard
7. Click topic → wiki article renders with citations
8. Source messages expand below article
9. Wiki search works
10. Category filter works

```bash
cd src-tauri && cargo fmt && cargo clippy -- -D warnings && cargo test
```

**Commit**: `git add -A && git commit -m "feat: wiki frontend — trending dashboard, articles, settings, search"`

---

## Verification Checklist

- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy -- -D warnings` passes
- [ ] `cargo test` passes (all new + existing tests)
- [ ] `cargo tauri dev` launches without errors
- [ ] Wiki tab renders correctly
- [ ] API key save/load works via Keychain
- [ ] Message collection enqueues to wiki queue
- [ ] Wiki worker processes queue items
- [ ] Topics appear on dashboard with correct trending scores
- [ ] Wiki articles render bilingual content with citations
- [ ] Source messages display correctly under articles
- [ ] Wiki search returns relevant results
- [ ] Category filter works
- [ ] Worker gracefully stops on app exit
- [ ] Stale queue items recover on worker restart
