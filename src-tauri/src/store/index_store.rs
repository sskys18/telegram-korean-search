use rusqlite::params;

use super::Store;

impl Store {
    pub fn insert_or_get_term(
        &self,
        term: &str,
        source_type: &str,
    ) -> Result<i64, rusqlite::Error> {
        self.conn.execute(
            "INSERT OR IGNORE INTO index_terms (term, source_type) VALUES (?1, ?2)",
            params![term, source_type],
        )?;
        let term_id: i64 = self.conn.query_row(
            "SELECT term_id FROM index_terms WHERE term = ?1",
            params![term],
            |row| row.get(0),
        )?;
        Ok(term_id)
    }

    pub fn insert_posting(
        &self,
        term_id: i64,
        chat_id: i64,
        message_id: i64,
        timestamp: i64,
    ) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "INSERT OR IGNORE INTO postings (term_id, chat_id, message_id, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
            params![term_id, chat_id, message_id, timestamp],
        )?;
        Ok(())
    }

    pub fn get_term_ids(&self, term: &str) -> Result<Vec<i64>, rusqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT term_id FROM index_terms WHERE term = ?1")?;
        let rows = stmt.query_map(params![term], |row| row.get(0))?;
        rows.collect()
    }

    pub fn get_term_ids_by_type(
        &self,
        term: &str,
        source_type: &str,
    ) -> Result<Vec<i64>, rusqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT term_id FROM index_terms WHERE term = ?1 AND source_type = ?2")?;
        let rows = stmt.query_map(params![term, source_type], |row| row.get(0))?;
        rows.collect()
    }

    pub fn term_count(&self) -> Result<i64, rusqlite::Error> {
        self.conn
            .query_row("SELECT COUNT(*) FROM index_terms", [], |row| row.get(0))
    }

    pub fn posting_count(&self) -> Result<i64, rusqlite::Error> {
        self.conn
            .query_row("SELECT COUNT(*) FROM postings", [], |row| row.get(0))
    }

    pub fn clear_index(&self) -> Result<(), rusqlite::Error> {
        self.conn.execute_batch(
            "DELETE FROM postings;
             DELETE FROM index_terms;",
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn test_insert_and_get_term() {
        let store = test_store();
        let id1 = store.insert_or_get_term("hello", "token").unwrap();
        let id2 = store.insert_or_get_term("hello", "token").unwrap();
        assert_eq!(id1, id2); // same term returns same ID

        let id3 = store.insert_or_get_term("world", "ngram").unwrap();
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_get_term_ids() {
        let store = test_store();
        store.insert_or_get_term("삼성", "token").unwrap();
        store.insert_or_get_term("삼성", "ngram").unwrap();

        // Same term text but different source_type → only 1 row (UNIQUE on term)
        let ids = store.get_term_ids("삼성").unwrap();
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn test_insert_posting() {
        let store = test_store();
        // Need a chat and message first
        use crate::store::chat::ChatRow;
        use crate::store::message::{strip_whitespace, MessageRow};

        store
            .upsert_chat(&ChatRow {
                chat_id: 1,
                title: "Test".to_string(),
                chat_type: "supergroup".to_string(),
                username: None,
                access_hash: None,
                is_excluded: false,
            })
            .unwrap();

        store
            .insert_messages_batch(&[MessageRow {
                message_id: 10,
                chat_id: 1,
                timestamp: 1000,
                text_plain: "test".to_string(),
                text_stripped: strip_whitespace("test"),
                link: None,
            }])
            .unwrap();

        let term_id = store.insert_or_get_term("test", "token").unwrap();
        store.insert_posting(term_id, 1, 10, 1000).unwrap();

        assert_eq!(store.posting_count().unwrap(), 1);
    }

    #[test]
    fn test_clear_index() {
        let store = test_store();
        store.insert_or_get_term("hello", "token").unwrap();
        assert_eq!(store.term_count().unwrap(), 1);

        store.clear_index().unwrap();
        assert_eq!(store.term_count().unwrap(), 0);
        assert_eq!(store.posting_count().unwrap(), 0);
    }
}
