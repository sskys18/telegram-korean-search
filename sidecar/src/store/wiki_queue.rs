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
}
