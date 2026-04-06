use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPage {
    pub page_id: i64,
    pub topic_id: i64,
    pub content_ko: String,
    pub content_en: String,
    pub source_count: Option<i64>,
    pub source_hash: Option<String>,
    pub version: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSource {
    pub citation_index: i64,
    pub chat_id: i64,
    pub message_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPageSearchResult {
    pub topic_id: i64,
    pub topic_title: String,
    pub snippet: String,
}

impl Store {
    pub fn insert_wiki_page(
        &self,
        topic_id: i64,
        content_ko: &str,
        content_en: &str,
        sources: &[(i64, i64)],
    ) -> Result<i64, sqlite::Error> {
        let version = {
            let mut stmt = self.conn().prepare(
                "SELECT COALESCE(MAX(version), 0) + 1 FROM wiki_pages WHERE topic_id = ?",
            )?;
            stmt.bind((1, topic_id))?;
            stmt.next()?;
            stmt.read::<i64, _>(0)?
        };

        let source_hash = compute_source_hash(sources);

        let mut stmt = self.conn().prepare(
            "INSERT INTO wiki_pages (topic_id, content_ko, content_en, source_count, source_hash, version)
             VALUES (?, ?, ?, ?, ?, ?)",
        )?;
        stmt.bind((1, topic_id))?;
        stmt.bind((2, content_ko))?;
        stmt.bind((3, content_en))?;
        stmt.bind((4, sources.len() as i64))?;
        stmt.bind((5, source_hash.as_str()))?;
        stmt.bind((6, version))?;
        stmt.next()?;

        let page_id = self.last_insert_rowid()?;

        let mut fts_stmt = self.conn().prepare(
            "INSERT INTO wiki_pages_fts (rowid, content_ko, content_en) VALUES (?, ?, ?)",
        )?;
        fts_stmt.bind((1, page_id))?;
        fts_stmt.bind((2, content_ko))?;
        fts_stmt.bind((3, content_en))?;
        fts_stmt.next()?;

        let mut src_stmt = self.conn().prepare(
            "INSERT INTO wiki_page_sources (page_id, citation_index, chat_id, message_id)
             VALUES (?, ?, ?, ?)",
        )?;
        for (i, &(chat_id, message_id)) in sources.iter().enumerate() {
            src_stmt.bind((1, page_id))?;
            src_stmt.bind((2, (i + 1) as i64))?;
            src_stmt.bind((3, chat_id))?;
            src_stmt.bind((4, message_id))?;
            src_stmt.next()?;
            src_stmt.reset()?;
        }

        self.conn().execute(format!(
            "UPDATE wiki_topics SET last_summary_at = strftime('%s', 'now') WHERE topic_id = {}",
            topic_id
        ))?;

        Ok(page_id)
    }

    pub fn get_latest_page(&self, topic_id: i64) -> Result<Option<WikiPage>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT page_id, topic_id, content_ko, content_en, source_count, source_hash, version, created_at
             FROM wiki_pages WHERE topic_id = ? ORDER BY version DESC LIMIT 1",
        )?;
        stmt.bind((1, topic_id))?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(Some(WikiPage {
                page_id: stmt.read::<i64, _>("page_id")?,
                topic_id: stmt.read::<i64, _>("topic_id")?,
                content_ko: stmt.read::<String, _>("content_ko")?,
                content_en: stmt.read::<String, _>("content_en")?,
                source_count: stmt.read::<Option<i64>, _>("source_count")?,
                source_hash: stmt.read::<Option<String>, _>("source_hash")?,
                version: stmt.read::<i64, _>("version")?,
                created_at: stmt.read::<String, _>("created_at")?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_page_sources(&self, page_id: i64) -> Result<Vec<PageSource>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT citation_index, chat_id, message_id FROM wiki_page_sources
             WHERE page_id = ? ORDER BY citation_index",
        )?;
        stmt.bind((1, page_id))?;
        let mut sources = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            sources.push(PageSource {
                citation_index: stmt.read::<i64, _>("citation_index")?,
                chat_id: stmt.read::<i64, _>("chat_id")?,
                message_id: stmt.read::<i64, _>("message_id")?,
            });
        }
        Ok(sources)
    }

    pub fn needs_regeneration(&self, topic_id: i64) -> Result<bool, sqlite::Error> {
        let page = self.get_latest_page(topic_id)?;
        match page {
            None => Ok(true),
            Some(p) => {
                let mut stmt = self.conn().prepare(
                    "SELECT chat_id, message_id FROM wiki_topic_messages
                     WHERE topic_id = ? ORDER BY chat_id, message_id",
                )?;
                stmt.bind((1, topic_id))?;
                let mut sources = Vec::new();
                while let sqlite::State::Row = stmt.next()? {
                    sources.push((
                        stmt.read::<i64, _>("chat_id")?,
                        stmt.read::<i64, _>("message_id")?,
                    ));
                }
                let current_hash = compute_source_hash(&sources);
                Ok(p.source_hash.as_deref() != Some(current_hash.as_str()))
            }
        }
    }

    pub fn search_wiki_pages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WikiPageSearchResult>, sqlite::Error> {
        if query.len() < 3 {
            return Ok(Vec::new());
        }
        let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
        let mut stmt = self.conn().prepare(format!(
            "SELECT wp.topic_id, wt.title, snippet(wiki_pages_fts, 0, '<b>', '</b>', '...', 32) as snippet
             FROM wiki_pages_fts fts
             JOIN wiki_pages wp ON wp.page_id = fts.rowid
             JOIN wiki_topics wt ON wt.topic_id = wp.topic_id
             WHERE wiki_pages_fts MATCH ?
             GROUP BY wp.topic_id
             LIMIT {}",
            limit
        ))?;
        stmt.bind((1, fts_query.as_str()))?;
        let mut results = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            results.push(WikiPageSearchResult {
                topic_id: stmt.read::<i64, _>("topic_id")?,
                topic_title: stmt.read::<String, _>("title")?,
                snippet: stmt.read::<String, _>("snippet")?,
            });
        }
        Ok(results)
    }

    pub fn clear_wiki_pages(&self) -> Result<(), sqlite::Error> {
        self.conn().execute("DELETE FROM wiki_page_sources")?;
        self.conn().execute("DELETE FROM wiki_pages")?;
        self.conn()
            .execute("INSERT INTO wiki_pages_fts(wiki_pages_fts) VALUES('rebuild')")?;
        Ok(())
    }
}

