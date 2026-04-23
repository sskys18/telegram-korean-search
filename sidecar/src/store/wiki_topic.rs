use serde::{Deserialize, Serialize};

use super::message::MessageWithChat;
use super::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiTopic {
    pub topic_id: i64,
    pub title: String,
    pub title_ko: Option<String>,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub category_name_ko: Option<String>,
    pub trending_score: f64,
    pub message_count: i64,
    pub channel_count: i64,
    pub first_seen_at: Option<i64>,
    pub last_seen_at: Option<i64>,
    pub last_summary_at: Option<i64>,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewTopic {
    pub title: String,
    pub title_ko: Option<String>,
    pub category_id: i64,
}

#[derive(Debug, Clone)]
pub struct TopicMessageLink {
    pub topic_id: i64,
    pub chat_id: i64,
    pub message_id: i64,
    pub relevance: f64,
    pub assigned_category: String,
}

impl Store {
    pub fn create_topic(&self, topic: &NewTopic) -> Result<i64, sqlite::Error> {
        let mut stmt = self
            .conn()
            .prepare("INSERT INTO wiki_topics (title, title_ko, category_id) VALUES (?, ?, ?)")?;
        stmt.bind((1, topic.title.as_str()))?;
        match topic.title_ko.as_deref() {
            Some(title_ko) => stmt.bind((2, title_ko))?,
            None => stmt.bind((2, sqlite::Value::Null))?,
        };
        stmt.bind((3, topic.category_id))?;
        stmt.next()?;

        let topic_id = self.last_insert_rowid()?;
        let alias = normalize_topic_title(&topic.title);
        self.add_topic_alias(topic_id, &alias)?;

        Ok(topic_id)
    }

    pub fn find_topic_by_alias(&self, raw_title: &str) -> Result<Option<i64>, sqlite::Error> {
        let normalized = normalize_topic_title(raw_title);
        let mut stmt = self
            .conn()
            .prepare("SELECT topic_id FROM wiki_topic_aliases WHERE alias = ?")?;
        stmt.bind((1, normalized.as_str()))?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(Some(stmt.read::<i64, _>(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn get_similar_aliases(
        &self,
        raw_title: &str,
        limit: usize,
    ) -> Result<Vec<(i64, String)>, sqlite::Error> {
        let normalized = normalize_topic_title(raw_title);
        let prefix = if normalized.len() >= 3 {
            &normalized[..3]
        } else {
            normalized.as_str()
        };
        let pattern = format!("{}%", prefix);
        let mut stmt = self.conn().prepare(format!(
            "SELECT topic_id, alias FROM wiki_topic_aliases WHERE alias LIKE ? LIMIT {}",
            limit
        ))?;
        stmt.bind((1, pattern.as_str()))?;
        let mut results = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            results.push((
                stmt.read::<i64, _>("topic_id")?,
                stmt.read::<String, _>("alias")?,
            ));
        }
        Ok(results)
    }

    /// Fuzzy-match a new topic title against existing topics using word overlap.
    /// Returns the best-matching topic_id if similarity exceeds the threshold.
    pub fn find_topic_fuzzy(&self, raw_title: &str) -> Result<Option<i64>, sqlite::Error> {
        let new_words = extract_significant_words(raw_title);
        if new_words.len() < 2 {
            return Ok(None);
        }

        // Get recent active topics to compare against
        let mut stmt = self.conn().prepare(
            "SELECT topic_id, title FROM wiki_topics
             WHERE message_count >= 2
             ORDER BY last_seen_at DESC LIMIT 500",
        )?;
        let mut best: Option<(i64, f64)> = None;
        while let sqlite::State::Row = stmt.next()? {
            let topic_id = stmt.read::<i64, _>("topic_id")?;
            let title = stmt.read::<String, _>("title")?;
            let existing_words = extract_significant_words(&title);
            if existing_words.is_empty() {
                continue;
            }
            let score = word_overlap_score(&new_words, &existing_words);
            if score >= 0.6 && (best.is_none() || score > best.unwrap().1) {
                best = Some((topic_id, score));
            }
        }
        Ok(best.map(|(id, _)| id))
    }

    pub fn add_topic_alias(&self, topic_id: i64, alias: &str) -> Result<(), sqlite::Error> {
        let mut stmt = self
            .conn()
            .prepare("INSERT OR IGNORE INTO wiki_topic_aliases (topic_id, alias) VALUES (?, ?)")?;
        stmt.bind((1, topic_id))?;
        stmt.bind((2, alias))?;
        stmt.next()?;
        Ok(())
    }

    pub fn link_message_to_topic(&self, link: &TopicMessageLink) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "INSERT OR IGNORE INTO wiki_topic_messages
             (topic_id, chat_id, message_id, relevance, assigned_category)
             VALUES (?, ?, ?, ?, ?)",
        )?;
        stmt.bind((1, link.topic_id))?;
        stmt.bind((2, link.chat_id))?;
        stmt.bind((3, link.message_id))?;
        stmt.bind((4, link.relevance))?;
        stmt.bind((5, link.assigned_category.as_str()))?;
        stmt.next()?;

        self.refresh_topic_counters(link.topic_id)?;

        Ok(())
    }

    fn refresh_topic_counters(&self, topic_id: i64) -> Result<(), sqlite::Error> {
        self.conn().execute(format!(
            "UPDATE wiki_topics SET
                message_count = (SELECT COUNT(*) FROM wiki_topic_messages WHERE topic_id = {0}),
                channel_count = (SELECT COUNT(DISTINCT chat_id) FROM wiki_topic_messages WHERE topic_id = {0}),
                first_seen_at = (SELECT MIN(m.timestamp) FROM wiki_topic_messages tm
                    JOIN messages m ON m.chat_id = tm.chat_id AND m.message_id = tm.message_id
                    WHERE tm.topic_id = {0}),
                last_seen_at = (SELECT MAX(m.timestamp) FROM wiki_topic_messages tm
                    JOIN messages m ON m.chat_id = tm.chat_id AND m.message_id = tm.message_id
                    WHERE tm.topic_id = {0}),
                updated_at = datetime('now')
             WHERE topic_id = {0}",
            topic_id
        ))?;
        Ok(())
    }

    pub fn set_title_ko_if_absent(
        &self,
        topic_id: i64,
        title_ko: &str,
    ) -> Result<(), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "UPDATE wiki_topics SET title_ko = ? WHERE topic_id = ? AND title_ko IS NULL",
        )?;
        stmt.bind((1, title_ko))?;
        stmt.bind((2, topic_id))?;
        stmt.next()?;
        Ok(())
    }

    pub fn check_category_reconciliation(
        &self,
        topic_id: i64,
    ) -> Result<Option<i64>, sqlite::Error> {
        let mut stmt = self
            .conn()
            .prepare("SELECT message_count, category_id FROM wiki_topics WHERE topic_id = ?")?;
        stmt.bind((1, topic_id))?;
        if let sqlite::State::Row = stmt.next()? {
            let count = stmt.read::<i64, _>("message_count")?;
            let current_cat = stmt.read::<Option<i64>, _>("category_id")?;
            if count <= 10 {
                return Ok(None);
            }

            let mut stmt2 = self.conn().prepare(
                "SELECT assigned_category, COUNT(*) as cnt
                 FROM wiki_topic_messages WHERE topic_id = ?
                 GROUP BY assigned_category ORDER BY cnt DESC LIMIT 1",
            )?;
            stmt2.bind((1, topic_id))?;
            if let sqlite::State::Row = stmt2.next()? {
                let top_cat = stmt2.read::<String, _>("assigned_category")?;
                let top_cnt = stmt2.read::<i64, _>("cnt")?;
                let ratio = top_cnt as f64 / count as f64;

                if ratio > 0.6 {
                    let new_id = self.normalize_category(&top_cat)?;
                    if current_cat != Some(new_id) {
                        return Ok(Some(new_id));
                    }
                }
            }
            Ok(None)
        } else {
            Ok(None)
        }
    }

    pub fn update_topic_category(
        &self,
        topic_id: i64,
        category_id: i64,
    ) -> Result<(), sqlite::Error> {
        let mut stmt = self
            .conn()
            .prepare("UPDATE wiki_topics SET category_id = ?, updated_at = datetime('now') WHERE topic_id = ?")?;
        stmt.bind((1, category_id))?;
        stmt.bind((2, topic_id))?;
        stmt.next()?;
        Ok(())
    }

    pub fn update_trending_score(&self, topic_id: i64, score: f64) -> Result<(), sqlite::Error> {
        let mut stmt = self
            .conn()
            .prepare("UPDATE wiki_topics SET trending_score = ? WHERE topic_id = ?")?;
        stmt.bind((1, score))?;
        stmt.bind((2, topic_id))?;
        stmt.next()?;
        Ok(())
    }

    pub fn get_trending_topics(
        &self,
        limit: usize,
        offset: usize,
        category_id: Option<i64>,
    ) -> Result<Vec<WikiTopic>, sqlite::Error> {
        let sql = match category_id {
            Some(_) => format!(
                "SELECT t.*, c.name as category_name, c.name_ko as category_name_ko
                 FROM wiki_topics t
                 LEFT JOIN wiki_categories c ON t.category_id = c.category_id
                 WHERE t.category_id = ?
                 ORDER BY t.trending_score DESC
                 LIMIT {} OFFSET {}",
                limit, offset
            ),
            None => format!(
                "SELECT t.*, c.name as category_name, c.name_ko as category_name_ko
                 FROM wiki_topics t
                 LEFT JOIN wiki_categories c ON t.category_id = c.category_id
                 ORDER BY t.trending_score DESC
                 LIMIT {} OFFSET {}",
                limit, offset
            ),
        };
        let mut stmt = self.conn().prepare(&sql)?;
        if let Some(cat_id) = category_id {
            stmt.bind((1, cat_id))?;
        }
        let mut topics = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            topics.push(read_wiki_topic(&stmt)?);
        }
        Ok(topics)
    }

    pub fn get_topic(&self, topic_id: i64) -> Result<Option<WikiTopic>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT t.*, c.name as category_name, c.name_ko as category_name_ko
             FROM wiki_topics t
             LEFT JOIN wiki_categories c ON t.category_id = c.category_id
             WHERE t.topic_id = ?",
        )?;
        stmt.bind((1, topic_id))?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(Some(read_wiki_topic(&stmt)?))
        } else {
            Ok(None)
        }
    }

    pub fn get_topic_sources(
        &self,
        topic_id: i64,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MessageWithChat>, sqlite::Error> {
        let mut stmt = self.conn().prepare(format!(
            "SELECT m.message_id, m.chat_id, m.timestamp, m.text_plain, m.link, ch.title as chat_title
             FROM wiki_topic_messages tm
             JOIN messages m ON m.chat_id = tm.chat_id AND m.message_id = tm.message_id
             JOIN chats ch ON ch.chat_id = m.chat_id
             WHERE tm.topic_id = ?
             ORDER BY tm.relevance DESC, m.timestamp DESC
             LIMIT {} OFFSET {}",
            limit, offset
        ))?;
        stmt.bind((1, topic_id))?;
        let mut msgs = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            msgs.push(MessageWithChat {
                message_id: stmt.read::<i64, _>("message_id")?,
                chat_id: stmt.read::<i64, _>("chat_id")?,
                timestamp: stmt.read::<i64, _>("timestamp")?,
                text_plain: stmt.read::<String, _>("text_plain")?,
                link: stmt.read::<Option<String>, _>("link")?,
                chat_title: stmt.read::<String, _>("chat_title")?,
            });
        }
        Ok(msgs)
    }

    pub fn get_topics_needing_summary(&self) -> Result<Vec<i64>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT topic_id FROM wiki_topics
             WHERE last_summary_at IS NULL
                OR last_seen_at > last_summary_at",
        )?;
        let mut ids = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            ids.push(stmt.read::<i64, _>(0)?);
        }
        Ok(ids)
    }

    pub fn search_topics(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WikiTopic>, sqlite::Error> {
        let pattern = format!("%{}%", query);
        let mut stmt = self.conn().prepare(format!(
            "SELECT t.*, c.name as category_name, c.name_ko as category_name_ko
             FROM wiki_topics t
             LEFT JOIN wiki_categories c ON t.category_id = c.category_id
             WHERE t.title LIKE ? OR t.title_ko LIKE ?
             ORDER BY t.trending_score DESC
             LIMIT {}",
            limit
        ))?;
        stmt.bind((1, pattern.as_str()))?;
        stmt.bind((2, pattern.as_str()))?;
        let mut topics = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            topics.push(read_wiki_topic(&stmt)?);
        }
        Ok(topics)
    }

    pub fn clear_wiki_topics(&self) -> Result<(), sqlite::Error> {
        self.conn().execute("DELETE FROM wiki_topic_messages")?;
        self.conn().execute("DELETE FROM wiki_topic_aliases")?;
        self.conn().execute("DELETE FROM wiki_topics")?;
        Ok(())
    }

    /// Latest messages linked to a topic, newest first. Uses LEFT JOIN
    /// on `chats` so a message whose chat row is missing still renders
    /// (with an empty title) instead of vanishing.
    pub fn get_topic_messages(
        &self,
        topic_id: i64,
        limit: usize,
    ) -> Result<Vec<TopicMessageRow>, sqlite::Error> {
        let mut stmt = self.conn().prepare(format!(
            "SELECT m.chat_id, m.message_id, m.timestamp, m.text_plain, m.link,
                    COALESCE(c.title, '')
             FROM wiki_topic_messages wtm
             JOIN messages m ON m.chat_id = wtm.chat_id
                             AND m.message_id = wtm.message_id
             LEFT JOIN chats c ON c.chat_id = m.chat_id
             WHERE wtm.topic_id = ?
             ORDER BY m.timestamp DESC
             LIMIT {limit}",
        ))?;
        stmt.bind((1, topic_id))?;
        let mut out = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            out.push(TopicMessageRow {
                chat_id: stmt.read::<i64, _>(0)?,
                message_id: stmt.read::<i64, _>(1)?,
                timestamp: stmt.read::<i64, _>(2)?,
                text: stmt.read::<String, _>(3)?,
                link: stmt.read::<Option<String>, _>(4)?,
                chat_title: stmt.read::<String, _>(5)?,
            });
        }
        Ok(out)
    }
}

