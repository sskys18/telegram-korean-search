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
    pub fn upsert_chat(&self, chat: &ChatRow) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn.prepare(
            "INSERT INTO chats (chat_id, title, chat_type, username, access_hash, is_excluded)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(chat_id) DO UPDATE SET
                title = excluded.title,
                chat_type = excluded.chat_type,
                username = excluded.username,
                access_hash = excluded.access_hash",
        )?;
        stmt.bind((1, chat.chat_id))?;
        stmt.bind((2, chat.title.as_str()))?;
        stmt.bind((3, chat.chat_type.as_str()))?;
        match &chat.username {
            Some(u) => stmt.bind((4, u.as_str()))?,
            None => stmt.bind((4, sqlite::Value::Null))?,
        };
        match chat.access_hash {
            Some(h) => stmt.bind((5, h))?,
            None => stmt.bind((5, sqlite::Value::Null))?,
        };
        stmt.bind((6, chat.is_excluded as i64))?;
        stmt.next()?;
        Ok(())
    }

    pub fn get_chat(&self, chat_id: i64) -> Result<Option<ChatRow>, sqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT chat_id, title, chat_type, username, access_hash, is_excluded
             FROM chats WHERE chat_id = ?",
        )?;
        stmt.bind((1, chat_id))?;
        if let Ok(sqlite::State::Row) = stmt.next() {
            Ok(Some(read_chat_row(&stmt)?))
        } else {
            Ok(None)
        }
    }

    pub fn get_active_chats(&self) -> Result<Vec<ChatRow>, sqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT chat_id, title, chat_type, username, access_hash, is_excluded
             FROM chats WHERE is_excluded = 0 ORDER BY title",
        )?;
        let mut results = Vec::new();
        while let Ok(sqlite::State::Row) = stmt.next() {
            results.push(read_chat_row(&stmt)?);
        }
        Ok(results)
    }

    pub fn get_all_chats(&self) -> Result<Vec<ChatRow>, sqlite::Error> {
        let mut stmt = self.conn.prepare(
            "SELECT chat_id, title, chat_type, username, access_hash, is_excluded
             FROM chats ORDER BY title",
        )?;
        let mut results = Vec::new();
        while let Ok(sqlite::State::Row) = stmt.next() {
            results.push(read_chat_row(&stmt)?);
        }
        Ok(results)
    }

    pub fn set_chat_excluded(&self, chat_id: i64, excluded: bool) -> Result<(), sqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("UPDATE chats SET is_excluded = ? WHERE chat_id = ?")?;
        stmt.bind((1, excluded as i64))?;
        stmt.bind((2, chat_id))?;
        stmt.next()?;
        Ok(())
    }

    pub fn chat_count(&self) -> Result<i64, sqlite::Error> {
        let mut stmt = self.conn.prepare("SELECT COUNT(*) FROM chats")?;
        stmt.next()?;
        stmt.read::<i64, _>(0)
    }
}

fn read_chat_row(stmt: &sqlite::Statement) -> Result<ChatRow, sqlite::Error> {
    Ok(ChatRow {
        chat_id: stmt.read::<i64, _>("chat_id")?,
        title: stmt.read::<String, _>("title")?,
        chat_type: stmt.read::<String, _>("chat_type")?,
        username: stmt.read::<Option<String>, _>("username")?,
        access_hash: stmt.read::<Option<i64>, _>("access_hash")?,
        is_excluded: stmt.read::<i64, _>("is_excluded")? != 0,
    })
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
