//! End-to-end smoke tests for the IPC server. Spins up a real
//! `SidecarServer` on a per-test socket, connects, and verifies a
//! ping → pong handshake plus an index + search round-trip.
//!
//! Each test uses a unique socket path under the process's temp dir
//! so parallel cargo test runs don't collide.

use std::path::PathBuf;

use seoyu::ipc::handlers::SidecarState;
use seoyu::ipc::{codec, SidecarServer};
use seoyu::store::Store;
use serde_json::{json, Value};

fn unique_socket_path(tag: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("seoyu-test-{tag}-{pid}-{nanos}.sock"))
}

fn unique_db_path(tag: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("seoyu-test-{tag}-{pid}-{nanos}.db"))
}

async fn connect_and_call(socket: &std::path::Path, request: Value) -> Value {
    let mut stream = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect");
    let body = serde_json::to_vec(&request).expect("encode request");
    codec::write_frame(&mut stream, &body).await.expect("write");
    let frame = codec::read_frame(&mut stream)
        .await
        .expect("read")
        .expect("frame");
    serde_json::from_slice(&frame).expect("decode response")
}

#[tokio::test]
async fn ping_returns_pong() {
    let socket = unique_socket_path("ping");
    let db = unique_db_path("ping");
    let store = Store::open(&db).expect("open store");
    let (server, _events) = SidecarServer::bind(&socket, SidecarState::new(store)).expect("bind");

    let server_handle = tokio::spawn(server.run());

    let resp = connect_and_call(&socket, json!({ "id": 1, "method": "ping" })).await;
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["version"], env!("CARGO_PKG_VERSION"));

    // Cleanly stop the server.
    let shutdown = connect_and_call(&socket, json!({ "id": 2, "method": "shutdown" })).await;
    assert_eq!(shutdown["id"], 2);
    let _ = server_handle.await;
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&db);
}

#[tokio::test]
async fn index_then_search_round_trip() {
    let socket = unique_socket_path("search");
    let db = unique_db_path("search");

    // Pre-seed the store with a chat so insert_messages_batch can
    // succeed; search would return nothing for a chat that doesn't
    // exist yet.
    {
        let store = Store::open(&db).expect("open store");
        store
            .upsert_chat(&seoyu::store::chat::ChatRow {
                chat_id: 7,
                title: "Test Chat".into(),
                chat_type: "channel".into(),
                username: None,
                access_hash: None,
                is_excluded: false,
            })
            .expect("seed chat");
    }
    let store = Store::open(&db).expect("open store");
    let (server, _events) = SidecarServer::bind(&socket, SidecarState::new(store)).expect("bind");
    let server_handle = tokio::spawn(server.run());

    let index = connect_and_call(
        &socket,
        json!({
            "id": 10,
            "method": "index_messages_batch",
            "params": {
                "messages": [
                    {
                        "chat_id": 7,
                        "message_id": 100,
                        "sender_id": null,
                        "sender_name": null,
                        "timestamp": 1_700_000_000,
                        "text": "삼성전자 주가 상승"
                    },
                    {
                        "chat_id": 7,
                        "message_id": 101,
                        "sender_id": null,
                        "sender_name": null,
                        "timestamp": 1_700_000_100,
                        "text": "apple unrelated"
                    }
                ]
            }
        }),
    )
    .await;
    assert_eq!(index["id"], 10);
    assert_eq!(index["result"]["inserted"], 2);
    assert_eq!(index["result"]["updated"], 0);

    let search = connect_and_call(
        &socket,
        json!({
            "id": 11,
            "method": "search",
            "params": { "query": "삼성전자" }
        }),
    )
    .await;
    assert_eq!(search["id"], 11);
    let items = search["result"]["items"].as_array().expect("items array");
    assert_eq!(items.len(), 1, "exactly one match expected, got {items:?}");
    assert_eq!(items[0]["message_id"], 100);

    let _ = connect_and_call(&socket, json!({ "id": 99, "method": "shutdown" })).await;
    let _ = server_handle.await;
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_file(&db);
}
