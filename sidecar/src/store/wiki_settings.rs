use super::Store;

impl Store {
    pub fn get_wiki_setting(&self, key: &str) -> Result<Option<String>, sqlite::Error> {
        let mut stmt = self
            .conn()
            .prepare("SELECT value FROM wiki_settings WHERE key = ?")?;
        stmt.bind((1, key))?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(Some(stmt.read::<String, _>(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn get_wiki_setting_i64(&self, key: &str, default: i64) -> i64 {
        self.get_wiki_setting(key)
            .ok()
            .flatten()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(default)
    }
}

#[cfg(test)]
mod tests {
    use crate::store::Store;

    #[test]
    fn seeded_classify_settings_present() {
        let s = Store::open_in_memory().unwrap();
        assert_eq!(
            s.get_wiki_setting("classify_batch_size")
                .unwrap()
                .as_deref(),
            Some("20")
        );
        assert_eq!(s.get_wiki_setting_i64("max_classify_attempts", 99), 3);
        assert_eq!(s.get_wiki_setting_i64("missing_key", 7), 7);
    }
}
