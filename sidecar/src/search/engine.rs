use crate::store::message::{Cursor, MessageWithChat};
use crate::store::Store;

use super::highlight::find_highlights;
use super::{SearchItem, SearchResult};

const DEFAULT_PAGE_SIZE: usize = 30;

/// Search scope: all chats or a specific chat.
#[derive(Debug, Clone)]
pub enum SearchScope {
    All,
    Chat(i64),
}

/// Build an FTS5 query from user input.
/// Each whitespace-separated term is quoted for exact substring matching.
/// Multiple terms are AND'd (FTS5 default).
fn build_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Execute a search query against the FTS5 trigram index.
pub fn search(
    store: &Store,
    query: &str,
    scope: &SearchScope,
    cursor: Option<&Cursor>,
    limit: Option<usize>,
) -> Result<SearchResult, sqlite::Error> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_SIZE);
    let query_trimmed = query.trim();

    if query_trimmed.is_empty() {
        return Ok(SearchResult {
            items: vec![],
            next_cursor: None,
        });
    }

    // Query tokens for highlighting (simple whitespace split)
    let tokens: Vec<String> = query_trimmed
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    if tokens.is_empty() {
        return Ok(SearchResult {
            items: vec![],
            next_cursor: None,
        });
    }

    // FTS5 trigram needs >= 3 chars per term. Fall back to LIKE for short terms.
    let use_fts = tokens.iter().all(|t| t.chars().count() >= 3);

    // Fetch limit+1 to detect if there's a next page
    let messages = if use_fts {
        let fts_query = build_fts_query(query_trimmed);
        match scope {
            SearchScope::All => store.search_messages_fts(&fts_query, cursor, limit + 1)?,
            SearchScope::Chat(chat_id) => {
                store.search_messages_fts_in_chat(&fts_query, *chat_id, cursor, limit + 1)?
            }
        }
    } else {
        match scope {
            SearchScope::All => store.search_messages_like(&tokens, cursor, limit + 1)?,
            SearchScope::Chat(chat_id) => {
                store.search_messages_like_in_chat(&tokens, *chat_id, cursor, limit + 1)?
            }
        }
    };

    let has_more = messages.len() > limit;
    let results: Vec<MessageWithChat> = if has_more {
        messages[..limit].to_vec()
    } else {
        messages
    };

    let next_cursor = if has_more {
        results.last().map(|last| Cursor {
            timestamp: last.timestamp,
            chat_id: last.chat_id,
            message_id: last.message_id,
        })
    } else {
        None
    };

    let items: Vec<SearchItem> = results
        .into_iter()
        .map(|msg| {
            let highlights = find_highlights(&msg.text_plain, &tokens);
            SearchItem {
                message_id: msg.message_id,
                chat_id: msg.chat_id,
                timestamp: msg.timestamp,
                text: msg.text_plain,
                link: msg.link,
                chat_title: msg.chat_title,
                highlights,
            }
        })
        .collect();

    Ok(SearchResult { items, next_cursor })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::chat::ChatRow;
    use crate::store::message::{strip_whitespace, MessageRow};

    fn test_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn setup(store: &Store) {
        store
            .upsert_chat(&ChatRow {
                chat_id: 1,
                title: "Korean Chat".to_string(),
                chat_type: "supergroup".to_string(),
                username: Some("koreanchat".to_string()),
                access_hash: None,
                is_excluded: false,
            })
            .unwrap();
        store
            .upsert_chat(&ChatRow {
                chat_id: 2,
                title: "English Chat".to_string(),
                chat_type: "supergroup".to_string(),
                username: None,
                access_hash: None,
                is_excluded: false,
            })
            .unwrap();
    }

    fn insert_msg(store: &Store, chat_id: i64, msg_id: i64, ts: i64, text: &str) {
        let stripped = strip_whitespace(text);
        store
            .insert_messages_batch(&[MessageRow {
                message_id: msg_id,
                chat_id,
                timestamp: ts,
                text_plain: text.to_string(),
                text_stripped: stripped,
                link: None,
            }])
            .unwrap();
    }

    #[test]
    fn test_search_english() {
        let store = test_store();
        setup(&store);
        insert_msg(&store, 1, 1, 1000, "Hello world test message");
        insert_msg(&store, 1, 2, 1001, "Another message here");

        let result = search(&store, "Hello", &SearchScope::All, None, None).unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].message_id, 1);
        assert!(!result.items[0].highlights.is_empty());
    }

    #[test]
    fn test_search_korean() {
        let store = test_store();
        setup(&store);
        insert_msg(&store, 1, 1, 1000, "삼성전자 주가가 상승했다");
        insert_msg(&store, 1, 2, 1001, "오늘 날씨가 좋습니다");

        let result = search(&store, "삼성", &SearchScope::All, None, None).unwrap();
        assert!(!result.items.is_empty());
        assert_eq!(result.items[0].chat_id, 1);
    }

    #[test]
    fn test_search_empty_query() {
        let store = test_store();
        let result = search(&store, "", &SearchScope::All, None, None).unwrap();
        assert!(result.items.is_empty());
        assert!(result.next_cursor.is_none());
    }

    #[test]
    fn test_search_no_results() {
        let store = test_store();
        setup(&store);
        insert_msg(&store, 1, 1, 1000, "Hello world");

        let result = search(&store, "zzzznonexistent", &SearchScope::All, None, None).unwrap();
        assert!(result.items.is_empty());
    }

    #[test]
    fn test_search_scoped_to_chat() {
        let store = test_store();
        setup(&store);
        insert_msg(&store, 1, 1, 1000, "Hello from chat 1");
        insert_msg(&store, 2, 2, 1001, "Hello from chat 2");

        let result = search(&store, "Hello", &SearchScope::Chat(1), None, None).unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].chat_id, 1);
    }

    #[test]
    fn test_search_pagination() {
        let store = test_store();
        setup(&store);
        for i in 0..5 {
            insert_msg(&store, 1, i + 1, 1000 + i, &format!("test message {}", i));
        }

        let page1 = search(&store, "test", &SearchScope::All, None, Some(2)).unwrap();
        assert_eq!(page1.items.len(), 2);
        assert!(page1.next_cursor.is_some());

        let page2 = search(
            &store,
            "test",
            &SearchScope::All,
            page1.next_cursor.as_ref(),
            Some(2),
        )
        .unwrap();
        assert_eq!(page2.items.len(), 2);
        assert!(page2.next_cursor.is_some());

        let page3 = search(
            &store,
            "test",
            &SearchScope::All,
            page2.next_cursor.as_ref(),
            Some(2),
        )
        .unwrap();
        assert_eq!(page3.items.len(), 1);
        assert!(page3.next_cursor.is_none());
    }

    #[test]
    fn test_search_results_have_highlights() {
        let store = test_store();
        setup(&store);
        insert_msg(&store, 1, 1, 1000, "Hello world test");

        let result = search(&store, "Hello", &SearchScope::All, None, None).unwrap();
        assert_eq!(result.items.len(), 1);
        let item = &result.items[0];
        assert!(!item.highlights.is_empty());
        assert_eq!(item.highlights[0].start, 0);
        assert_eq!(item.highlights[0].end, 5);
    }

    #[test]
    fn test_search_results_ordered_by_timestamp_desc() {
        let store = test_store();
        setup(&store);
        insert_msg(&store, 1, 1, 1000, "test old message");
        insert_msg(&store, 1, 2, 2000, "test new message");
        insert_msg(&store, 1, 3, 1500, "test middle message");

        let result = search(&store, "test", &SearchScope::All, None, None).unwrap();
        assert_eq!(result.items.len(), 3);
        assert_eq!(result.items[0].timestamp, 2000);
        assert_eq!(result.items[1].timestamp, 1500);
        assert_eq!(result.items[2].timestamp, 1000);
    }

    #[test]
    fn test_build_fts_query() {
        assert_eq!(build_fts_query("hello world"), "\"hello\" \"world\"");
        assert_eq!(build_fts_query("삼성전자"), "\"삼성전자\"");
        assert_eq!(build_fts_query("  spaces  "), "\"spaces\"");
    }
}
