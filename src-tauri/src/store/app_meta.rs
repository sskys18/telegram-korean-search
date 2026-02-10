use rusqlite::params;

use super::Store;

impl Store {
    pub fn get_meta(&self, key: &str) -> Result<Option<String>, rusqlite::Error> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM app_meta WHERE key = ?1")?;
        let mut rows = stmt.query_map(params![key], |row| row.get::<_, String>(0))?;
        match rows.next() {
            Some(Ok(value)) => Ok(Some(value)),
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "INSERT INTO app_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn delete_meta(&self, key: &str) -> Result<(), rusqlite::Error> {
        self.conn
            .execute("DELETE FROM app_meta WHERE key = ?1", params![key])?;
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
