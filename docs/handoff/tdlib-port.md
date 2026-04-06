# TDLib Port — Handoff Document

## Why

The current Telegram client uses **grammers** (community Rust MTProto library). It has recurring session stability issues:
- "cannot commit - no transaction is active" panics on startup
- "readonly database" errors on session save
- "disk I/O error" from concurrent access
- Stale sessions require manual deletion of `telegram.session`

**TDLib** (Telegram Database Library) is Telegram's official C++ client library. It powers Telegram Desktop, Bot API, and handles 24K+ bots per instance. It would eliminate all session issues.

## Scope

**Only `src-tauri/src/collector/` changes.** Everything else is untouched:

| Module | Change? | Notes |
|--------|---------|-------|
| `collector/mod.rs` | **Rewrite** | TDLib client init instead of grammers |
| `collector/auth.rs` | **Rewrite** | TDLib auth flow (supports QR code) |
| `collector/messages.rs` | **Rewrite** | TDLib message fetching API |
| `collector/link.rs` | **Keep** | Deep link format is protocol-level, same regardless of client |
| `commands.rs` | **Minor edits** | Update connect/auth commands to use TDLib types |
| `lib.rs` | **Minor edits** | AppState client type changes |
| `store/` | **No change** | Messages are stored in our SQLite, not TDLib's |
| `search/` | **No change** | |
| `wiki/` | **No change** | |
| `security/` | **No change** | |
| Frontend | **No change** | Same Tauri commands, same types |

## Current Architecture (grammers)

```
AppState {
    client: TokioMutex<Option<grammers_client::Client>>,
    runner_handle: TokioMutex<Option<JoinHandle<()>>>,
}

Flow:
1. SqliteSession::open("telegram.session")
2. SenderPool::new(session, api_id) → Client + runner
3. runner.run() on tokio task (keeps connection alive)
4. Client.is_authorized() / Client.request_login_code() / etc.
5. fetch_chats() → iterate dialogs
6. fetch_messages() → get_messages with limit 100
```

### Current Files

- `collector/mod.rs` — `connect(api_id)` creates client + runner, `session_path()`, panic hook for grammers errors
- `collector/auth.rs` — `request_login_code()`, `sign_in()`, `check_password()`, `is_authorized()`
- `collector/messages.rs` — `fetch_chats()` iterates dialogs, `fetch_messages()` gets 100 latest per chat, `fetch_messages_with_retry()` exponential backoff on FLOOD_WAIT
- `collector/link.rs` — generates `t.me` and `tg://` deep links from chat_id/message_id

### Current Data Flow

```
Telegram API → grammers Client → MessageRow { message_id, chat_id, timestamp, text_plain, text_stripped, link }
                                → ChatRow { chat_id, title, chat_type, username, access_hash, is_excluded }
                                → Store.insert_messages_batch() / Store.upsert_chat()
```

## Target Architecture (TDLib)

### Rust TDLib Options

1. **`tdlib` crate** (crates.io) — thin FFI wrapper, low-level
2. **`rust-tdlib`** (github.com/antonio-antuan/rust-tdlib) — higher-level async API, recommended
3. **Build TDLib from source** — required for either crate

### Recommended: `rust-tdlib`

```toml
[dependencies]
rust-tdlib = "0.4"
```

Requires TDLib shared library installed:
```bash
# macOS
brew install tdlib
# Or build from source: https://github.com/tdlib/td#building
```

### New AppState

```rust
pub struct AppState {
    pub store: Mutex<Store>,
    pub tdlib: TokioMutex<Option<rust_tdlib::client::Client>>,
    // Remove: client, login_token, password_token, runner_handle
}
```

TDLib manages its own:
- Session persistence (encrypted local DB)
- Connection lifecycle (reconnect, keepalive)
- Auth state machine
- File downloads

### New Auth Flow

TDLib has a built-in auth state machine:
```
AuthorizationStateWaitTdlibParameters → set parameters
AuthorizationStateWaitPhoneNumber → send phone OR QR code
AuthorizationStateWaitCode → send verification code
AuthorizationStateWaitPassword → send 2FA password
AuthorizationStateReady → authenticated!
```

**QR code login** (new capability):
```rust
// Request QR code
client.request_qr_code_authentication(vec![]).await?;
// TDLib emits AuthorizationStateWaitOtherDeviceConfirmation with QR link
// User scans from phone → auto-transitions to Ready
```

### New Message Fetching

