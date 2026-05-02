use crate::store::message::{strip_whitespace, Cursor, MessageWithChat};
use crate::store::Store;

use super::hangul::decompose_jamo;
use super::highlight::find_highlights;
use super::{SearchItem, SearchResult};

const DEFAULT_PAGE_SIZE: usize = 30;

/// Search scope: all chats or a specific chat.
#[derive(Debug, Clone)]
pub enum SearchScope {
    All,
    Chat(i64),
}

/// Build an FTS5 `MATCH` argument from user input. Each whitespace
/// separated term is quoted so FTS5 treats it as a phrase (exact
/// substring in trigram mode); multiple terms are AND'd implicitly.
fn build_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Build one MATCH expression over the combined v8 FTS table. The
/// variants preserve the old plain, nospace, and jamo match behavior
/// while BM25 ranks the single result set.
fn build_match_query(raw_query: &str) -> Option<String> {
    let trimmed = raw_query.trim();
    if trimmed.is_empty() {
        return None;
    }

    let stripped = strip_whitespace(trimmed);
    let jamo = decompose_jamo(trimmed);
    let mut variants = Vec::new();

    for variant in [trimmed.to_string(), stripped, jamo] {
        if variant.is_empty() {
            continue;
        }
        let query = build_fts_query(&variant);
        if trigram_ready(&query) && !variants.contains(&query) {
            variants.push(query);
        }
    }

    if variants.is_empty() {
        None
    } else {
        Some(variants.join(" OR "))
    }
}

/// FTS5 trigram refuses queries whose longest term is < 3 chars.
/// Check against the already-quoted query string the planner built.
fn trigram_ready(fts_query: &str) -> bool {
    fts_query
        .split_whitespace()
        .any(|term| term.trim_matches('"').chars().count() >= 3)
}

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

    let tokens: Vec<String> = query_trimmed
        .split_whitespace()
        .map(|s| s.to_string())
        .collect();

    let fts_query = build_match_query(query_trimmed);
    let scope_chat = match scope {
        SearchScope::All => None,
        SearchScope::Chat(id) => Some(*id),
    };

    let messages = if let Some(fts_query) = fts_query {
        store.search_messages_bm25(&fts_query, scope_chat, cursor, limit + 1)?
    } else {
        // Trigram needs >=3 chars; fall back to LIKE.
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
            rank: last.rank,
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
                sender_id: 0,
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
    fn bm25_strong_match_beats_newer_weak_match() {
        let store = test_store();
        setup(&store);
        insert_msg(
            &store,
            1,
            1,
            1000,
            "bitcoin etf bitcoin etf bitcoin etf confirmed inflows",
        );
        insert_msg(&store, 1, 2, 1001, "bitcoin etf mentioned once");

        let result = search(&store, "bitcoin etf", &SearchScope::All, None, None).unwrap();
        assert_eq!(result.items.len(), 2);
        assert_eq!(result.items[0].message_id, 1);
    }

    #[test]
    fn test_build_fts_query() {
        assert_eq!(build_fts_query("hello world"), "\"hello\" \"world\"");
        assert_eq!(build_fts_query("삼성전자"), "\"삼성전자\"");
        assert_eq!(build_fts_query("  spaces  "), "\"spaces\"");
    }

    // -------- Korean-specific integration tests (schema v6) --------

    fn korean_store() -> Store {
        let store = test_store();
        setup(&store);
        insert_msg(&store, 1, 1, 1000, "삼성전자 주가가 상승했다");
        insert_msg(&store, 1, 2, 1001, "삼성 전자 실적 발표");
        insert_msg(&store, 1, 3, 1002, "대한민국 만세");
        insert_msg(&store, 1, 4, 1003, "오늘 날씨가 좋다");
        insert_msg(&store, 1, 5, 1004, "Apple Galaxy 비교");
        store
    }

    #[test]
    fn korean_partial_syllable_match() {
        // `삼성` should hit both `삼성전자` and `삼성 전자` via the
        // plain trigram path.
        let store = korean_store();
        let result = search(&store, "삼성", &SearchScope::All, None, None).unwrap();
        let ids: Vec<i64> = result.items.iter().map(|i| i.message_id).collect();
        assert!(ids.contains(&1), "expected 삼성전자 row, got {ids:?}");
        assert!(ids.contains(&2), "expected 삼성 전자 row, got {ids:?}");
    }

    #[test]
    fn korean_whitespace_insensitive_match() {
        // `삼성전자` should match the row that has a space inserted.
        let store = korean_store();
        let result = search(&store, "삼성전자", &SearchScope::All, None, None).unwrap();
        let ids: Vec<i64> = result.items.iter().map(|i| i.message_id).collect();
        assert!(ids.contains(&1), "expected exact 삼성전자 row, got {ids:?}");
        assert!(
            ids.contains(&2),
            "expected spaced 삼성 전자 row via nospace index, got {ids:?}"
        );
    }

    #[test]
    fn korean_bare_jamo_query() {
        // `ㅅㅏㅁ` should match `삼` via the jamo index.
        let store = korean_store();
        let result = search(&store, "ㅅㅏㅁ", &SearchScope::All, None, None).unwrap();
        let ids: Vec<i64> = result.items.iter().map(|i| i.message_id).collect();
        assert!(
            ids.contains(&1) || ids.contains(&2),
            "expected 삼-prefixed row via jamo, got {ids:?}"
        );
    }

    #[test]
    fn korean_short_jamo_like_fallback() {
        let store = korean_store();
        let result = search(&store, "ㅅㅏ", &SearchScope::All, None, None).unwrap();
        let ids: Vec<i64> = result.items.iter().map(|i| i.message_id).collect();
        assert!(
            ids.contains(&1) || ids.contains(&2),
            "expected short jamo LIKE fallback to hit 삼성 rows, got {ids:?}"
        );
    }

    #[test]
    fn plan_branches_falls_back_to_like_for_short_queries() {
        // One-char query should skip FTS so the LIKE path can handle it.
        assert!(build_match_query("a").is_none());
        assert!(build_match_query("ㅅ").is_none());
    }
}