#[derive(Debug, Clone)]
pub struct TopicMessageRow {
    pub chat_id: i64,
    pub message_id: i64,
    pub timestamp: i64,
    pub text: String,
    pub link: Option<String>,
    pub chat_title: String,
}

fn read_wiki_topic(stmt: &sqlite::Statement) -> Result<WikiTopic, sqlite::Error> {
    Ok(WikiTopic {
        topic_id: stmt.read::<i64, _>("topic_id")?,
        title: stmt.read::<String, _>("title")?,
        title_ko: stmt.read::<Option<String>, _>("title_ko")?,
        category_id: stmt.read::<Option<i64>, _>("category_id")?,
        category_name: stmt.read::<Option<String>, _>("category_name")?,
        category_name_ko: stmt.read::<Option<String>, _>("category_name_ko")?,
        trending_score: stmt.read::<f64, _>("trending_score")?,
        message_count: stmt.read::<i64, _>("message_count")?,
        channel_count: stmt.read::<i64, _>("channel_count")?,
        first_seen_at: stmt.read::<Option<i64>, _>("first_seen_at")?,
        last_seen_at: stmt.read::<Option<i64>, _>("last_seen_at")?,
        last_summary_at: stmt.read::<Option<i64>, _>("last_summary_at")?,
        updated_at: stmt.read::<String, _>("updated_at")?,
    })
}

