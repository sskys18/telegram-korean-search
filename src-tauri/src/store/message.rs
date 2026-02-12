use serde::{Deserialize, Serialize};

use super::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRow {
    pub message_id: i64,
    pub chat_id: i64,
    pub timestamp: i64,
    pub text_plain: String,
    pub text_stripped: String,
    pub link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageWithChat {
    pub message_id: i64,
    pub chat_id: i64,
    pub timestamp: i64,
    pub text_plain: String,
    pub link: Option<String>,
    pub chat_title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cursor {
    pub timestamp: i64,
    pub chat_id: i64,
    pub message_id: i64,
}

pub fn strip_whitespace(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

impl Store {
    pub fn insert_messages_batch(&self, messages: &[MessageRow]) -> Result<(), sqlite::Error> {
        self.conn.execute("BEGIN")?;
        for msg in messages {
            let mut stmt = self.conn.prepare(
                "INSERT OR IGNORE INTO messages (message_id, chat_id, timestamp, text_plain, text_stripped, link)
                 VALUES (?, ?, ?, ?, ?, ?)",
            )?;
            stmt.bind((1, msg.message_id))?;
            stmt.bind((2, msg.chat_id))?;
            stmt.bind((3, msg.timestamp))?;
            stmt.bind((4, msg.text_plain.as_str()))?;
            stmt.bind((5, msg.text_stripped.as_str()))?;
            match &msg.link {
                Some(l) => stmt.bind((6, l.as_str()))?,
                None => stmt.bind((6, sqlite::Value::Null))?,
            };
            stmt.next()?;

            // Check if the row was actually inserted (not a duplicate)
            let mut changes_stmt = self.conn.prepare("SELECT changes()")?;
            changes_stmt.next()?;
            let changes: i64 = changes_stmt.read(0)?;

            if changes > 0 {
                // New message — index in FTS5
                let mut rowid_stmt = self.conn.prepare("SELECT last_insert_rowid()")?;
                rowid_stmt.next()?;
                let msg_rowid: i64 = rowid_stmt.read(0)?;

                let mut fts_stmt = self.conn.prepare(
                    "INSERT INTO messages_fts(rowid, text_plain) VALUES (?, ?)",
                )?;
                fts_stmt.bind((1, msg_rowid))?;
                fts_stmt.bind((2, msg.text_plain.as_str()))?;
                fts_stmt.next()?;
            }
        }
        self.conn.execute("COMMIT")?;
        Ok(())
    }

    pub fn get_message(
        &self,
        chat_id: i64,
        message_id: i64,
    ) -> Result<Option<MessageRow>, sqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT message_id, chat_id, timestamp, text_plain, text_stripped, link
             FROM messages WHERE chat_id = ? AND message_id = ?",
        )?;
        stmt.bind((1, chat_id))?;
        stmt.bind((2, message_id))?;
        if let Ok(sqlite::State::Row) = stmt.next() {
            Ok(Some(MessageRow {
                message_id: stmt.read::<i64, _>(0)?,
                chat_id: stmt.read::<i64, _>(1)?,
                timestamp: stmt.read::<i64, _>(2)?,
                text_plain: stmt.read::<String, _>(3)?,
                text_stripped: stmt.read::<String, _>(4)?,
                link: stmt.read::<Option<String>, _>(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn search_messages_fts(
        &self,
        fts_query: &str,
        cursor: Option<&Cursor>,
        limit: usize,
    ) -> Result<Vec<MessageWithChat>, sqlite::Error> {
        let cursor_clause = if cursor.is_some() {
            "AND (m.timestamp < ?
                  OR (m.timestamp = ? AND m.chat_id > ?)
                  OR (m.timestamp = ? AND m.chat_id = ? AND m.message_id > ?))"
        } else {
            ""
        };

        let sql = format!(
            "SELECT m.message_id, m.chat_id, m.timestamp, m.text_plain, m.link, c.title
             FROM messages m
             JOIN chats c ON m.chat_id = c.chat_id
             WHERE m.rowid IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)
             AND c.is_excluded = 0
             {}
             ORDER BY m.timestamp DESC, m.chat_id ASC, m.message_id ASC
             LIMIT ?",
            cursor_clause
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let mut bind_idx = 1;
        stmt.bind((bind_idx, fts_query))?;
        bind_idx += 1;
        if let Some(c) = cursor {
            stmt.bind((bind_idx, c.timestamp))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.timestamp))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.chat_id))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.timestamp))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.chat_id))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.message_id))?;
            bind_idx += 1;
        }
        stmt.bind((bind_idx, limit as i64))?;

        let mut results = Vec::new();
        while let Ok(sqlite::State::Row) = stmt.next() {
            results.push(MessageWithChat {
                message_id: stmt.read::<i64, _>(0)?,
                chat_id: stmt.read::<i64, _>(1)?,
                timestamp: stmt.read::<i64, _>(2)?,
                text_plain: stmt.read::<String, _>(3)?,
                link: stmt.read::<Option<String>, _>(4)?,
                chat_title: stmt.read::<String, _>(5)?,
            });
        }

        Ok(results)
    }

    pub fn search_messages_fts_in_chat(
        &self,
        fts_query: &str,
        chat_id: i64,
        cursor: Option<&Cursor>,
        limit: usize,
    ) -> Result<Vec<MessageWithChat>, sqlite::Error> {
        let cursor_clause = if cursor.is_some() {
            "AND (m.timestamp < ?
                  OR (m.timestamp = ? AND m.message_id > ?))"
        } else {
            ""
        };

        let sql = format!(
            "SELECT m.message_id, m.chat_id, m.timestamp, m.text_plain, m.link, c.title
             FROM messages m
             JOIN chats c ON m.chat_id = c.chat_id
             WHERE m.rowid IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)
             AND m.chat_id = ? AND c.is_excluded = 0
             {}
             ORDER BY m.timestamp DESC, m.message_id ASC
             LIMIT ?",
            cursor_clause
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let mut bind_idx = 1;
        stmt.bind((bind_idx, fts_query))?;
        bind_idx += 1;
        stmt.bind((bind_idx, chat_id))?;
        bind_idx += 1;
        if let Some(c) = cursor {
            stmt.bind((bind_idx, c.timestamp))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.timestamp))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.message_id))?;
            bind_idx += 1;
        }
        stmt.bind((bind_idx, limit as i64))?;

        let mut results = Vec::new();
        while let Ok(sqlite::State::Row) = stmt.next() {
            results.push(MessageWithChat {
                message_id: stmt.read::<i64, _>(0)?,
                chat_id: stmt.read::<i64, _>(1)?,
                timestamp: stmt.read::<i64, _>(2)?,
                text_plain: stmt.read::<String, _>(3)?,
                link: stmt.read::<Option<String>, _>(4)?,
                chat_title: stmt.read::<String, _>(5)?,
            });
        }

        Ok(results)
    }

    /// LIKE-based search fallback for queries with terms shorter than 3 chars
    /// (FTS5 trigram needs >= 3 chars to produce trigrams).
    pub fn search_messages_like(
        &self,
        terms: &[String],
        cursor: Option<&Cursor>,
        limit: usize,
    ) -> Result<Vec<MessageWithChat>, sqlite::Error> {
        if terms.is_empty() {
            return Ok(vec![]);
        }

        let like_clauses: Vec<String> = terms
            .iter()
            .map(|_| "m.text_plain LIKE '%' || ? || '%'".to_string())
            .collect();
        let like_where = like_clauses.join(" AND ");

        let cursor_clause = if cursor.is_some() {
            "AND (m.timestamp < ?
                  OR (m.timestamp = ? AND m.chat_id > ?)
                  OR (m.timestamp = ? AND m.chat_id = ? AND m.message_id > ?))"
        } else {
            ""
        };

        let sql = format!(
            "SELECT m.message_id, m.chat_id, m.timestamp, m.text_plain, m.link, c.title
             FROM messages m
             JOIN chats c ON m.chat_id = c.chat_id
             WHERE {} AND c.is_excluded = 0
             {}
             ORDER BY m.timestamp DESC, m.chat_id ASC, m.message_id ASC
             LIMIT ?",
            like_where, cursor_clause
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let mut bind_idx = 1;
        for term in terms {
            stmt.bind((bind_idx, term.as_str()))?;
            bind_idx += 1;
        }
        if let Some(c) = cursor {
            stmt.bind((bind_idx, c.timestamp))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.timestamp))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.chat_id))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.timestamp))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.chat_id))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.message_id))?;
            bind_idx += 1;
        }
        stmt.bind((bind_idx, limit as i64))?;

        let mut results = Vec::new();
        while let Ok(sqlite::State::Row) = stmt.next() {
            results.push(MessageWithChat {
                message_id: stmt.read::<i64, _>(0)?,
                chat_id: stmt.read::<i64, _>(1)?,
                timestamp: stmt.read::<i64, _>(2)?,
                text_plain: stmt.read::<String, _>(3)?,
                link: stmt.read::<Option<String>, _>(4)?,
                chat_title: stmt.read::<String, _>(5)?,
            });
        }

        Ok(results)
    }

    pub fn search_messages_like_in_chat(
        &self,
        terms: &[String],
        chat_id: i64,
        cursor: Option<&Cursor>,
        limit: usize,
    ) -> Result<Vec<MessageWithChat>, sqlite::Error> {
        if terms.is_empty() {
            return Ok(vec![]);
        }

        let like_clauses: Vec<String> = terms
            .iter()
            .map(|_| "m.text_plain LIKE '%' || ? || '%'".to_string())
            .collect();
        let like_where = like_clauses.join(" AND ");

        let cursor_clause = if cursor.is_some() {
            "AND (m.timestamp < ?
                  OR (m.timestamp = ? AND m.message_id > ?))"
        } else {
            ""
        };

        let sql = format!(
            "SELECT m.message_id, m.chat_id, m.timestamp, m.text_plain, m.link, c.title
             FROM messages m
             JOIN chats c ON m.chat_id = c.chat_id
             WHERE {} AND m.chat_id = ? AND c.is_excluded = 0
             {}
             ORDER BY m.timestamp DESC, m.message_id ASC
             LIMIT ?",
            like_where, cursor_clause
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let mut bind_idx = 1;
        for term in terms {
            stmt.bind((bind_idx, term.as_str()))?;
            bind_idx += 1;
        }
        stmt.bind((bind_idx, chat_id))?;
        bind_idx += 1;
        if let Some(c) = cursor {
            stmt.bind((bind_idx, c.timestamp))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.timestamp))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.message_id))?;
            bind_idx += 1;
        }
        stmt.bind((bind_idx, limit as i64))?;

        let mut results = Vec::new();
        while let Ok(sqlite::State::Row) = stmt.next() {
            results.push(MessageWithChat {
                message_id: stmt.read::<i64, _>(0)?,
                chat_id: stmt.read::<i64, _>(1)?,
                timestamp: stmt.read::<i64, _>(2)?,
                text_plain: stmt.read::<String, _>(3)?,
                link: stmt.read::<Option<String>, _>(4)?,
                chat_title: stmt.read::<String, _>(5)?,
            });
        }

        Ok(results)
    }

    pub fn message_count(&self) -> Result<i64, sqlite::Error> {
        let mut stmt = self.conn.prepare("SELECT COUNT(*) FROM messages")?;
        stmt.next()?;
        stmt.read::<i64, _>(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::chat::ChatRow;

    fn test_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn setup_chat(store: &Store, chat_id: i64) {
        store
            .upsert_chat(&ChatRow {
                chat_id,
                title: format!("Chat {}", chat_id),
                chat_type: "supergroup".to_string(),
                username: None,
                access_hash: None,
                is_excluded: false,
            })
            .unwrap();
    }

    fn make_message(chat_id: i64, msg_id: i64, ts: i64, text: &str) -> MessageRow {
        MessageRow {
            message_id: msg_id,
            chat_id,
            timestamp: ts,
            text_plain: text.to_string(),
            text_stripped: strip_whitespace(text),
            link: None,
        }
    }

    #[test]
    fn test_insert_and_get() {
        let store = test_store();
        setup_chat(&store, 1);

        let msg = make_message(1, 100, 1000, "hello world");
        store.insert_messages_batch(&[msg]).unwrap();

        let fetched = store.get_message(1, 100).unwrap().unwrap();
        assert_eq!(fetched.text_plain, "hello world");
        assert_eq!(fetched.text_stripped, "helloworld");
    }

    #[test]
    fn test_batch_insert() {
        let store = test_store();
        setup_chat(&store, 1);

        let messages: Vec<MessageRow> = (0..100)
            .map(|i| make_message(1, i, 1000 + i, &format!("message {}", i)))
            .collect();
        store.insert_messages_batch(&messages).unwrap();
        assert_eq!(store.message_count().unwrap(), 100);
    }

    #[test]
    fn test_duplicate_insert_ignored() {
        let store = test_store();
        setup_chat(&store, 1);

        let msg = make_message(1, 100, 1000, "hello");
        store.insert_messages_batch(&[msg.clone()]).unwrap();
        store.insert_messages_batch(&[msg]).unwrap();
        assert_eq!(store.message_count().unwrap(), 1);
    }

    #[test]
    fn test_fts_search_long_query() {
        let store = test_store();
        setup_chat(&store, 1);

        store
            .insert_messages_batch(&[
                make_message(1, 1, 1000, "삼성전자 주가가 상승했다"),
                make_message(1, 2, 1001, "오늘 날씨가 좋습니다"),
            ])
            .unwrap();

        // FTS5 trigram needs >= 3 chars
        let results = store.search_messages_fts("\"삼성전\"", None, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message_id, 1);
    }

    #[test]
    fn test_like_search_short_query() {
        let store = test_store();
        setup_chat(&store, 1);

        store
            .insert_messages_batch(&[
                make_message(1, 1, 1000, "삼성전자 주가가 상승했다"),
                make_message(1, 2, 1001, "오늘 날씨가 좋습니다"),
            ])
            .unwrap();

        // LIKE fallback for < 3 char queries
        let terms = vec!["삼성".to_string()];
        let results = store.search_messages_like(&terms, None, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message_id, 1);
    }

    #[test]
    fn test_fts_search_in_chat() {
        let store = test_store();
        setup_chat(&store, 1);
        setup_chat(&store, 2);

        store
            .insert_messages_batch(&[
                make_message(1, 1, 1000, "hello from chat 1"),
                make_message(2, 1, 1001, "hello from chat 2"),
            ])
            .unwrap();

        let results = store
            .search_messages_fts_in_chat("\"hello\"", 1, None, 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chat_id, 1);
    }

    #[test]
    fn test_strip_whitespace() {
        assert_eq!(strip_whitespace("삼성 전자 주가"), "삼성전자주가");
        assert_eq!(strip_whitespace("hello world"), "helloworld");
        assert_eq!(strip_whitespace("  spaces  "), "spaces");
    }

    #[test]
    fn test_message_count() {
        let store = test_store();
        assert_eq!(store.message_count().unwrap(), 0);
    }
}
