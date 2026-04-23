//! Exercises the public UniFFI surface as if the Swift shell were
//! calling it. UniFFI does not generate anything special that only
//! Swift can hit — the `#[uniffi::export]` impl is ordinary Rust
//! that any caller can reach, so this test catches regressions in
//! the FFI types and in the wiring that forwards to the core
//! modules.

use seoyu::uniffi_api::{ChatInfo, IndexedMessage, SearchScope, Seoyu};

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

    let count = seoyu
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
    assert_eq!(count, 2);

    let page = seoyu
        .search("삼성".into(), SearchScope::All, 30, None)
        .expect("search");
    let ids: Vec<i64> = page.items.iter().map(|h| h.message_id).collect();
    assert!(
        ids.contains(&1),
        "expected 삼성전자 row via partial match, got {ids:?}"
    );

    let chosung_page = seoyu
        .search("ㅅㅅㅈㅈ".into(), SearchScope::All, 30, None)
        .expect("chosung search");
    let chosung_ids: Vec<i64> = chosung_page.items.iter().map(|h| h.message_id).collect();
    assert!(
        chosung_ids.contains(&1),
        "expected 삼성전자 via chosung, got {chosung_ids:?}"
    );

    let _ = std::fs::remove_file(&path);
}
