use super::Store;

impl Store {
    pub fn get_meta(&self, key: &str) -> Result<Option<String>, sqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM app_meta WHERE key = ?")?;
        stmt.bind((1, key))?;
        if let Ok(sqlite::State::Row) = stmt.next() {
            Ok(Some(stmt.read::<String, _>(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn.prepare(
            "INSERT INTO app_meta (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )?;
        stmt.bind((1, key))?;
        stmt.bind((2, value))?;
        stmt.next()?;
        Ok(())
    }

    pub fn delete_meta(&self, key: &str) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn.prepare("DELETE FROM app_meta WHERE key = ?")?;
        stmt.bind((1, key))?;
        stmt.next()?;
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
    fn test_set_and_get() {
        let store = test_store();
        store.set_meta("schema_version", "1").unwrap();
        assert_eq!(
            store.get_meta("schema_version").unwrap(),
            Some("1".to_string())
        );
    }

    #[test]
    fn test_update() {
        let store = test_store();
        store.set_meta("key", "v1").unwrap();
        store.set_meta("key", "v2").unwrap();
        assert_eq!(store.get_meta("key").unwrap(), Some("v2".to_string()));
    }

    #[test]
    fn test_get_nonexistent() {
        let store = test_store();
        assert!(store.get_meta("missing").unwrap().is_none());
    }

    #[test]
    fn test_delete() {
        let store = test_store();
        store.set_meta("key", "value").unwrap();
        store.delete_meta("key").unwrap();
        assert!(store.get_meta("key").unwrap().is_none());
    }
}
