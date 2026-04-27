//! Exercises the public UniFFI surface as if the Swift shell were
//! calling it. UniFFI does not generate anything special that only
//! Swift can hit — the `#[uniffi::export]` impl is ordinary Rust
//! that any caller can reach, so this test catches regressions in
//! the FFI types and in the wiring that forwards to the core
//! modules.

use seoyu::uniffi_api::{ChatInfo, IndexedMessage, MessageRef, SearchScope, Seoyu};

fn tmp_db(tag: &str) -> String {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir()
        .join(format!("seoyu-uniffi-{tag}-{pid}-{nanos}.db"))
        .to_string_lossy()
        .into_owned()
}

#[test]
fn constructor_and_version() {
    let path = tmp_db("version");
    let seoyu = Seoyu::new(path.clone()).expect("open");
    assert_eq!(seoyu.version(), env!("CARGO_PKG_VERSION"));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn index_then_korean_search_round_trip() {
    let path = tmp_db("search");
    let seoyu = Seoyu::new(path.clone()).expect("open");

    seoyu
        .upsert_chat(ChatInfo {
            chat_id: 42,
            title: "Test".into(),
            chat_type: "channel".into(),
            username: None,
            access_hash: None,
            is_excluded: false,
        })
        .expect("upsert");

    let outcome = seoyu
        .index_messages(vec![
            IndexedMessage {
                chat_id: 42,
                message_id: 1,
                timestamp: 1_700_000_000,
                text: "삼성전자 실적 발표".into(),
                link: None,
            },
            IndexedMessage {
                chat_id: 42,
                message_id: 2,
                timestamp: 1_700_000_100,
                text: "apple unrelated".into(),
                link: None,
            },
        ])
        .expect("index");
    assert_eq!(outcome.inserted, 2);
    assert_eq!(outcome.updated, 0);

    let page = seoyu
        .search("삼성".into(), SearchScope::All, 30, None)
        .expect("search");
    let ids: Vec<i64> = page.items.iter().map(|h| h.message_id).collect();
    assert!(
        ids.contains(&1),
        "expected 삼성전자 row via partial match, got {ids:?}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn index_update_and_delete_round_trip() {
    let path = tmp_db("update-delete");
    let seoyu = Seoyu::new(path.clone()).expect("open");

    seoyu
        .upsert_chat(ChatInfo {
            chat_id: 77,
            title: "Edits".into(),
            chat_type: "channel".into(),
            username: None,
            access_hash: None,
            is_excluded: false,
        })
        .expect("upsert");

    let first = seoyu
        .index_messages(vec![IndexedMessage {
            chat_id: 77,
            message_id: 1,
            timestamp: 1_700_000_000,
            text: "old keyword".into(),
            link: None,
        }])
        .expect("insert");
    assert_eq!((first.inserted, first.updated), (1, 0));

    let second = seoyu
        .index_messages(vec![IndexedMessage {
            chat_id: 77,
            message_id: 1,
            timestamp: 1_700_000_001,
            text: "new keyword".into(),
            link: None,
        }])
        .expect("update");
    assert_eq!((second.inserted, second.updated), (0, 1));

    assert!(seoyu
        .search("old".into(), SearchScope::All, 30, None)
        .expect("old search")
        .items
        .is_empty());
    assert_eq!(
        seoyu
            .search("new".into(), SearchScope::All, 30, None)
            .expect("new search")
            .items
            .len(),
        1
    );

    assert_eq!(
        seoyu
            .delete_messages(vec![MessageRef {
                chat_id: 77,
                message_id: 1,
            }])
            .expect("delete"),
        1
    );
    assert!(seoyu
        .search("new".into(), SearchScope::All, 30, None)
        .expect("deleted search")
        .items
        .is_empty());

    let _ = std::fs::remove_file(&path);
}