pub fn compute_source_hash(sources: &[(i64, i64)]) -> String {
    let mut hasher = Sha256::new();
    for &(chat_id, message_id) in sources {
        hasher.update(chat_id.to_le_bytes());
        hasher.update(message_id.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use crate::store::message::MessageRow;
    use crate::store::Store;

    fn setup() -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .conn()
            .execute("INSERT INTO chats (chat_id, title, chat_type) VALUES (1, 'Test', 'channel')")
            .unwrap();
        store
            .insert_messages_batch(&[
                MessageRow {
                    message_id: 1,
                    chat_id: 1,
                    timestamp: 1000,
                    text_plain: "test msg 1".to_string(),
                    text_stripped: "testmsg1".to_string(),
                    link: None,
                },
                MessageRow {
                    message_id: 2,
                    chat_id: 1,
                    timestamp: 2000,
                    text_plain: "test msg 2".to_string(),
                    text_stripped: "testmsg2".to_string(),
                    link: None,
                },
            ])
            .unwrap();
        store
    }

    #[test]
    fn test_insert_and_get_page() {
        let store = setup();
        let topic = crate::store::wiki_topic::NewTopic {
            title: "Test Topic".to_string(),
            title_ko: None,
            category_id: 1,
        };
        let topic_id = store.create_topic(&topic).unwrap();

        let page_id = store
            .insert_wiki_page(
                topic_id,
                "한국어 내용",
                "English content",
                &[(1, 1), (1, 2)],
            )
            .unwrap();
        assert!(page_id > 0);

        let page = store.get_latest_page(topic_id).unwrap().unwrap();
        assert_eq!(page.content_ko, "한국어 내용");
        assert_eq!(page.content_en, "English content");
        assert_eq!(page.version, 1);
        assert_eq!(page.source_count, Some(2));

        let sources = store.get_page_sources(page_id).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].citation_index, 1);
        assert_eq!(sources[1].citation_index, 2);
    }

    #[test]
    fn test_needs_regeneration() {
        let store = setup();
        let topic = crate::store::wiki_topic::NewTopic {
            title: "Regen Test".to_string(),
            title_ko: None,
            category_id: 1,
        };
        let topic_id = store.create_topic(&topic).unwrap();

        assert!(store.needs_regeneration(topic_id).unwrap());

        let link = crate::store::wiki_topic::TopicMessageLink {
            topic_id,
            chat_id: 1,
            message_id: 1,
            relevance: 1.0,
            assigned_category: "DeFi".to_string(),
        };
        store.link_message_to_topic(&link).unwrap();
        store
            .insert_wiki_page(topic_id, "ko", "en", &[(1, 1)])
            .unwrap();

        assert!(!store.needs_regeneration(topic_id).unwrap());

        let link2 = crate::store::wiki_topic::TopicMessageLink {
            topic_id,
            chat_id: 1,
            message_id: 2,
            relevance: 0.8,
            assigned_category: "DeFi".to_string(),
        };
        store.link_message_to_topic(&link2).unwrap();
        assert!(store.needs_regeneration(topic_id).unwrap());
    }
}