```rust
// Get chat list
let chats = client.get_chats(ChatList::Main, 1000).await?;

// Get messages for a chat
let messages = client.get_chat_history(chat_id, 0, 0, 100, false).await?;

// Convert to our MessageRow
for msg in messages {
    if let MessageContent::Text(text) = &msg.content {
        let row = MessageRow {
            message_id: msg.id,
            chat_id: msg.chat_id,
            timestamp: msg.date,
            text_plain: text.text.text.clone(),
            text_stripped: strip_whitespace(&text.text.text),
            link: generate_link(chat_id, msg.id, chat_username),
        };
    }
}
```

### TDLib Parameters

```rust
TdlibParameters {
    database_directory: "~/Library/Application Support/telegram-korean-search/tdlib",
    use_message_database: true,    // TDLib caches messages locally
    use_secret_chats: false,
    api_id,
    api_hash,
    system_language_code: "ko",
    device_model: "telegram-korean-search",
    application_version: env!("CARGO_PKG_VERSION"),
    use_file_database: false,      // We don't need file/photo downloads
    use_chat_info_database: true,
}
```

## Migration Steps

### Phase 1: Setup (30 min)
1. Install TDLib: `brew install tdlib`
2. Add `rust-tdlib` to Cargo.toml
3. Remove `grammers-*` crates from Cargo.toml
4. Verify `cargo check` compiles with the new dependency

### Phase 2: Rewrite collector/mod.rs (1 hour)
1. Replace `connect(api_id)` with TDLib client initialization
2. Set TDLib parameters (database_directory, api_id, api_hash, etc.)
3. Remove `SenderPool`, `SqliteSession`, panic hook
4. TDLib handles its own connection lifecycle — no runner thread needed

### Phase 3: Rewrite collector/auth.rs (1 hour)
1. Implement TDLib auth state machine handler
2. Map TDLib auth states to existing `AuthStep` enum (loading, login, code, 2fa, ready)
3. Add QR code login support (new!)
4. Remove grammers LoginToken/PasswordToken from AppState

### Phase 4: Rewrite collector/messages.rs (1 hour)
1. `fetch_chats()` → use `client.get_chats()` → map to `ChatRow`
2. `fetch_messages()` → use `client.get_chat_history()` → map to `MessageRow`
3. Keep the same `MessageRow`/`ChatRow` structs — store layer unchanged
4. FLOOD_WAIT handling is built into TDLib — remove manual retry logic

### Phase 5: Update commands.rs + lib.rs (30 min)
1. Update AppState to use TDLib client type
2. Update `connect_telegram` command
3. Update auth commands to use TDLib auth flow
4. Update `start_collection` to use new fetch functions
5. Keep all wiki commands unchanged

### Phase 6: Verify (30 min)
1. `cargo fmt && cargo clippy -- -D warnings && cargo test`
2. `cargo tauri dev` — test login, collection, search, wiki
3. Verify deep links still work
4. Verify wiki worker still processes messages

## Session Migration

Users with existing grammers sessions will need to re-login once:
- Delete old `telegram.session` file
- TDLib creates its own database in `tdlib/` subdirectory
- One-time QR code scan or phone login
- After that, TDLib manages sessions permanently

## Build System Changes

### macOS
```bash
brew install tdlib
```
Cargo.toml:
```toml
[dependencies]
rust-tdlib = "0.4"
# Remove all grammers-* crates
```

### CI/CD
- GitHub Actions: add TDLib install step
- Release build: bundle TDLib dylib or static link

### Cargo.toml diff
```diff
- grammers-client = "0.8"
- grammers-mtsender = "0.8"
- grammers-session = "0.8"
- grammers-tl-types = "0.8"
+ rust-tdlib = "0.4"
```

## Risk Assessment

| Risk | Mitigation |
|------|-----------|
| TDLib build complexity | Use `brew install tdlib` for dev, prebuilt for release |
| Binary size increase (~20-30MB) | Acceptable for desktop app |
| `rust-tdlib` crate maturity | Well-maintained, used in production bots |
| Breaking change to auth flow | Frontend auth steps map 1:1 to TDLib states |
| Message format differences | MessageRow struct stays same, only fetch logic changes |

## What This Enables

- **QR code login** — scan from phone, no API ID/hash entry needed
- **Stable sessions** — no more grammers panics
- **Automatic reconnection** — TDLib handles network drops
- **Incremental sync** — TDLib tracks what's been fetched
- **Media support** (future) — TDLib handles file downloads
- **Real-time updates** (future) — TDLib pushes new messages as they arrive
