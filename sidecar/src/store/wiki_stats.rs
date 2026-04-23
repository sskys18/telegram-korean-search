use super::Store;

impl Store {
    pub fn record_topic_stat(
        &self,
        topic_id: i64,
        message_timestamp: i64,
        chat_id: i64,
    ) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "INSERT INTO topic_stats_daily (topic_id, date, msg_count)
             VALUES (?, date(?, 'unixepoch'), 1)
             ON CONFLICT(topic_id, date) DO UPDATE SET msg_count = msg_count + 1",
        )?;
        stmt.bind((1, topic_id))?;
        stmt.bind((2, message_timestamp))?;
        stmt.next()?;

        let mut stmt2 = self.conn().prepare(
            "INSERT OR IGNORE INTO topic_channel_membership (topic_id, date, chat_id)
             VALUES (?, date(?, 'unixepoch'), ?)",
        )?;
        stmt2.bind((1, topic_id))?;
        stmt2.bind((2, message_timestamp))?;
        stmt2.bind((3, chat_id))?;
        stmt2.next()?;

        Ok(())
    }

    pub fn get_topic_msg_count_days(&self, topic_id: i64, days: i64) -> Result<i64, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT COALESCE(SUM(msg_count), 0) FROM topic_stats_daily
             WHERE topic_id = ? AND date >= date('now', ? || ' days')",
        )?;
        let modifier = format!("-{}", days);
        stmt.bind((1, topic_id))?;
        stmt.bind((2, modifier.as_str()))?;
        stmt.next()?;
        stmt.read::<i64, _>(0)
    }

    pub fn get_topic_channel_count_days(
        &self,
        topic_id: i64,
        days: i64,
    ) -> Result<i64, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT COUNT(DISTINCT chat_id) FROM topic_channel_membership
             WHERE topic_id = ? AND date >= date('now', ? || ' days')",
        )?;
        let modifier = format!("-{}", days);
        stmt.bind((1, topic_id))?;
        stmt.bind((2, modifier.as_str()))?;
        stmt.next()?;
        stmt.read::<i64, _>(0)
    }

    pub fn get_total_active_channels(&self) -> Result<i64, sqlite::Error> {
        let mut stmt = self
            .conn()
            .prepare("SELECT COUNT(*) FROM chats WHERE is_excluded = 0")?;
        stmt.next()?;
        stmt.read::<i64, _>(0)
    }

    pub fn get_active_topic_ids(&self, days: i64) -> Result<Vec<i64>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT DISTINCT topic_id FROM topic_stats_daily
             WHERE date >= date('now', ? || ' days')",
        )?;
        let modifier = format!("-{}", days);
        stmt.bind((1, modifier.as_str()))?;
        let mut ids = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            ids.push(stmt.read::<i64, _>(0)?);
        }
        Ok(ids)
    }

    pub fn clear_wiki_stats(&self) -> Result<(), sqlite::Error> {
        self.conn().execute("DELETE FROM topic_stats_daily")?;
        self.conn()
            .execute("DELETE FROM topic_channel_membership")?;
        Ok(())
    }

    /// Count of distinct topics and of total topic-message links whose
    /// underlying message has `timestamp >= since_ts`. The
    /// `wiki_topic_messages` table does not store a timestamp, so we
    /// join to `messages` for the filter.
    pub fn wiki_counts_since(&self, since_ts: i64) -> Result<(i64, i64), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT
                (SELECT COUNT(DISTINCT wtm.topic_id)
                 FROM wiki_topic_messages wtm
                 JOIN messages m
                   ON m.chat_id = wtm.chat_id AND m.message_id = wtm.message_id
                 WHERE m.timestamp >= ?1),
                (SELECT COUNT(*)
                 FROM wiki_topic_messages wtm
                 JOIN messages m
                   ON m.chat_id = wtm.chat_id AND m.message_id = wtm.message_id
                 WHERE m.timestamp >= ?1)
            ",
        )?;
        stmt.bind((1, since_ts))?;
        stmt.next()?;
        Ok((stmt.read::<i64, _>(0)?, stmt.read::<i64, _>(1)?))
    }
}

