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

    pub fn dequeue_classify_batch(&self, limit: usize) -> Result<Vec<QueueItem>, sqlite::Error> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        self.conn().execute("BEGIN IMMEDIATE")?;

        let selected = (|| -> Result<Vec<(i64, i64)>, sqlite::Error> {
            let mut stmt = self.conn().prepare(format!(
                "SELECT chat_id, message_id FROM wiki_classify_queue
                 WHERE status = 'pending'
                 ORDER BY created_at
                 LIMIT {}",
                limit
            ))?;
            let mut rows = Vec::new();
            while let sqlite::State::Row = stmt.next()? {
                rows.push((
                    stmt.read::<i64, _>("chat_id")?,
                    stmt.read::<i64, _>("message_id")?,
                ));
            }
            Ok(rows)
        })();

        let selected = match selected {
            Ok(rows) => rows,
            Err(err) => {
                let _ = self.conn().execute("ROLLBACK");
                return Err(err);
            }
        };

        if selected.is_empty() {
            self.conn().execute("COMMIT")?;
            return Ok(Vec::new());
        }

        let update_result = (|| -> Result<(), sqlite::Error> {
            let mut stmt = self.conn().prepare(
                "UPDATE wiki_classify_queue
                 SET status = 'processing', claimed_at = datetime('now'), attempts = attempts + 1
                 WHERE chat_id = ? AND message_id = ?",
            )?;
            for &(chat_id, message_id) in &selected {
                stmt.bind((1, chat_id))?;
                stmt.bind((2, message_id))?;
                stmt.next()?;
                stmt.reset()?;
            }
            Ok(())
        })();
        if let Err(err) = update_result {
            let _ = self.conn().execute("ROLLBACK");
            return Err(err);
        }

        let items = (|| -> Result<Vec<QueueItem>, sqlite::Error> {
            let mut stmt = self.conn().prepare(
                "SELECT chat_id, message_id, status, attempts, error, claimed_at
                 FROM wiki_classify_queue
                 WHERE chat_id = ? AND message_id = ?",
            )?;
            let mut items = Vec::with_capacity(selected.len());
            for &(chat_id, message_id) in &selected {
                stmt.bind((1, chat_id))?;
                stmt.bind((2, message_id))?;
                if let sqlite::State::Row = stmt.next()? {
                    items.push(QueueItem {
                        chat_id: stmt.read::<i64, _>("chat_id")?,
                        message_id: stmt.read::<i64, _>("message_id")?,
                        status: stmt.read::<String, _>("status")?,
                        attempts: stmt.read::<i64, _>("attempts")?,
                        error: stmt.read::<Option<String>, _>("error")?,
                        claimed_at: stmt.read::<Option<String>, _>("claimed_at")?,
                    });
                }
                stmt.reset()?;
            }
            Ok(items)
        })();
        let items = match items {
            Ok(items) => items,
            Err(err) => {
                let _ = self.conn().execute("ROLLBACK");
                return Err(err);
            }
        };

        self.conn().execute("COMMIT")?;
        Ok(items)
    }

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

    pub fn recover_stale_claims(&self) -> Result<usize, sqlite::Error> {
        self.conn().execute(
            "UPDATE wiki_classify_queue
             SET status = 'pending', claimed_at = NULL
             WHERE status = 'processing'
               AND claimed_at < datetime('now', '-5 minutes')",
        )?;
        Ok(self.conn().change_count())
    }

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

    pub fn clear_classify_queue(&self) -> Result<(), sqlite::Error> {
        self.conn().execute("DELETE FROM wiki_classify_queue")?;
        Ok(())
    }

    pub fn enqueue_all_messages(&self) -> Result<usize, sqlite::Error> {
        self.conn().execute(
            "INSERT OR IGNORE INTO wiki_classify_queue (chat_id, message_id)
             SELECT chat_id, message_id FROM messages",
        )?;
        Ok(self.conn().change_count())
    }
}

