use serde::{Deserialize, Serialize};

use super::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiCategory {
    pub category_id: i64,
    pub name: String,
    pub name_ko: Option<String>,
    pub sort_order: i64,
}

impl Store {
    pub fn get_all_categories(&self) -> Result<Vec<WikiCategory>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT category_id, name, name_ko, sort_order FROM wiki_categories ORDER BY sort_order",
        )?;
        let mut cats = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            cats.push(WikiCategory {
                category_id: stmt.read::<i64, _>("category_id")?,
                name: stmt.read::<String, _>("name")?,
                name_ko: stmt.read::<Option<String>, _>("name_ko")?,
                sort_order: stmt.read::<i64, _>("sort_order")?,
            });
        }
        Ok(cats)
    }

    pub fn get_category_by_id(
        &self,
        category_id: i64,
    ) -> Result<Option<WikiCategory>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT category_id, name, name_ko, sort_order FROM wiki_categories WHERE category_id = ?",
        )?;
        stmt.bind((1, category_id))?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(Some(WikiCategory {
                category_id: stmt.read::<i64, _>("category_id")?,
                name: stmt.read::<String, _>("name")?,
                name_ko: stmt.read::<Option<String>, _>("name_ko")?,
                sort_order: stmt.read::<i64, _>("sort_order")?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn normalize_category(&self, raw: &str) -> Result<i64, sqlite::Error> {
        let normalized = raw.trim().to_lowercase();

        let mut stmt = self
            .conn()
            .prepare("SELECT category_id FROM wiki_categories WHERE LOWER(name) = ?")?;
        stmt.bind((1, normalized.as_str()))?;
        if let sqlite::State::Row = stmt.next()? {
            return stmt.read::<i64, _>(0);
        }

        let like_pattern = format!("%{}%", normalized);
        let mut stmt = self
            .conn()
            .prepare("SELECT category_id FROM wiki_categories WHERE LOWER(name) LIKE ? LIMIT 1")?;
        stmt.bind((1, like_pattern.as_str()))?;
        if let sqlite::State::Row = stmt.next()? {
            return stmt.read::<i64, _>(0);
        }

        let mut stmt = self
            .conn()
            .prepare("SELECT category_id FROM wiki_categories WHERE name = 'Other'")?;
        if let sqlite::State::Row = stmt.next()? {
            stmt.read::<i64, _>(0)
        } else {
            Ok(9)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::store::Store;

    #[test]
    fn test_get_all_categories() {
        let store = Store::open_in_memory().unwrap();
        let cats = store.get_all_categories().unwrap();
        assert_eq!(cats.len(), 9);
        assert_eq!(cats[0].name, "DeFi");
        assert_eq!(cats[0].name_ko, Some("디파이".to_string()));
    }

    #[test]
    fn test_normalize_category_exact() {
        let store = Store::open_in_memory().unwrap();
        let id = store.normalize_category("DeFi").unwrap();
        let cat = store.get_category_by_id(id).unwrap().unwrap();
        assert_eq!(cat.name, "DeFi");
    }

    #[test]
    fn test_normalize_category_case_insensitive() {
        let store = Store::open_in_memory().unwrap();
        let id = store.normalize_category("defi").unwrap();
        let cat = store.get_category_by_id(id).unwrap().unwrap();
        assert_eq!(cat.name, "DeFi");
    }

    #[test]
    fn test_normalize_category_fallback() {
        let store = Store::open_in_memory().unwrap();
        let id = store.normalize_category("something unknown").unwrap();
        let cat = store.get_category_by_id(id).unwrap().unwrap();
        assert_eq!(cat.name, "Other");
    }
}
