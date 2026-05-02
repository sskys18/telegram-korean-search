use serde::{Deserialize, Serialize};

use super::Store;

fn fts_insert(
    conn: &sqlite::Connection,
    rowid: i64,
    text_plain: &str,
    text_stripped: &str,
    text_jamo: &str,
) -> Result<(), sqlite::Error> {
    let mut stmt = conn.prepare(
        "INSERT INTO messages_fts(rowid, text_plain, text_stripped, text_jamo)
         VALUES (?, ?, ?, ?)",
    )?;
    stmt.bind((1, rowid))?;
    stmt.bind((2, text_plain))?;
    stmt.bind((3, text_stripped))?;
    stmt.bind((4, text_jamo))?;
    stmt.next()?;
    Ok(())
}

fn fts_delete(
    conn: &sqlite::Connection,
    rowid: i64,
    text_plain: &str,
    text_stripped: &str,
    text_jamo: &str,
) -> Result<(), sqlite::Error> {
    let mut stmt = conn.prepare(
        "INSERT INTO messages_fts(messages_fts, rowid, text_plain, text_stripped, text_jamo)
         VALUES('delete', ?, ?, ?, ?)",
    )?;
    stmt.bind((1, rowid))?;
    stmt.bind((2, text_plain))?;
    stmt.bind((3, text_stripped))?;
    stmt.bind((4, text_jamo))?;
    stmt.next()?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRow {
    pub message_id: i64,
    pub chat_id: i64,
    pub timestamp: i64,
    pub text_plain: String,
    pub text_stripped: String,
    pub link: Option<String>,
    pub sender_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexOutcome {
    pub inserted: u64,
    pub updated: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRef {
    pub chat_id: i64,
    pub message_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageWithChat {
    pub message_id: i64,
    pub chat_id: i64,
    pub timestamp: i64,
    pub text_plain: String,
    pub link: Option<String>,
    pub chat_title: String,
    pub rank: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cursor {
    #[serde(default)]
    pub rank: f64,
    pub timestamp: i64,
    pub chat_id: i64,
    pub message_id: i64,
}

pub fn strip_whitespace(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn enqueue_wiki_classify(
    conn: &sqlite::Connection,
    chat_id: i64,
    message_id: i64,
    text_plain: &str,
) -> Result<(), sqlite::Error> {
    // v1 queue: kept until phase-6 worker rewrite consumes v2 instead.
    let mut q = conn.prepare(
        "INSERT OR IGNORE INTO wiki_classify_queue (chat_id, message_id)
         VALUES (?, ?)",
    )?;
    q.bind((1, chat_id))?;
    q.bind((2, message_id))?;
    q.next()?;

    // v2 queue: spec §6.1 ingest. NFC-normalized blake3-16 over text_plain.
    // Match logic: existing row with same hash = noop;
    // existing row with different hash = reset to pending and bump hash;
    // missing row = insert pending.
    let text_hash = crate::wiki::norm::blake3_16_nfc(text_plain);
    let now = crate::wiki::norm::unix_now();

    let existing: Option<(String, Vec<u8>)> = {
        let mut stmt = conn.prepare(
            "SELECT status, text_hash FROM wiki_classify_queue_v2
             WHERE msg_id = ? AND chat_id = ?",
        )?;
        stmt.bind((1, message_id))?;
        stmt.bind((2, chat_id))?;
        if let sqlite::State::Row = stmt.next()? {
            Some((stmt.read::<String, _>(0)?, stmt.read::<Vec<u8>, _>(1)?))
        } else {
            None
        }
    };

    match existing {
        None => {
            let mut stmt = conn.prepare(
                "INSERT INTO wiki_classify_queue_v2
                    (msg_id, chat_id, status, attempts, text_hash,
                     enqueued_at, next_attempt_at)
                 VALUES (?, ?, 'pending', 0, ?, ?, ?)",
            )?;
            stmt.bind((1, message_id))?;
            stmt.bind((2, chat_id))?;
            stmt.bind((3, text_hash.as_slice()))?;
            stmt.bind((4, now))?;
            stmt.bind((5, now))?;
            stmt.next()?;
        }
        Some((_, prior_hash)) if prior_hash == text_hash => {
            // Same text — leave attempts/error/status alone.
        }
        Some((status, _)) if status == "pending" => {
            // Edited before pickup: just bump hash + reschedule.
            let mut stmt = conn.prepare(
                "UPDATE wiki_classify_queue_v2
                    SET text_hash = ?, next_attempt_at = ?, enqueued_at = ?
                  WHERE msg_id = ? AND chat_id = ?",
            )?;
            stmt.bind((1, text_hash.as_slice()))?;
            stmt.bind((2, now))?;
            stmt.bind((3, now))?;
            stmt.bind((4, message_id))?;
            stmt.bind((5, chat_id))?;
            stmt.next()?;
        }
        Some(_) => {
            // done / processing / failed with different text: full reset.
            let mut stmt = conn.prepare(
                "UPDATE wiki_classify_queue_v2
                    SET status = 'pending', attempts = 0, last_error = NULL,
                        hint = NULL, hint_page_id = NULL,
                        text_hash = ?, claimed_at = NULL,
                        next_attempt_at = ?, enqueued_at = ?
                  WHERE msg_id = ? AND chat_id = ?",
            )?;
            stmt.bind((1, text_hash.as_slice()))?;
            stmt.bind((2, now))?;
            stmt.bind((3, now))?;
            stmt.bind((4, message_id))?;
            stmt.bind((5, chat_id))?;
            stmt.next()?;
        }
    }
    Ok(())
}

fn like_variants(term: &str) -> [String; 3] {
    [
        term.to_string(),
        strip_whitespace(term),
        crate::search::hangul::decompose_jamo(term),
    ]
}

impl Store {
    pub fn insert_messages_batch(
        &self,
        messages: &[MessageRow],
    ) -> Result<IndexOutcome, sqlite::Error> {
        // Clean up any transaction leftover from a previous failed call so
        // this one does not hit "cannot start a transaction within a
        // transaction". Ignore the error if no txn is active.
        let _ = self.conn.execute("ROLLBACK");
        self.conn.execute("BEGIN")?;
        let result = (|| -> Result<IndexOutcome, sqlite::Error> {
            let mut outcome = IndexOutcome::default();
            // Satisfy the FK from messages.chat_id -> chats.chat_id without
            // requiring callers to call upsert_chat first. The shell (Swift)
            // mirror may upsert a richer ChatInfo later; this stub keeps
            // ingestion unblocked when it does not.
            let mut seen_chats: std::collections::HashSet<i64> = std::collections::HashSet::new();
            for msg in messages {
                if seen_chats.insert(msg.chat_id) {
                    let mut stmt = self.conn.prepare(
                    "INSERT OR IGNORE INTO chats (chat_id, title, chat_type, username, access_hash, is_excluded)
                     VALUES (?, '', 'dm', NULL, NULL, 0)",
                )?;
                    stmt.bind((1, msg.chat_id))?;
                    stmt.next()?;
                }
            }
            for msg in messages {
                let jamo = crate::search::hangul::decompose_jamo(&msg.text_plain);
                let prior = {
                    let mut stmt = self.conn.prepare(
                        "SELECT rowid, timestamp, text_plain, text_stripped, text_jamo, link, sender_id
                         FROM messages WHERE chat_id = ? AND message_id = ?",
                    )?;
                    stmt.bind((1, msg.chat_id))?;
                    stmt.bind((2, msg.message_id))?;
                    if let sqlite::State::Row = stmt.next()? {
                        Some((
                            stmt.read::<i64, _>(0)?,
                            stmt.read::<i64, _>(1)?,
                            stmt.read::<String, _>(2)?,
                            stmt.read::<String, _>(3)?,
                            stmt.read::<String, _>(4)?,
                            stmt.read::<Option<String>, _>(5)?,
                            stmt.read::<Option<i64>, _>(6)?,
                        ))
                    } else {
                        None
                    }
                };

                match prior {
                    None => {
                        let mut stmt = self.conn.prepare(
                            "INSERT INTO messages
                                (message_id, chat_id, timestamp, text_plain, text_stripped, link,
                                 text_jamo, sender_id)
                             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
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
                        stmt.bind((7, jamo.as_str()))?;
                        stmt.bind((8, msg.sender_id))?;
                        stmt.next()?;

                        let mut rowid_stmt = self.conn.prepare("SELECT last_insert_rowid()")?;
                        rowid_stmt.next()?;
                        let rowid: i64 = rowid_stmt.read(0)?;

                        fts_insert(
                            &self.conn,
                            rowid,
                            &msg.text_plain,
                            &msg.text_stripped,
                            &jamo,
                        )?;
                        enqueue_wiki_classify(
                            &self.conn,
                            msg.chat_id,
                            msg.message_id,
                            &msg.text_plain,
                        )?;
                        outcome.inserted += 1;
                    }
                    Some((
                        rowid,
                        old_ts,
                        old_plain,
                        old_stripped,
                        old_jamo,
                        old_link,
                        old_sender,
                    )) => {
                        if old_ts == msg.timestamp
                            && old_plain == msg.text_plain
                            && old_stripped == msg.text_stripped
                            && old_jamo == jamo
                            && old_link == msg.link
                            && old_sender == Some(msg.sender_id)
                        {
                            continue;
                        }

                        let text_changed = old_plain != msg.text_plain
                            || old_stripped != msg.text_stripped
                            || old_jamo != jamo;

                        let mut stmt = self.conn.prepare(
                            "UPDATE messages
                             SET timestamp = ?, text_plain = ?, text_stripped = ?, link = ?, text_jamo = ?, sender_id = ?
                             WHERE rowid = ?",
                        )?;
                        stmt.bind((1, msg.timestamp))?;
                        stmt.bind((2, msg.text_plain.as_str()))?;
                        stmt.bind((3, msg.text_stripped.as_str()))?;
                        match &msg.link {
                            Some(l) => stmt.bind((4, l.as_str()))?,
                            None => stmt.bind((4, sqlite::Value::Null))?,
                        };
                        stmt.bind((5, jamo.as_str()))?;
                        stmt.bind((6, msg.sender_id))?;
                        stmt.bind((7, rowid))?;
                        stmt.next()?;

                        if text_changed {
                            fts_delete(&self.conn, rowid, &old_plain, &old_stripped, &old_jamo)?;
                            fts_insert(
                                &self.conn,
                                rowid,
                                &msg.text_plain,
                                &msg.text_stripped,
                                &jamo,
                            )?;
                            enqueue_wiki_classify(
                                &self.conn,
                                msg.chat_id,
                                msg.message_id,
                                &msg.text_plain,
                            )?;
                        }
                        outcome.updated += 1;
                    }
                }
            }
            Ok(outcome)
        })();
        match result {
            Ok(outcome) => {
                self.conn.execute("COMMIT")?;
                Ok(outcome)
            }
            Err(e) => {
                let _ = self.conn.execute("ROLLBACK");
                Err(e)
            }
        }
    }

    pub fn delete_messages(&self, refs: &[MessageRef]) -> Result<u64, sqlite::Error> {
        if refs.is_empty() {
            return Ok(0);
        }

        let _ = self.conn.execute("ROLLBACK");
        self.conn.execute("BEGIN")?;
        let result = (|| -> Result<u64, sqlite::Error> {
            let mut deleted = 0_u64;
            let mut affected_topics: std::collections::BTreeSet<i64> =
                std::collections::BTreeSet::new();
            for msg in refs {
                let prior = {
                    let mut stmt = self.conn.prepare(
                        "SELECT rowid, text_plain, text_stripped, text_jamo
                         FROM messages WHERE chat_id = ? AND message_id = ?",
                    )?;
                    stmt.bind((1, msg.chat_id))?;
                    stmt.bind((2, msg.message_id))?;
                    if let sqlite::State::Row = stmt.next()? {
                        Some((
                            stmt.read::<i64, _>(0)?,
                            stmt.read::<String, _>(1)?,
                            stmt.read::<String, _>(2)?,
                            stmt.read::<String, _>(3)?,
                        ))
                    } else {
                        None
                    }
                };
                let Some((rowid, text_plain, text_stripped, text_jamo)) = prior else {
                    continue;
                };

                fts_delete(&self.conn, rowid, &text_plain, &text_stripped, &text_jamo)?;

                let mut queue_stmt = self.conn.prepare(
                    "DELETE FROM wiki_classify_queue WHERE chat_id = ? AND message_id = ?",
                )?;
                queue_stmt.bind((1, msg.chat_id))?;
                queue_stmt.bind((2, msg.message_id))?;
                queue_stmt.next()?;

                let mut v2_queue_stmt = self.conn.prepare(
                    "DELETE FROM wiki_classify_queue_v2 WHERE chat_id = ? AND msg_id = ?",
                )?;
                v2_queue_stmt.bind((1, msg.chat_id))?;
                v2_queue_stmt.bind((2, msg.message_id))?;
                v2_queue_stmt.next()?;

                {
                    let mut find_stmt = self.conn.prepare(
                        "SELECT topic_id FROM wiki_topic_messages WHERE chat_id = ? AND message_id = ?",
                    )?;
                    find_stmt.bind((1, msg.chat_id))?;
                    find_stmt.bind((2, msg.message_id))?;
                    while let sqlite::State::Row = find_stmt.next()? {
                        affected_topics.insert(find_stmt.read::<i64, _>(0)?);
                    }
                }
                let mut topic_stmt = self.conn.prepare(
                    "DELETE FROM wiki_topic_messages WHERE chat_id = ? AND message_id = ?",
                )?;
                topic_stmt.bind((1, msg.chat_id))?;
                topic_stmt.bind((2, msg.message_id))?;
                topic_stmt.next()?;

                let mut msg_stmt = self.conn.prepare("DELETE FROM messages WHERE rowid = ?")?;
                msg_stmt.bind((1, rowid))?;
                msg_stmt.next()?;
                deleted += 1;
            }
            for topic_id in &affected_topics {
                self.conn.execute(format!(
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
                self.conn.execute(format!(
                    "DELETE FROM topic_stats_daily WHERE topic_id = {0};
                     INSERT INTO topic_stats_daily (topic_id, date, msg_count)
                     SELECT {0}, date(m.timestamp, 'unixepoch') AS d, COUNT(*)
                     FROM wiki_topic_messages tm
                     JOIN messages m ON m.chat_id = tm.chat_id AND m.message_id = tm.message_id
                     WHERE tm.topic_id = {0}
                     GROUP BY d;
                     DELETE FROM topic_channel_membership WHERE topic_id = {0};
                     INSERT OR IGNORE INTO topic_channel_membership (topic_id, date, chat_id)
                     SELECT {0}, date(m.timestamp, 'unixepoch'), m.chat_id
                     FROM wiki_topic_messages tm
                     JOIN messages m ON m.chat_id = tm.chat_id AND m.message_id = tm.message_id
                     WHERE tm.topic_id = {0};",
                    topic_id
                ))?;
                self.recompute_topic_trending_score(*topic_id)?;
            }
            Ok(deleted)
        })();
        match result {
            Ok(deleted) => {
                self.conn.execute("COMMIT")?;
                Ok(deleted)
            }
            Err(e) => {
                let _ = self.conn.execute("ROLLBACK");
                Err(e)
            }
        }
    }

    pub fn get_message(
        &self,
        chat_id: i64,
        message_id: i64,
    ) -> Result<Option<MessageRow>, sqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT message_id, chat_id, timestamp, text_plain, text_stripped, link, sender_id
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
                sender_id: stmt.read::<Option<i64>, _>(6)?.unwrap_or(0),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn search_messages_bm25(
        &self,
        fts_query: &str,
        scope_chat: Option<i64>,
        cursor: Option<&Cursor>,
        limit: usize,
    ) -> Result<Vec<MessageWithChat>, sqlite::Error> {
        let chat_clause = if scope_chat.is_some() {
            "AND m.chat_id = ?"
        } else {
            ""
        };
        let cursor_clause = if cursor.is_some() {
            "AND (r.rank > ?
                  OR (r.rank = ? AND m.timestamp < ?)
                  OR (r.rank = ? AND m.timestamp = ? AND m.chat_id > ?)
                  OR (r.rank = ? AND m.timestamp = ? AND m.chat_id = ? AND m.message_id > ?))"
        } else {
            ""
        };

        let sql = format!(
            "WITH ranked AS (
                 SELECT f.rowid,
                        bm25(messages_fts, 1.0, 0.7, 0.5)
                          - (m.timestamp / 86400.0) * 0.05 AS rank
                 FROM messages_fts f
                 JOIN messages m ON m.rowid = f.rowid
                 WHERE messages_fts MATCH ?
             )
             SELECT m.message_id, m.chat_id, m.timestamp, m.text_plain, m.link, c.title, r.rank
             FROM ranked r
             JOIN messages m ON m.rowid = r.rowid
             JOIN chats c ON m.chat_id = c.chat_id
             WHERE c.is_excluded = 0
             {chat_clause}
             {cursor_clause}
             ORDER BY r.rank ASC, m.timestamp DESC, m.chat_id ASC, m.message_id ASC
             LIMIT ?"
        );

        let mut stmt = self.conn.prepare(&sql)?;
        let mut bind_idx = 1;
        stmt.bind((bind_idx, fts_query))?;
        bind_idx += 1;
        if let Some(chat_id) = scope_chat {
            stmt.bind((bind_idx, chat_id))?;
            bind_idx += 1;
        }
        if let Some(c) = cursor {
            stmt.bind((bind_idx, c.rank))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.rank))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.timestamp))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.rank))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.timestamp))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.chat_id))?;
            bind_idx += 1;
            stmt.bind((bind_idx, c.rank))?;
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
                rank: stmt.read::<f64, _>(6)?,
            });
        }

        Ok(results)
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
                rank: 0.0,
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
                rank: 0.0,
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
            .map(|_| {
                "(m.text_plain LIKE '%' || ? || '%'
                  OR m.text_stripped LIKE '%' || ? || '%'
                  OR m.text_jamo LIKE '%' || ? || '%')"
                    .to_string()
            })
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
            for variant in like_variants(term) {
                stmt.bind((bind_idx, variant.as_str()))?;
                bind_idx += 1;
            }
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
                rank: 0.0,
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
            .map(|_| {
                "(m.text_plain LIKE '%' || ? || '%'
                  OR m.text_stripped LIKE '%' || ? || '%'
                  OR m.text_jamo LIKE '%' || ? || '%')"
                    .to_string()
            })
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
            for variant in like_variants(term) {
                stmt.bind((bind_idx, variant.as_str()))?;
                bind_idx += 1;
            }
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
                rank: 0.0,
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
            sender_id: 0,
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
        store
            .insert_messages_batch(std::slice::from_ref(&msg))
            .unwrap();
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

    #[test]
    fn insert_messages_batch_enqueues_v2_queue() {
        let store = Store::open_in_memory().unwrap();
        let msg = MessageRow {
            message_id: 42,
            chat_id: 1,
            timestamp: 1_700_000_000,
            text_plain: "first".into(),
            text_stripped: "first".into(),
            link: None,
            sender_id: 0,
        };
        store
            .insert_messages_batch(std::slice::from_ref(&msg))
            .unwrap();

        let mut stmt = store
            .conn()
            .prepare(
                "SELECT status, attempts, length(text_hash)
                 FROM wiki_classify_queue_v2 WHERE msg_id = 42 AND chat_id = 1",
            )
            .unwrap();
        assert!(matches!(stmt.next(), Ok(sqlite::State::Row)));
        assert_eq!(stmt.read::<String, _>(0).unwrap(), "pending");
        assert_eq!(stmt.read::<i64, _>(1).unwrap(), 0);
        assert_eq!(stmt.read::<i64, _>(2).unwrap(), 16);
        drop(stmt);

        // Same text re-ingested → noop, hash unchanged.
        let prior_hash: Vec<u8> = {
            let mut s = store
                .conn()
                .prepare("SELECT text_hash FROM wiki_classify_queue_v2 WHERE msg_id = 42")
                .unwrap();
            s.next().unwrap();
            s.read(0).unwrap()
        };
        store
            .insert_messages_batch(std::slice::from_ref(&msg))
            .unwrap();
        let same_hash: Vec<u8> = {
            let mut s = store
                .conn()
                .prepare("SELECT text_hash FROM wiki_classify_queue_v2 WHERE msg_id = 42")
                .unwrap();
            s.next().unwrap();
            s.read(0).unwrap()
        };
        assert_eq!(prior_hash, same_hash);

        // Edited text → hash changes, status stays pending.
        let edited = MessageRow {
            text_plain: "second".into(),
            text_stripped: "second".into(),
            ..msg
        };
        store.insert_messages_batch(&[edited]).unwrap();
        let new_hash: Vec<u8> = {
            let mut s = store
                .conn()
                .prepare("SELECT text_hash FROM wiki_classify_queue_v2 WHERE msg_id = 42")
                .unwrap();
            s.next().unwrap();
            s.read(0).unwrap()
        };
        assert_ne!(prior_hash, new_hash);
    }

    #[test]
    fn delete_messages_clears_v2_queue() {
        let store = Store::open_in_memory().unwrap();
        let msg = MessageRow {
            message_id: 7,
            chat_id: 1,
            timestamp: 1_700_000_000,
            text_plain: "foo".into(),
            text_stripped: "foo".into(),
            link: None,
            sender_id: 0,
        };
        store.insert_messages_batch(&[msg]).unwrap();
        store
            .delete_messages(&[MessageRef {
                chat_id: 1,
                message_id: 7,
            }])
            .unwrap();
        let mut stmt = store
            .conn()
            .prepare("SELECT COUNT(*) FROM wiki_classify_queue_v2 WHERE msg_id = 7")
            .unwrap();
        stmt.next().unwrap();
        assert_eq!(stmt.read::<i64, _>(0).unwrap(), 0);
    }

    #[test]
    fn insert_messages_batch_enqueues_classify() {
        let store = Store::open_in_memory().unwrap();
        let msgs = vec![
            MessageRow {
                message_id: 10,
                chat_id: 1,
                timestamp: 1_700_000_000,
                text_plain: "테스트 메시지".into(),
                text_stripped: "테스트메시지".into(),
                link: None,
                sender_id: 0,
            },
            MessageRow {
                message_id: 11,
                chat_id: 1,
                timestamp: 1_700_000_001,
                text_plain: "another".into(),
                text_stripped: "another".into(),
                link: None,
                sender_id: 0,
            },
        ];
        store.insert_messages_batch(&msgs).unwrap();

        let stats = store.get_queue_stats().unwrap();
        assert_eq!(stats.pending, 2);

        store.insert_messages_batch(&msgs).unwrap();
        let stats = store.get_queue_stats().unwrap();
        assert_eq!(stats.pending, 2);
    }
}
