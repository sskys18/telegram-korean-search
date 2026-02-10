pub mod ngram;
pub mod tokenizer;

use crate::store::Store;

/// Index a single message into the store's inverted index.
/// Follows the 3-step pipeline:
///   1. Morpheme tokens from original text → 'token'
///   2. Per-token bigrams → 'ngram'
///   3. Bigrams from whitespace-stripped morphemes → 'stripped_ngram'
pub fn index_message(
    store: &Store,
    chat_id: i64,
    message_id: i64,
    timestamp: i64,
    text: &str,
    text_stripped: &str,
) -> Result<(), sqlite::Error> {
    if text.trim().is_empty() {
        return Ok(());
    }

    let tokenizer = tokenizer::Tokenizer::new();

    // Step 1: Morpheme tokens from original text
    let tokens = tokenizer.tokenize(text);
    for token in &tokens {
        let term_id = store.insert_or_get_term(token, "token")?;
        store.insert_posting(term_id, chat_id, message_id, timestamp)?;
    }

    // Step 2: Per-token bigrams
    for token in &tokens {
        let bigrams = ngram::bigrams(token);
        for bg in bigrams {
            let term_id = store.insert_or_get_term(&bg, "ngram")?;
            store.insert_posting(term_id, chat_id, message_id, timestamp)?;
        }
    }

    // Step 3: Whitespace-stripped text → morphemes → bigrams
    if !text_stripped.is_empty() {
        let stripped_tokens = tokenizer.tokenize(text_stripped);
        let joined: String = stripped_tokens.join("");
        let stripped_bigrams = ngram::bigrams(&joined);
        for bg in stripped_bigrams {
            let term_id = store.insert_or_get_term(&bg, "stripped_ngram")?;
            store.insert_posting(term_id, chat_id, message_id, timestamp)?;
        }
    }

    Ok(())
}

/// Index a batch of messages. Each entry is (chat_id, message_id, timestamp, text, text_stripped).
pub fn index_batch(
    store: &Store,
    messages: &[(i64, i64, i64, &str, &str)],
) -> Result<(), sqlite::Error> {
    for &(chat_id, message_id, timestamp, text, text_stripped) in messages {
        index_message(store, chat_id, message_id, timestamp, text, text_stripped)?;
    }
    Ok(())
}

/// Tokenize a search query into term groups for posting list lookup.
/// Returns a Vec where each element is a keyword's terms (token + ngram variants).
pub fn tokenize_query(query: &str) -> Vec<String> {
    let tokenizer = tokenizer::Tokenizer::new();
    let tokens = tokenizer.tokenize(query);
    if tokens.is_empty() {
        // Fallback: generate bigrams from the raw query
        let stripped: String = query.chars().filter(|c| !c.is_whitespace()).collect();
        if stripped.is_empty() {
            return vec![];
        }
        return ngram::bigrams(&stripped);
    }
    tokens
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
                title: "Test Chat".to_string(),
                chat_type: "supergroup".to_string(),
                username: None,
                access_hash: None,
                is_excluded: false,
            })
            .unwrap();
    }

    fn insert_msg(store: &Store, msg_id: i64, text: &str) {
        store
            .insert_messages_batch(&[MessageRow {
                message_id: msg_id,
                chat_id: 1,
                timestamp: 1000 + msg_id,
                text_plain: text.to_string(),
                text_stripped: strip_whitespace(text),
                link: None,
            }])
            .unwrap();
    }

    #[test]
    fn test_index_korean_message() {
        let store = test_store();
        setup(&store);
        insert_msg(&store, 1, "삼성전자 주가가 상승했다");

        let text = "삼성전자 주가가 상승했다";
        let stripped = strip_whitespace(text);
        index_message(&store, 1, 1, 1001, text, &stripped).unwrap();

        // Should have terms in the index
        assert!(store.term_count().unwrap() > 0);
        assert!(store.posting_count().unwrap() > 0);
    }

    #[test]
    fn test_index_english_message() {
        let store = test_store();
        setup(&store);
        insert_msg(&store, 1, "Hello World Test");

        index_message(&store, 1, 1, 1001, "Hello World Test", "helloworldtest").unwrap();

        assert!(store.term_count().unwrap() > 0);
        assert!(store.posting_count().unwrap() > 0);

        // English tokens should be lowercased
        let ids = store.get_term_ids("hello").unwrap();
        assert!(!ids.is_empty());
    }

    #[test]
    fn test_index_empty_message() {
        let store = test_store();
        setup(&store);

        index_message(&store, 1, 1, 1001, "", "").unwrap();
        assert_eq!(store.term_count().unwrap(), 0);
    }

    #[test]
    fn test_index_mixed_language() {
        let store = test_store();
        setup(&store);
        insert_msg(&store, 1, "텔레그램 search test");

        index_message(
            &store,
            1,
            1,
            1001,
            "텔레그램 search test",
            "텔레그램searchtest",
        )
        .unwrap();

        assert!(store.term_count().unwrap() > 0);
    }

    #[test]
    fn test_index_batch() {
        let store = test_store();
        setup(&store);
        insert_msg(&store, 1, "first message");
        insert_msg(&store, 2, "second message");

        let batch = vec![
            (1i64, 1i64, 1001i64, "first message", "firstmessage"),
            (1, 2, 1002, "second message", "secondmessage"),
        ];
        index_batch(&store, &batch).unwrap();

        assert!(store.posting_count().unwrap() > 0);
    }

    #[test]
    fn test_tokenize_query_korean() {
        let tokens = tokenize_query("삼성전자 주가");
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_tokenize_query_english() {
        let tokens = tokenize_query("hello world");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
    }

    #[test]
    fn test_tokenize_query_empty() {
        let tokens = tokenize_query("");
        assert!(tokens.is_empty());
    }
}
