use crate::indexer;
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

/// Execute a search query against the inverted index.
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

    // Tokenize the query using the same pipeline as indexing
    let tokens = indexer::tokenize_query(query_trimmed);
    if tokens.is_empty() {
        return Ok(SearchResult {
            items: vec![],
            next_cursor: None,
        });
    }

    // Look up term IDs for each token (across all source types)
    let term_id_groups: Vec<Vec<i64>> = tokens
        .iter()
        .map(|token| store.get_term_ids(token))
        .collect::<Result<Vec<_>, _>>()?;

    // Fetch limit+1 to detect if there's a next page
    let messages = match scope {
        SearchScope::All => store.search_messages_by_terms(&term_id_groups, cursor, limit + 1)?,
        SearchScope::Chat(chat_id) => {
            store.search_messages_by_terms_in_chat(&term_id_groups, *chat_id, cursor, limit + 1)?
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
    use crate::indexer;
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

    fn insert_and_index(store: &Store, chat_id: i64, msg_id: i64, ts: i64, text: &str) {
        let stripped = strip_whitespace(text);
        store
            .insert_messages_batch(&[MessageRow {
                message_id: msg_id,
                chat_id,
                timestamp: ts,
                text_plain: text.to_string(),
                text_stripped: stripped.clone(),
                link: None,
            }])
            .unwrap();
        indexer::index_message(store, chat_id, msg_id, ts, text, &stripped).unwrap();
    }

    #[test]
    fn test_search_english() {
        let store = test_store();
        setup(&store);
        insert_and_index(&store, 1, 1, 1000, "Hello world test message");
        insert_and_index(&store, 1, 2, 1001, "Another message here");

        let result = search(&store, "hello", &SearchScope::All, None, None).unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].message_id, 1);
        assert!(!result.items[0].highlights.is_empty());
    }

    #[test]
    fn test_search_korean() {
        let store = test_store();
        setup(&store);
        insert_and_index(&store, 1, 1, 1000, "삼성전자 주가가 상승했다");
        insert_and_index(&store, 1, 2, 1001, "오늘 날씨가 좋습니다");

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
        insert_and_index(&store, 1, 1, 1000, "Hello world");

        let result = search(&store, "zzzznonexistent", &SearchScope::All, None, None).unwrap();
        assert!(result.items.is_empty());
    }

    #[test]
    fn test_search_scoped_to_chat() {
        let store = test_store();
        setup(&store);
        insert_and_index(&store, 1, 1, 1000, "Hello from chat 1");
        insert_and_index(&store, 2, 2, 1001, "Hello from chat 2");

        let result = search(&store, "hello", &SearchScope::Chat(1), None, None).unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].chat_id, 1);
    }

    #[test]
    fn test_search_pagination() {
        let store = test_store();
        setup(&store);
        // Insert 5 messages all containing "test"
        for i in 0..5 {
            insert_and_index(&store, 1, i + 1, 1000 + i, &format!("test message {}", i));
        }

        // Page size 2
        let page1 = search(&store, "test", &SearchScope::All, None, Some(2)).unwrap();
        assert_eq!(page1.items.len(), 2);
        assert!(page1.next_cursor.is_some());

        // Page 2
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

        // Page 3 (last)
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
        insert_and_index(&store, 1, 1, 1000, "Hello world test");

        let result = search(&store, "hello", &SearchScope::All, None, None).unwrap();
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
        insert_and_index(&store, 1, 1, 1000, "test old message");
        insert_and_index(&store, 1, 2, 2000, "test new message");
        insert_and_index(&store, 1, 3, 1500, "test middle message");

        let result = search(&store, "test", &SearchScope::All, None, None).unwrap();
        assert_eq!(result.items.len(), 3);
        assert_eq!(result.items[0].timestamp, 2000); // newest first
        assert_eq!(result.items[1].timestamp, 1500);
        assert_eq!(result.items[2].timestamp, 1000);
    }
}