pub fn normalize_topic_title(title: &str) -> String {
    let mut s: String = title
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    for suffix in &["update", "news", "alert", "analysis"] {
        if s.ends_with(suffix) && s.len() > suffix.len() + 3 {
            s = s[..s.len() - suffix.len()].to_string();
        }
    }
    s
}

/// Stop words filtered out for fuzzy matching.
const STOP_WORDS: &[&str] = &[
    "the", "a", "an", "of", "in", "on", "for", "to", "and", "is", "are", "was", "were", "with",
    "by", "at", "from", "as", "or", "its", "it", "this", "that", "new", "more", "most", "will",
    "may", "can", "has", "had", "have", "been", "about", "after", "before", "into",
];

/// Extract significant words from a title for fuzzy matching.
fn extract_significant_words(title: &str) -> Vec<String> {
    title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| w.len() >= 2 && !STOP_WORDS.contains(w))
        .map(|w| w.to_string())
        .collect()
}

/// Jaccard-like word overlap score between two word sets.
fn word_overlap_score(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let set_a: std::collections::HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let set_b: std::collections::HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let intersection = set_a.intersection(&set_b).count() as f64;
    let smaller = set_a.len().min(set_b.len()) as f64;
    // Use overlap coefficient (intersection / min) instead of Jaccard
    // This handles cases where one title is a subset of another
    intersection / smaller
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn test_word_overlap_score() {
        // Same subject, different phrasing — high overlap
        let a = extract_significant_words("Strategy Bitcoin Purchases");
        let b = extract_significant_words("Strategy Buys More Bitcoin");
        assert!(word_overlap_score(&a, &b) >= 0.5);

        // Subset match: "Bitcoin ETF Approval" vs "Bitcoin ETF"
        let e = extract_significant_words("Bitcoin ETF Approval");
        let f = extract_significant_words("Bitcoin ETF Inflows");
        assert!(word_overlap_score(&e, &f) >= 0.6);

        // Unrelated topics should score low
        let d = extract_significant_words("Ethereum Gas Fee Reduction");
        assert!(word_overlap_score(&a, &d) < 0.3);
    }

    #[test]
    fn test_normalize_topic_title() {
        assert_eq!(normalize_topic_title("ETH Layer 2 Fees"), "ethlayer2fees");
        assert_eq!(
            normalize_topic_title("Bitcoin Price Update"),
            "bitcoinprice"
        );
        assert_eq!(normalize_topic_title("BTC"), "btc");
    }

    #[test]
    fn test_create_and_find_topic() {
        let store = Store::open_in_memory().unwrap();
        let topic = NewTopic {
            title: "ETH Layer 2 Fees".to_string(),
            title_ko: Some("이더리움 L2 수수료".to_string()),
            category_id: store.resolve_category("Test", None).unwrap(),
        };
        let id = store.create_topic(&topic).unwrap();
        assert!(id > 0);

        let found = store.find_topic_by_alias("ETH Layer 2 Fees").unwrap();
        assert_eq!(found, Some(id));

        let found2 = store.find_topic_by_alias("eth layer 2 fees").unwrap();
        assert_eq!(found2, Some(id));
    }

    #[test]
    fn test_get_topic_with_category() {
        let store = Store::open_in_memory().unwrap();
        let cat_id = store.resolve_category("DeFi", Some("디파이")).unwrap();
        let topic = NewTopic {
            title: "DeFi Test".to_string(),
            title_ko: None,
            category_id: cat_id,
        };
        let id = store.create_topic(&topic).unwrap();
        let loaded = store.get_topic(id).unwrap().unwrap();
        assert_eq!(loaded.title, "DeFi Test");
        assert_eq!(loaded.category_name, Some("DeFi".to_string()));
    }

    fn mk_chat(store: &Store, chat_id: i64, title: &str) {
        store
            .conn()
            .execute(format!(
                "INSERT INTO chats (chat_id, title, chat_type) VALUES ({}, '{}', 'channel')",
                chat_id, title
            ))
            .unwrap();
    }

    fn mk_message(store: &Store, chat_id: i64, message_id: i64, ts: i64, text: &str) {
        store
            .conn()
            .execute(format!(
                "INSERT INTO messages (message_id, chat_id, timestamp, text_plain, text_stripped)
                 VALUES ({}, {}, {}, '{}', '{}')",
                message_id, chat_id, ts, text, text
            ))
            .unwrap();
    }

    #[test]
    fn test_get_topic_messages_order_and_limit() {
        let store = Store::open_in_memory().unwrap();
        let cat = store.resolve_category("Test", None).unwrap();
        let topic_id = store
            .create_topic(&NewTopic {
                title: "T".into(),
                title_ko: None,
                category_id: cat,
            })
            .unwrap();
        mk_chat(&store, 1, "room");
        mk_message(&store, 1, 1, 100, "old");
        mk_message(&store, 1, 2, 300, "newest");
        mk_message(&store, 1, 3, 200, "middle");
        store
            .link_message_to_topic(&TopicMessageLink {
                topic_id,
                chat_id: 1,
                message_id: 1,
                relevance: 1.0,
                assigned_category: "Test".into(),
            })
            .unwrap();
        store
            .link_message_to_topic(&TopicMessageLink {
                topic_id,
                chat_id: 1,
                message_id: 2,
                relevance: 1.0,
                assigned_category: "Test".into(),
            })
            .unwrap();
        store
            .link_message_to_topic(&TopicMessageLink {
                topic_id,
                chat_id: 1,
                message_id: 3,
                relevance: 1.0,
                assigned_category: "Test".into(),
            })
            .unwrap();

        let rows = store.get_topic_messages(topic_id, 10).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].message_id, 2);
        assert_eq!(rows[1].message_id, 3);
        assert_eq!(rows[2].message_id, 1);
        assert_eq!(rows[0].chat_title, "room");

        let limited = store.get_topic_messages(topic_id, 2).unwrap();
        assert_eq!(limited.len(), 2);
        assert_eq!(limited[0].message_id, 2);
    }

    #[test]
    fn test_get_topic_messages_empty() {
        let store = Store::open_in_memory().unwrap();
        let cat = store.resolve_category("Test", None).unwrap();
        let topic_id = store
            .create_topic(&NewTopic {
                title: "Empty".into(),
                title_ko: None,
                category_id: cat,
            })
            .unwrap();
        let rows = store.get_topic_messages(topic_id, 10).unwrap();
        assert!(rows.is_empty());
    }
}
