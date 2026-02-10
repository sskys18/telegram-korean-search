use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRow {
    pub chat_id: i64,
    pub title: String,
    pub chat_type: String,
    pub username: Option<String>,
    pub access_hash: Option<i64>,
    pub is_excluded: bool,
}

impl Store {
    pub fn upsert_chat(&self, chat: &ChatRow) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "INSERT INTO chats (chat_id, title, chat_type, username, access_hash, is_excluded)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(chat_id) DO UPDATE SET
                title = excluded.title,
                chat_type = excluded.chat_type,
                username = excluded.username,
                access_hash = excluded.access_hash",
            params![
                chat.chat_id,
                chat.title,
                chat.chat_type,
                chat.username,
                chat.access_hash,
                chat.is_excluded as i32,
            ],
        )?;
        Ok(())
    }

    pub fn get_chat(&self, chat_id: i64) -> Result<Option<ChatRow>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT chat_id, title, chat_type, username, access_hash, is_excluded
             FROM chats WHERE chat_id = ?1",
        )?;
        let mut rows = stmt.query_map(params![chat_id], |row| {
            Ok(ChatRow {
                chat_id: row.get(0)?,
                title: row.get(1)?,
                chat_type: row.get(2)?,
                username: row.get(3)?,
                access_hash: row.get(4)?,
                is_excluded: row.get::<_, i32>(5)? != 0,
            })
        })?;
        match rows.next() {
            Some(Ok(chat)) => Ok(Some(chat)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    pub fn get_active_chats(&self) -> Result<Vec<ChatRow>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT chat_id, title, chat_type, username, access_hash, is_excluded
             FROM chats WHERE is_excluded = 0 ORDER BY title",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ChatRow {
                chat_id: row.get(0)?,
                title: row.get(1)?,
                chat_type: row.get(2)?,
                username: row.get(3)?,
                access_hash: row.get(4)?,
                is_excluded: row.get::<_, i32>(5)? != 0,
            })
        })?;
        rows.collect()
    }

    pub fn get_all_chats(&self) -> Result<Vec<ChatRow>, rusqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT chat_id, title, chat_type, username, access_hash, is_excluded
             FROM chats ORDER BY title",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ChatRow {
                chat_id: row.get(0)?,
                title: row.get(1)?,
                chat_type: row.get(2)?,
                username: row.get(3)?,
                access_hash: row.get(4)?,
                is_excluded: row.get::<_, i32>(5)? != 0,
            })
        })?;
        rows.collect()
    }

    pub fn set_chat_excluded(&self, chat_id: i64, excluded: bool) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "UPDATE chats SET is_excluded = ?1 WHERE chat_id = ?2",
            params![excluded as i32, chat_id],
        )?;
        Ok(())
    }

    pub fn chat_count(&self) -> Result<i64, rusqlite::Error> {
        self.conn
            .query_row("SELECT COUNT(*) FROM chats", [], |row| row.get(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn sample_chat(id: i64) -> ChatRow {
        ChatRow {
            chat_id: id,
            title: format!("Chat {}", id),
            chat_type: "supergroup".to_string(),
            username: Some(format!("chat_{}", id)),
            access_hash: Some(12345),
            is_excluded: false,
        }
    }

    #[test]
    fn test_upsert_and_get() {
        let store = test_store();
        let chat = sample_chat(100);
        store.upsert_chat(&chat).unwrap();

        let fetched = store.get_chat(100).unwrap().unwrap();
        assert_eq!(fetched.title, "Chat 100");
        assert_eq!(fetched.chat_type, "supergroup");
        assert_eq!(fetched.username, Some("chat_100".to_string()));
    }

    #[test]
    fn test_upsert_updates_existing() {
        let store = test_store();
        let mut chat = sample_chat(100);
        store.upsert_chat(&chat).unwrap();

        chat.title = "Updated Title".to_string();
        store.upsert_chat(&chat).unwrap();

        let fetched = store.get_chat(100).unwrap().unwrap();
        assert_eq!(fetched.title, "Updated Title");
    }

    #[test]
    fn test_get_nonexistent() {
        let store = test_store();
        assert!(store.get_chat(999).unwrap().is_none());
    }

    #[test]
    fn test_active_chats_excludes_excluded() {
        let store = test_store();
        store.upsert_chat(&sample_chat(1)).unwrap();
        store.upsert_chat(&sample_chat(2)).unwrap();
        store.set_chat_excluded(2, true).unwrap();

        let active = store.get_active_chats().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].chat_id, 1);
    }

    #[test]
    fn test_chat_count() {
        let store = test_store();
        assert_eq!(store.chat_count().unwrap(), 0);
        store.upsert_chat(&sample_chat(1)).unwrap();
        store.upsert_chat(&sample_chat(2)).unwrap();
        assert_eq!(store.chat_count().unwrap(), 2);
    }
}