#[cfg(test)]
mod tests {
    use crate::store::Store;

    #[test]
    fn test_record_and_query_stats() {
        let store = Store::open_in_memory().unwrap();
        let cat_id = store.resolve_category("Test", None).unwrap();
        store
            .conn()
            .execute(format!(
                "INSERT INTO wiki_topics (title, category_id) VALUES ('Test', {})",
                cat_id
            ))
            .unwrap();
        let topic_id = 1;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        store.record_topic_stat(topic_id, now, 100).unwrap();
        store.record_topic_stat(topic_id, now, 200).unwrap();
        store.record_topic_stat(topic_id, now, 100).unwrap();

        let msg_count = store.get_topic_msg_count_days(topic_id, 1).unwrap();
        assert_eq!(msg_count, 3);

        let chan_count = store.get_topic_channel_count_days(topic_id, 1).unwrap();
        assert_eq!(chan_count, 2);
    }

    #[test]
    fn test_total_active_channels() {
        let store = Store::open_in_memory().unwrap();
        store
            .conn()
            .execute("INSERT INTO chats (chat_id, title, chat_type) VALUES (1, 'A', 'channel')")
            .unwrap();
        store
            .conn()
            .execute(
                "INSERT INTO chats (chat_id, title, chat_type, is_excluded) VALUES (2, 'B', 'channel', 1)",
            )
            .unwrap();
        let count = store.get_total_active_channels().unwrap();
        assert_eq!(count, 1);
    }

    fn seed_chat(store: &Store, chat_id: i64) {
        store
            .conn()
            .execute(format!(
                "INSERT INTO chats (chat_id, title, chat_type) VALUES ({}, 'c', 'channel')",
                chat_id
            ))
            .unwrap();
    }

    fn seed_message(store: &Store, chat_id: i64, message_id: i64, ts: i64) {
        store
            .conn()
            .execute(format!(
                "INSERT INTO messages (message_id, chat_id, timestamp, text_plain, text_stripped)
                 VALUES ({}, {}, {}, 'x', 'x')",
                message_id, chat_id, ts
            ))
            .unwrap();
    }

    fn seed_topic(store: &Store, title: &str) -> i64 {
        let cat = store.resolve_category("Test", None).unwrap();
        store
            .conn()
            .execute(format!(
                "INSERT INTO wiki_topics (title, category_id) VALUES ('{}', {})",
                title, cat
            ))
            .unwrap();
        let mut stmt = store.conn().prepare("SELECT last_insert_rowid()").unwrap();
        stmt.next().unwrap();
        stmt.read::<i64, _>(0).unwrap()
    }

    fn link(store: &Store, topic_id: i64, chat_id: i64, message_id: i64) {
        store
            .conn()
            .execute(format!(
                "INSERT INTO wiki_topic_messages (topic_id, chat_id, message_id) VALUES ({}, {}, {})",
                topic_id, chat_id, message_id
            ))
            .unwrap();
    }

    #[test]
    fn test_wiki_counts_since_filters_and_distinct_topics() {
        let store = Store::open_in_memory().unwrap();
        seed_chat(&store, 1);
        let t1 = seed_topic(&store, "Alpha");
        let t2 = seed_topic(&store, "Beta");

        // Two old messages (should be filtered out), three recent.
        seed_message(&store, 1, 10, 100);
        seed_message(&store, 1, 11, 200);
        seed_message(&store, 1, 20, 1_000);
        seed_message(&store, 1, 21, 1_100);
        seed_message(&store, 1, 22, 1_200);

        link(&store, t1, 1, 10); // old
        link(&store, t1, 1, 20); // recent, t1
        link(&store, t1, 1, 21); // recent, t1 again
        link(&store, t2, 1, 22); // recent, t2

        let (topics, msgs) = store.wiki_counts_since(1_000).unwrap();
        assert_eq!(topics, 2);
        assert_eq!(msgs, 3);
    }

    #[test]
    fn test_wiki_counts_since_empty() {
        let store = Store::open_in_memory().unwrap();
        let (topics, msgs) = store.wiki_counts_since(0).unwrap();
        assert_eq!(topics, 0);
        assert_eq!(msgs, 0);
    }
}
