use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStateRow {
    pub chat_id: i64,
    pub last_message_id: i64,
    pub oldest_message_id: Option<i64>,
    pub initial_done: bool,
    pub last_sync_at: Option<String>,
}

impl Store {
    pub fn get_sync_state(&self, chat_id: i64) -> Result<Option<SyncStateRow>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT chat_id, last_message_id, oldest_message_id, initial_done, last_sync_at
             FROM sync_state WHERE chat_id = ?1",
        )?;
        let mut rows = stmt.query_map(params![chat_id], |row| {
            Ok(SyncStateRow {
                chat_id: row.get(0)?,
                last_message_id: row.get(1)?,
                oldest_message_id: row.get(2)?,
                initial_done: row.get::<_, i32>(3)? != 0,
                last_sync_at: row.get(4)?,
            })
        })?;
        match rows.next() {
            Some(Ok(state)) => Ok(Some(state)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    pub fn upsert_sync_state(&self, state: &SyncStateRow) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "INSERT INTO sync_state (chat_id, last_message_id, oldest_message_id, initial_done, last_sync_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(chat_id) DO UPDATE SET
                last_message_id = excluded.last_message_id,
                oldest_message_id = excluded.oldest_message_id,
                initial_done = excluded.initial_done,
                last_sync_at = excluded.last_sync_at",
            params![
                state.chat_id,
                state.last_message_id,
                state.oldest_message_id,
                state.initial_done as i32,
                state.last_sync_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_last_message_id(
        &self,
        chat_id: i64,
        last_message_id: i64,
        last_sync_at: &str,
    ) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "UPDATE sync_state SET last_message_id = ?1, last_sync_at = ?2 WHERE chat_id = ?3",
            params![last_message_id, last_sync_at, chat_id],
        )?;
        Ok(())
    }

    pub fn update_oldest_message_id(
        &self,
        chat_id: i64,
        oldest_message_id: i64,
    ) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "UPDATE sync_state SET oldest_message_id = ?1 WHERE chat_id = ?2",
            params![oldest_message_id, chat_id],
        )?;
        Ok(())
    }

    pub fn mark_initial_done(&self, chat_id: i64) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "UPDATE sync_state SET initial_done = 1 WHERE chat_id = ?1",
            params![chat_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::chat::ChatRow;

    fn test_store() -> Store {
        let store = Store::open_in_memory().unwrap();
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
    }

    #[test]
    fn test_upsert_and_get() {
        let store = test_store();
        let state = SyncStateRow {
            chat_id: 1,
            last_message_id: 500,
            oldest_message_id: Some(100),
            initial_done: false,
            last_sync_at: Some("2025-02-10T12:00:00Z".to_string()),
        };
        store.upsert_sync_state(&state).unwrap();

        let fetched = store.get_sync_state(1).unwrap().unwrap();
        assert_eq!(fetched.last_message_id, 500);
        assert_eq!(fetched.oldest_message_id, Some(100));
        assert!(!fetched.initial_done);
    }

    #[test]
    fn test_mark_initial_done() {
        let store = test_store();
        let state = SyncStateRow {
            chat_id: 1,
            last_message_id: 0,
            oldest_message_id: None,
            initial_done: false,
            last_sync_at: None,
        };
        store.upsert_sync_state(&state).unwrap();
        store.mark_initial_done(1).unwrap();

        let fetched = store.get_sync_state(1).unwrap().unwrap();
        assert!(fetched.initial_done);
    }

    #[test]
    fn test_get_nonexistent() {
        let store = test_store();
        assert!(store.get_sync_state(999).unwrap().is_none());
    }
}