/// Row claimed for v2 classification.
#[derive(Debug, Clone)]
pub struct ClassifyV2Item {
    pub msg_id: i64,
    pub chat_id: i64,
    pub attempts: i64,
    pub hint: Option<String>,
    pub hint_page_id: Option<i64>,
    pub text_hash: Vec<u8>,
}

impl Store {
    /// Atomically claim up to `limit` rows from `wiki_classify_queue_v2`.
    pub fn claim_classify_v2_batch(
        &self,
        limit: usize,
    ) -> Result<Vec<ClassifyV2Item>, sqlite::Error> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let now = crate::wiki::norm::unix_now();
        self.conn().execute("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<Vec<ClassifyV2Item>, sqlite::Error> {
            let mut sel = self.conn().prepare(format!(
                "SELECT msg_id, chat_id, attempts, hint, hint_page_id, text_hash
                   FROM wiki_classify_queue_v2
                  WHERE status = 'pending'
                    AND (next_attempt_at IS NULL OR next_attempt_at <= ?)
                  ORDER BY enqueued_at
                  LIMIT {}",
                limit
            ))?;
            sel.bind((1, now))?;
            let mut rows = Vec::new();
            while let sqlite::State::Row = sel.next()? {
                rows.push(ClassifyV2Item {
                    msg_id: sel.read::<i64, _>("msg_id")?,
                    chat_id: sel.read::<i64, _>("chat_id")?,
                    attempts: sel.read::<i64, _>("attempts")?,
                    hint: sel.read::<Option<String>, _>("hint")?,
                    hint_page_id: sel.read::<Option<i64>, _>("hint_page_id")?,
                    text_hash: sel.read::<Vec<u8>, _>("text_hash")?,
                });
            }
            if rows.is_empty() {
                return Ok(rows);
            }

            let mut upd = self.conn().prepare(
                "UPDATE wiki_classify_queue_v2
                    SET status = 'processing', claimed_at = ?
                  WHERE msg_id = ? AND chat_id = ?",
            )?;
            for r in &rows {
                upd.bind((1, now))?;
                upd.bind((2, r.msg_id))?;
                upd.bind((3, r.chat_id))?;
                upd.next()?;
                upd.reset()?;
            }
            Ok(rows)
        })();
        match result {
            Ok(rows) => {
                self.conn().execute("COMMIT")?;
                Ok(rows)
            }
            Err(e) => {
                let _ = self.conn().execute("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Terminal success.
    pub fn mark_classify_v2_done(&self, msg_id: i64, chat_id: i64) -> Result<(), sqlite::Error> {
        let mut s = self.conn().prepare(
            "UPDATE wiki_classify_queue_v2
                SET status = 'done', attempts = attempts + 1,
                    claimed_at = NULL, last_error = NULL
              WHERE msg_id = ? AND chat_id = ?",
        )?;
        s.bind((1, msg_id))?;
        s.bind((2, chat_id))?;
        s.next()?;
        Ok(())
    }

    /// Bump attempts, back off, and transition to `failed` when exhausted.
    pub fn mark_classify_v2_retry(
        &self,
        msg_id: i64,
        chat_id: i64,
        err: &str,
        max_attempts: i64,
    ) -> Result<(), sqlite::Error> {
        let now = crate::wiki::norm::unix_now();
        let mut s = self.conn().prepare(
            "UPDATE wiki_classify_queue_v2
                SET attempts = attempts + 1,
                    last_error = ?,
                    claimed_at = NULL,
                    status = CASE WHEN attempts + 1 >= ? THEN 'failed' ELSE 'pending' END,
                    next_attempt_at = CASE
                        WHEN attempts + 1 >= ? THEN ?
                        ELSE ? + (30 * (1 << MIN(attempts + 1, 8)))
                    END
              WHERE msg_id = ? AND chat_id = ?",
        )?;
        s.bind((1, err))?;
        s.bind((2, max_attempts))?;
        s.bind((3, max_attempts))?;
        s.bind((4, now))?;
        s.bind((5, now))?;
        s.bind((6, msg_id))?;
        s.bind((7, chat_id))?;
        s.next()?;
        Ok(())
    }

    /// Re-queue with successor hint per spec §6.2 apply step.
    pub fn mark_classify_v2_successor_needed(
        &self,
        msg_id: i64,
        chat_id: i64,
        hint_page_id: i64,
    ) -> Result<(), sqlite::Error> {
        let now = crate::wiki::norm::unix_now();
        let mut s = self.conn().prepare(
            "UPDATE wiki_classify_queue_v2
                SET status = 'pending',
                    hint = 'successor_needed',
                    hint_page_id = ?,
                    attempts = attempts + 1,
                    claimed_at = NULL,
                    next_attempt_at = ? + 30
              WHERE msg_id = ? AND chat_id = ?",
        )?;
        s.bind((1, hint_page_id))?;
        s.bind((2, now))?;
        s.bind((3, msg_id))?;
        s.bind((4, chat_id))?;
        s.next()?;
        Ok(())
    }

    /// Reset rows that crashed mid-process.
    pub fn recover_stale_v2_claims(&self) -> Result<usize, sqlite::Error> {
        let cutoff = crate::wiki::norm::unix_now() - 300;
        let mut s = self.conn().prepare(
            "UPDATE wiki_classify_queue_v2
                SET status = 'pending', claimed_at = NULL
              WHERE status = 'processing' AND claimed_at < ?",
        )?;
        s.bind((1, cutoff))?;
        s.next()?;
        Ok(self.conn().change_count())
    }

    pub fn get_classify_v2_stats(&self) -> Result<QueueStats, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT
                SUM(CASE WHEN status='pending' THEN 1 ELSE 0 END) AS pending,
                SUM(CASE WHEN status='processing' THEN 1 ELSE 0 END) AS processing,
                SUM(CASE WHEN status='done' THEN 1 ELSE 0 END) AS done,
                SUM(CASE WHEN status='failed' THEN 1 ELSE 0 END) AS failed
             FROM wiki_classify_queue_v2",
        )?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(QueueStats {
                pending: stmt.read::<Option<i64>, _>("pending")?.unwrap_or(0),
                processing: stmt.read::<Option<i64>, _>("processing")?.unwrap_or(0),
                done: stmt.read::<Option<i64>, _>("done")?.unwrap_or(0),
                failed: stmt.read::<Option<i64>, _>("failed")?.unwrap_or(0),
                skipped: 0,
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
}

/// Row claimed for a v2 wiki page rewrite.
#[derive(Debug, Clone)]
pub struct RewriteQueueItem {
    pub page_id: i64,
    pub attempts: i64,
}

impl Store {
    /// Enqueue (or re-arm) a rewrite for `page_id`. Preserves an in-flight
    /// `processing` claim so a concurrent classify cannot wipe a worker's
    /// lease (and trick the worker into committing against stale rows).
    pub fn enqueue_rewrite(&self, page_id: i64) -> Result<(), sqlite::Error> {
        // Force enqueued_at strictly greater than both the previous
        // enqueued_at AND the current claimed_at. mark_rewrite_done detects
        // a "re-enqueue arrived during processing" via `enqueued_at >
        // claimed_at`; if both happen in the same wall-clock second the
        // naive `enqueued_at = now()` write would lose the signal and the
        // worker would mark the row done with the new evidence stranded.
        let now = crate::wiki::norm::unix_now();
        let mut s = self.conn().prepare(
            "INSERT INTO wiki_rewrite_queue
                (page_id, status, attempts, enqueued_at, next_attempt_at)
             VALUES (?, 'pending', 0, ?, ?)
             ON CONFLICT(page_id) DO UPDATE SET
                status = CASE
                    WHEN wiki_rewrite_queue.status = 'processing' THEN 'processing'
                    ELSE 'pending'
                END,
                enqueued_at = MAX(?, wiki_rewrite_queue.enqueued_at + 1,
                                  COALESCE(wiki_rewrite_queue.claimed_at, 0) + 1),
                next_attempt_at = CASE
                    WHEN wiki_rewrite_queue.status = 'processing' THEN wiki_rewrite_queue.next_attempt_at
                    ELSE ?
                END,
                attempts = CASE
                    WHEN wiki_rewrite_queue.status = 'done' THEN 0
                    WHEN wiki_rewrite_queue.status = 'failed' THEN 0
                    ELSE wiki_rewrite_queue.attempts
                END,
                last_error = CASE
                    WHEN wiki_rewrite_queue.status IN ('done','failed') THEN NULL
                    ELSE wiki_rewrite_queue.last_error
                END",
        )?;
        s.bind((1, page_id))?;
        s.bind((2, now))?;
        s.bind((3, now))?;
        s.bind((4, now))?;
        s.bind((5, now))?;
        s.next()?;
        Ok(())
    }

    /// Atomically claim up to `limit` rewrite rows.
    pub fn claim_rewrite_batch(
        &self,
        limit: usize,
    ) -> Result<Vec<RewriteQueueItem>, sqlite::Error> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let now = crate::wiki::norm::unix_now();
        self.conn().execute("BEGIN IMMEDIATE")?;
        let result = (|| -> Result<Vec<RewriteQueueItem>, sqlite::Error> {
            let mut sel = self.conn().prepare(format!(
                "SELECT page_id, attempts
                   FROM wiki_rewrite_queue
                  WHERE status = 'pending'
                    AND (next_attempt_at IS NULL OR next_attempt_at <= ?)
                  ORDER BY enqueued_at
                  LIMIT {}",
                limit
            ))?;
            sel.bind((1, now))?;
            let mut rows = Vec::new();
            while let sqlite::State::Row = sel.next()? {
                rows.push(RewriteQueueItem {
                    page_id: sel.read::<i64, _>("page_id")?,
                    attempts: sel.read::<i64, _>("attempts")?,
                });
            }
            if rows.is_empty() {
                return Ok(rows);
            }
            let mut upd = self.conn().prepare(
                "UPDATE wiki_rewrite_queue
                    SET status = 'processing', claimed_at = ?
                  WHERE page_id = ?",
            )?;
            for r in &rows {
                upd.bind((1, now))?;
                upd.bind((2, r.page_id))?;
                upd.next()?;
                upd.reset()?;
            }
            Ok(rows)
        })();
        match result {
            Ok(rows) => {
                self.conn().execute("COMMIT")?;
                Ok(rows)
            }
            Err(e) => {
                let _ = self.conn().execute("ROLLBACK");
                Err(e)
            }
        }
    }

    pub fn mark_rewrite_done(&self, page_id: i64) -> Result<(), sqlite::Error> {
        // If a re-enqueue arrived while this row was processing,
        // `enqueue_rewrite` left status='processing' (preserving the lease)
        // but advanced enqueued_at. Detect that here so the new evidence
        // doesn't get silently dropped: re-arm pending instead of done.
        let now = crate::wiki::norm::unix_now();
        let mut s = self.conn().prepare(
            "UPDATE wiki_rewrite_queue
                SET claimed_at = NULL,
                    last_error = NULL,
                    status = CASE
                        WHEN claimed_at IS NOT NULL AND enqueued_at > claimed_at THEN 'pending'
                        ELSE 'done'
                    END,
                    next_attempt_at = CASE
                        WHEN claimed_at IS NOT NULL AND enqueued_at > claimed_at THEN ?
                        ELSE next_attempt_at
                    END,
                    attempts = CASE
                        WHEN claimed_at IS NOT NULL AND enqueued_at > claimed_at THEN 0
                        ELSE attempts + 1
                    END
              WHERE page_id = ?",
        )?;
        s.bind((1, now))?;
        s.bind((2, page_id))?;
        s.next()?;
        Ok(())
    }

    pub fn mark_rewrite_retry(
        &self,
        page_id: i64,
        err: &str,
        max_attempts: i64,
    ) -> Result<(), sqlite::Error> {
        let now = crate::wiki::norm::unix_now();
        let mut s = self.conn().prepare(
            "UPDATE wiki_rewrite_queue
                SET attempts = attempts + 1,
                    last_error = ?,
                    claimed_at = NULL,
                    status = CASE WHEN attempts + 1 >= ? THEN 'failed' ELSE 'pending' END,
                    next_attempt_at = CASE
                        WHEN attempts + 1 >= ? THEN ?
                        ELSE ? + (60 * (1 << MIN(attempts + 1, 6)))
                    END
              WHERE page_id = ?",
        )?;
        s.bind((1, err))?;
        s.bind((2, max_attempts))?;
        s.bind((3, max_attempts))?;
        s.bind((4, now))?;
        s.bind((5, now))?;
        s.bind((6, page_id))?;
        s.next()?;
        Ok(())
    }

    pub fn recover_stale_rewrite_claims(&self) -> Result<usize, sqlite::Error> {
        let cutoff = crate::wiki::norm::unix_now() - 600;
        let mut s = self.conn().prepare(
            "UPDATE wiki_rewrite_queue
                SET status = 'pending', claimed_at = NULL
              WHERE status = 'processing' AND claimed_at < ?",
        )?;
        s.bind((1, cutoff))?;
        s.next()?;
        Ok(self.conn().change_count())
    }

    pub fn get_rewrite_stats(&self) -> Result<QueueStats, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT
                SUM(CASE WHEN status='pending' THEN 1 ELSE 0 END) AS pending,
                SUM(CASE WHEN status='processing' THEN 1 ELSE 0 END) AS processing,
                SUM(CASE WHEN status='done' THEN 1 ELSE 0 END) AS done,
                SUM(CASE WHEN status='failed' THEN 1 ELSE 0 END) AS failed
             FROM wiki_rewrite_queue",
        )?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(QueueStats {
                pending: stmt.read::<Option<i64>, _>("pending")?.unwrap_or(0),
                processing: stmt.read::<Option<i64>, _>("processing")?.unwrap_or(0),
                done: stmt.read::<Option<i64>, _>("done")?.unwrap_or(0),
                failed: stmt.read::<Option<i64>, _>("failed")?.unwrap_or(0),
                skipped: 0,
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
}

#[cfg(test)]
mod tests {
    use crate::store::message::MessageRow;
    use crate::store::Store;

    fn setup_store_with_messages() -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .conn()
            .execute("INSERT INTO chats (chat_id, title, chat_type) VALUES (1, 'Test', 'channel')")
            .unwrap();
        let msgs = vec![
            MessageRow {
                message_id: 1,
                chat_id: 1,
                timestamp: 1000,
                text_plain: "hello".to_string(),
                text_stripped: "hello".to_string(),
                link: None,
                sender_id: 0,
            },
            MessageRow {
                message_id: 2,
                chat_id: 1,
                timestamp: 2000,
                text_plain: "world".to_string(),
                text_stripped: "world".to_string(),
                link: None,
                sender_id: 0,
            },
        ];
        store.insert_messages_batch(&msgs).unwrap();
        // insert_messages_batch auto-enqueues for classification; clear
        // the queue so each test starts with a known empty state.
        store
            .conn()
            .execute("DELETE FROM wiki_classify_queue")
            .unwrap();
        store
            .conn()
            .execute("DELETE FROM wiki_classify_queue_v2")
            .unwrap();
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
        store.dequeue_classify_batch(1).unwrap();
        store.mark_queue_failed(1, 1, "timeout").unwrap();
        let stats = store.get_queue_stats().unwrap();
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

    #[test]
    fn v2_claim_and_mark_done() {
        let store = setup_store_with_messages();
        let now = crate::wiki::norm::unix_now();
        for mid in [1_i64, 2] {
            let mut s = store
                .conn()
                .prepare(
                    "INSERT INTO wiki_classify_queue_v2
                  (msg_id, chat_id, status, attempts, text_hash, enqueued_at, next_attempt_at)
                 VALUES (?, 1, 'pending', 0, X'00', ?, ?)",
                )
                .unwrap();
            s.bind((1, mid)).unwrap();
            s.bind((2, now)).unwrap();
            s.bind((3, now)).unwrap();
            s.next().unwrap();
        }

        let claimed = store.claim_classify_v2_batch(10).unwrap();
        assert_eq!(claimed.len(), 2);
        let stats = store.get_classify_v2_stats().unwrap();
        assert_eq!(stats.processing, 2);

        store.mark_classify_v2_done(1, 1).unwrap();
        let stats = store.get_classify_v2_stats().unwrap();
        assert_eq!(stats.done, 1);
        assert_eq!(stats.processing, 1);
    }

    #[test]
    fn v2_retry_backoff_then_failed() {
        let store = setup_store_with_messages();
        let now = crate::wiki::norm::unix_now();
        let mut s = store
            .conn()
            .prepare(
                "INSERT INTO wiki_classify_queue_v2
              (msg_id, chat_id, status, attempts, text_hash, enqueued_at, next_attempt_at)
             VALUES (1, 1, 'processing', 0, X'00', ?, ?)",
            )
            .unwrap();
        s.bind((1, now)).unwrap();
        s.bind((2, now)).unwrap();
        s.next().unwrap();

        store.mark_classify_v2_retry(1, 1, "err1", 3).unwrap();
        store.mark_classify_v2_retry(1, 1, "err2", 3).unwrap();
        store.mark_classify_v2_retry(1, 1, "err3", 3).unwrap();
        let stats = store.get_classify_v2_stats().unwrap();
        assert_eq!(stats.failed, 1);
    }

    #[test]
    fn v2_recover_stale_claims() {
        let store = setup_store_with_messages();
        let now = crate::wiki::norm::unix_now();
        let mut s = store.conn().prepare(
            "INSERT INTO wiki_classify_queue_v2
              (msg_id, chat_id, status, attempts, text_hash, enqueued_at, claimed_at, next_attempt_at)
             VALUES (1, 1, 'processing', 1, X'00', ?, ?, ?)",
        ).unwrap();
        s.bind((1, now - 1000)).unwrap();
        s.bind((2, now - 1000)).unwrap();
        s.bind((3, now - 1000)).unwrap();
        s.next().unwrap();

        let n = store.recover_stale_v2_claims().unwrap();
        assert_eq!(n, 1);
        let stats = store.get_classify_v2_stats().unwrap();
        assert_eq!(stats.pending, 1);
    }

    fn make_page(store: &Store, title: &str) -> i64 {
        store.conn().execute("BEGIN").unwrap();
        let p = store.dedup_or_insert_page_v2("topic", title, &[]).unwrap();
        store.conn().execute("COMMIT").unwrap();
        p.id
    }

    #[test]
    fn rewrite_enqueue_claim_done() {
        let store = setup_store_with_messages();
        let pid = make_page(&store, "Bitcoin ETF");
        store.enqueue_rewrite(pid).unwrap();

        let claimed = store.claim_rewrite_batch(10).unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].page_id, pid);
        let stats = store.get_rewrite_stats().unwrap();
        assert_eq!(stats.processing, 1);

        store.mark_rewrite_done(pid).unwrap();
        let stats = store.get_rewrite_stats().unwrap();
        assert_eq!(stats.done, 1);
        assert_eq!(stats.processing, 0);
    }

    #[test]
    fn rewrite_enqueue_preserves_processing_lease() {
        // Spec / advisor flag: a re-enqueue while a worker holds a
        // processing claim must not flip the row back to 'pending,
        // attempts=0' under the worker's feet.
        let store = setup_store_with_messages();
        let pid = make_page(&store, "ETH L2");
        store.enqueue_rewrite(pid).unwrap();
        let _ = store.claim_rewrite_batch(1).unwrap();

        // Concurrent classify path re-enqueues.
        store.enqueue_rewrite(pid).unwrap();

        let stats = store.get_rewrite_stats().unwrap();
        assert_eq!(stats.processing, 1);
        assert_eq!(stats.pending, 0);
    }

    #[test]
    fn rewrite_done_reenqueues_when_concurrent_enqueue_arrived() {
        // Worker claims a row, classify path then re-enqueues during
        // processing (preserving the lease), worker calls mark_rewrite_done.
        // The new evidence MUST NOT be dropped — row should re-arm pending.
        let store = setup_store_with_messages();
        let pid = make_page(&store, "Reentrant");
        store.enqueue_rewrite(pid).unwrap();
        let claimed = store.claim_rewrite_batch(1).unwrap();
        assert_eq!(claimed.len(), 1);

        // claim sets claimed_at = now. Force enqueued_at > claimed_at to
        // simulate a re-enqueue that landed *after* the claim. (In wall
        // time the live path also bumps enqueued_at via the upsert; the
        // ordering matters more than the magnitude.)
        store
            .conn()
            .execute(format!(
                "UPDATE wiki_rewrite_queue
                    SET enqueued_at = claimed_at + 5
                  WHERE page_id = {pid}"
            ))
            .unwrap();

        store.mark_rewrite_done(pid).unwrap();
        let stats = store.get_rewrite_stats().unwrap();
        assert_eq!(
            stats.pending, 1,
            "concurrent re-enqueue must re-arm pending"
        );
        assert_eq!(stats.done, 0);
    }

    #[test]
    fn rewrite_done_reenqueue_survives_same_second_clock() {
        // Real-world: claim and re-enqueue often land in the same unix
        // second. The upsert must force enqueued_at strictly above
        // claimed_at so mark_done's `>` detection still fires.
        let store = setup_store_with_messages();
        let pid = make_page(&store, "SameSec");
        store.enqueue_rewrite(pid).unwrap();
        let _ = store.claim_rewrite_batch(1).unwrap();
        // Don't fudge timestamps — let real wall clock decide.
        store.enqueue_rewrite(pid).unwrap();
        store.mark_rewrite_done(pid).unwrap();
        let stats = store.get_rewrite_stats().unwrap();
        assert_eq!(
            stats.pending, 1,
            "same-second re-enqueue must still re-arm pending"
        );
        assert_eq!(stats.done, 0);
    }

    #[test]
    fn rewrite_done_marks_done_when_no_concurrent_enqueue() {
        let store = setup_store_with_messages();
        let pid = make_page(&store, "Plain");
        store.enqueue_rewrite(pid).unwrap();
        let _ = store.claim_rewrite_batch(1).unwrap();
        store.mark_rewrite_done(pid).unwrap();
        let stats = store.get_rewrite_stats().unwrap();
        assert_eq!(stats.done, 1);
        assert_eq!(stats.pending, 0);
    }

    #[test]
    fn rewrite_retry_then_failed() {
        let store = setup_store_with_messages();
        let pid = make_page(&store, "DeFi");
        store.enqueue_rewrite(pid).unwrap();
        let _ = store.claim_rewrite_batch(1).unwrap();

        store.mark_rewrite_retry(pid, "err1", 3).unwrap();
        store.mark_rewrite_retry(pid, "err2", 3).unwrap();
        store.mark_rewrite_retry(pid, "err3", 3).unwrap();
        let stats = store.get_rewrite_stats().unwrap();
        assert_eq!(stats.failed, 1);
    }

    #[test]
    fn rewrite_recover_stale() {
        let store = setup_store_with_messages();
        let pid = make_page(&store, "Memecoin");
        let now = crate::wiki::norm::unix_now();
        let mut s = store
            .conn()
            .prepare(
                "INSERT INTO wiki_rewrite_queue
              (page_id, status, attempts, enqueued_at, claimed_at, next_attempt_at)
             VALUES (?, 'processing', 1, ?, ?, ?)",
            )
            .unwrap();
        s.bind((1, pid)).unwrap();
        s.bind((2, now - 1000)).unwrap();
        s.bind((3, now - 1000)).unwrap();
        s.bind((4, now - 1000)).unwrap();
        s.next().unwrap();

        let n = store.recover_stale_rewrite_claims().unwrap();
        assert_eq!(n, 1);
        let stats = store.get_rewrite_stats().unwrap();
        assert_eq!(stats.pending, 1);
    }
}
