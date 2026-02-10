use super::Store;

impl Store {
    pub fn insert_or_get_term(&self, term: &str, source_type: &str) -> Result<i64, sqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("INSERT OR IGNORE INTO index_terms (term, source_type) VALUES (?, ?)")?;
        stmt.bind((1, term))?;
        stmt.bind((2, source_type))?;
        stmt.next()?;

        let mut stmt2 = self
            .conn
            .prepare("SELECT term_id FROM index_terms WHERE term = ?")?;
        stmt2.bind((1, term))?;
        stmt2.next()?;
        stmt2.read::<i64, _>(0)
    }

    pub fn insert_posting(
        &self,
        term_id: i64,
        chat_id: i64,
        message_id: i64,
        timestamp: i64,
    ) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn.prepare(
            "INSERT OR IGNORE INTO postings (term_id, chat_id, message_id, timestamp)
             VALUES (?, ?, ?, ?)",
        )?;
        stmt.bind((1, term_id))?;
        stmt.bind((2, chat_id))?;
        stmt.bind((3, message_id))?;
        stmt.bind((4, timestamp))?;
        stmt.next()?;
        Ok(())
    }

    pub fn get_term_ids(&self, term: &str) -> Result<Vec<i64>, sqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT term_id FROM index_terms WHERE term = ?")?;
        stmt.bind((1, term))?;
        let mut results = Vec::new();
        while let Ok(sqlite::State::Row) = stmt.next() {
            results.push(stmt.read::<i64, _>(0)?);
        }
        Ok(results)
    }

    pub fn get_term_ids_by_type(
        &self,
        term: &str,
        source_type: &str,
    ) -> Result<Vec<i64>, sqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT term_id FROM index_terms WHERE term = ? AND source_type = ?")?;
        stmt.bind((1, term))?;
        stmt.bind((2, source_type))?;
        let mut results = Vec::new();
        while let Ok(sqlite::State::Row) = stmt.next() {
            results.push(stmt.read::<i64, _>(0)?);
        }
        Ok(results)
    }

    pub fn term_count(&self) -> Result<i64, sqlite::Error> {
        let mut stmt = self.conn.prepare("SELECT COUNT(*) FROM index_terms")?;
        stmt.next()?;
        stmt.read::<i64, _>(0)
    }

    pub fn posting_count(&self) -> Result<i64, sqlite::Error> {
        let mut stmt = self.conn.prepare("SELECT COUNT(*) FROM postings")?;
        stmt.next()?;
        stmt.read::<i64, _>(0)
    }

    pub fn clear_index(&self) -> Result<(), sqlite::Error> {
        self.conn.execute(
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
        assert_eq!(id1, id2);

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
