# 텔레그램 한국어 검색 — Architecture Design Document

> Local-first Telegram Korean/English search system
> Version: 2.0
> Date: 2026-02-12

---

## 1. Overview

### 1.1 Purpose

텔레그램 한국어 검색 is a local-only search tool for Telegram messages that supports Korean partial search and whitespace-agnostic search. Users open a mini search panel via a global hotkey while using Telegram Desktop, search instantly, and selecting a result returns them to Telegram Desktop.

### 1.2 Core Principles

- **Local-only**: All data exists solely on the user's local machine. No external transmission.
- **Non-intrusive to Desktop**: No access to Telegram Desktop internals in any form.
- **Low-resource**: Idle by default. Operates briefly only when needed.
- **Instant search**: Open panel -> type -> view results -> return to Telegram within seconds.

---

## 2. System Architecture

### 2.1 Overall Structure

```
┌──────────────────────────────────────────────────────────┐
│              텔레그램 한국어 검색 App                        │
│                                                          │
│  ┌────────────┐    ┌────────────┐    ┌────────────┐      │
│  │  Tauri UI   │◄──►│  Core Engine │◄──►│  SQLite DB  │  │
│  │ (WebView)  │    │   (Rust)   │    │  + FTS5    │      │
│  └────────────┘    └─────┬──────┘    └────────────┘      │
│                          │                               │
│                    ┌─────▼──────┐                         │
│                    │  Telegram   │                         │
│                    │  Collector  │                         │
│                    │ (grammers)  │                         │
│                    └─────┬──────┘                         │
│                          │ MTProto                        │
└──────────────────────────┼───────────────────────────────┘
                           │
                    ┌──────▼──────┐
                    │  Telegram   │
                    │   Servers   │
                    └─────────────┘
```

### 2.2 Module Structure

| Module | Role | Technology |
|--------|------|------------|
| **UI Layer** | Search panel, login flow | Tauri v2 + React + TypeScript |
| **Core Engine** | Search, sync orchestration | Rust |
| **Collector** | Telegram message collection (MTProto) | Rust + grammers |
| **Search** | FTS5 trigram full-text search with LIKE fallback | SQLite FTS5 |
| **Store** | Data storage/retrieval | SQLite (`sqlite` crate) |
| **Linker** | Search result -> Telegram deep link generation | Rust |

### 2.3 Data Flow

```
[Telegram Servers]
       │ MTProto (grammers)
       ▼
[Collector] ── fetch messages ──► [Store] ── save to messages table
                                              │ (FTS5 auto-indexed on insert)
                                              ▼
[UI] ── search query ──► [Core Engine] ── FTS5 MATCH / LIKE ──► [Store]
                           │
                           ▼
                    [results + deep links] ──► [UI] ──► [Telegram Desktop]
```

---

## 3. Data Model

### 3.1 SQLite Schema

```sql
-- Chat information
CREATE TABLE chats (
    chat_id       INTEGER PRIMARY KEY,
    title         TEXT NOT NULL,
    chat_type     TEXT NOT NULL CHECK (chat_type IN ('group', 'supergroup', 'channel')),
    username      TEXT,              -- @username if public
    access_hash   INTEGER,           -- for MTProto access
    is_excluded   INTEGER NOT NULL DEFAULT 0,  -- user exclusion setting
    created_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Message originals
CREATE TABLE messages (
    message_id    INTEGER NOT NULL,
    chat_id       INTEGER NOT NULL,
    timestamp     INTEGER NOT NULL,  -- Unix epoch seconds
    text_plain    TEXT NOT NULL,      -- plain text (formatting removed)
    text_stripped TEXT NOT NULL,      -- whitespace-removed version
    link          TEXT,               -- tg:// or https://t.me/... deep link
    PRIMARY KEY (chat_id, message_id),
    FOREIGN KEY (chat_id) REFERENCES chats(chat_id)
);

CREATE INDEX idx_messages_timestamp ON messages (timestamp DESC);
CREATE INDEX idx_messages_chat_timestamp ON messages (chat_id, timestamp DESC);

-- FTS5 trigram index for full-text search
-- Automatically maintained: new rows indexed on insert via insert_messages_batch()
CREATE VIRTUAL TABLE messages_fts USING fts5(
    text_plain,
    content='messages',
    tokenize='trigram case_sensitive 0'
);

-- Sync/meta state
CREATE TABLE sync_state (
    chat_id           INTEGER PRIMARY KEY,
    last_message_id   INTEGER NOT NULL DEFAULT 0,   -- incremental sync cursor
    oldest_message_id INTEGER,                       -- initial collection depth tracking
    initial_done      INTEGER NOT NULL DEFAULT 0,    -- whether initial collection is complete for this chat
    last_sync_at      TEXT,                           -- ISO 8601
    FOREIGN KEY (chat_id) REFERENCES chats(chat_id)
);

-- App global meta
CREATE TABLE app_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
-- Example keys: 'schema_version', 'tg_api_id', 'tg_api_hash', 'tg_authenticated'
```

### 3.2 Schema Versioning

Schema migrations are tracked via `app_meta.schema_version`:

| Version | Changes |
|---------|---------|
| 1 | Base tables (chats, messages, sync_state, app_meta) |
| 2 | Add `messages_fts` FTS5 virtual table, drop legacy `index_terms` and `postings` tables |

Migrations run automatically on startup and are idempotent.

### 3.3 Cursor-based Pagination

Use cursors instead of offsets for infinite scroll:

```sql
-- FTS5 search (terms >= 3 chars)
SELECT m.message_id, m.chat_id, m.timestamp, m.text_plain, m.link, c.title
FROM messages m
JOIN chats c ON m.chat_id = c.chat_id
WHERE m.rowid IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?)
  AND c.is_excluded = 0
ORDER BY m.timestamp DESC, m.chat_id ASC, m.message_id ASC
LIMIT 30;

-- Next page (cursor: last_timestamp, last_chat_id, last_message_id)
... AND (m.timestamp < :last_ts
         OR (m.timestamp = :last_ts AND m.chat_id > :last_chat_id)
         OR (m.timestamp = :last_ts AND m.chat_id = :last_chat_id AND m.message_id > :last_msg_id))
ORDER BY m.timestamp DESC
LIMIT 30;

-- LIKE fallback (terms < 3 chars, where FTS5 trigram cannot produce trigrams)
SELECT m.message_id, m.chat_id, m.timestamp, m.text_plain, m.link, c.title
FROM messages m
JOIN chats c ON m.chat_id = c.chat_id
WHERE m.text_plain LIKE '%' || ? || '%'
  AND c.is_excluded = 0
ORDER BY m.timestamp DESC
LIMIT 30;
```

---

## 4. Telegram Collection (Collector)

### 4.1 Authentication Flow

```
[First app launch]
     │
     ▼
[Unified login form: API ID + API Hash + Phone number]
     │ User input → "Send Code"
     ▼
[grammers: connect → request_login_code]
     │
     ▼
[Verification code input screen]
     │ User input
     ▼
[grammers: sign_in]
     │
     ├─── Success ──► [Search page + Start background collection]
     │
     └─── 2FA required ──► [Password input screen]
                              │ User input
                              ▼
                        [grammers: check_password]
                              │
                              ▼
                        [Search page + Start background collection]
```

Session management:
- Session file: `~/Library/Application Support/telegram-korean-search/session.bin`
- Encryption: AES key stored in macOS Keychain, session file encrypted with AES-256-GCM
- Auto-login via session file on app restart (no re-authentication needed)
- Stale session recovery: if `AUTH_KEY_UNREGISTERED` is detected, the session file is deleted and a fresh connection is established
- `tg_authenticated` flag in `app_meta` tracks whether login was completed (prevents reusing partial sessions)
- Graceful shutdown: Telegram runner is aborted on app exit to prevent stale session files

Login UX:
- API credentials and phone number are entered in a single unified form
- Saved credentials are pre-filled on subsequent logins
- Friendly error messages for common errors (invalid phone, wrong code, flood wait, etc.)
- Back button allows returning to the login form from code/2FA steps

### 4.2 Initial Collection (Non-blocking)

Collection runs in a background thread and does **not** block the search UI. The store lock is held only for brief DB writes, never during network I/O, so search and other commands remain responsive.

```
Phase 1: Fetch chat list from network (no store lock)
         ├─ Exclude DMs (groups/supergroups/channels only)
         ├─ Brief lock: save to chats table
         └─ Search-ready immediately

Phase 2: Fetch messages per chat (newest → oldest)
         ├─ Fetch from network (no store lock)
         ├─ Brief lock: save batch to messages table (FTS5 auto-indexed)
         └─ Rate limit: 400ms delay between chats
```

UI display: Non-blocking sync bar on the search page showing "Syncing: chat_name (3/15)"

### 4.3 Incremental Sync

Trigger: When panel opens, if `global_last_sync` is more than 5 minutes old.

```
1. Query active chat list (chats WHERE is_excluded = 0)
2. For each chat:
   - Fetch messages after sync_state.last_message_id (network, no lock)
   - Brief lock: save new messages (FTS5 auto-indexed)
   - Update last_message_id, last_sync_at
3. Update global_last_sync
```

Collection runs asynchronously in the background. Search is immediately available using the previous snapshot.

### 4.4 FLOOD_WAIT Handling

```rust
match collector.fetch_messages(...).await {
    Err(FloodWait(seconds)) => {
        log::warn!("FLOOD_WAIT: {}s", seconds);
        sleep(Duration::from_secs(seconds)).await;
        // retry
    }
    ...
}
```

---

## 5. Search Engine

### 5.1 FTS5 Trigram Index

Search uses SQLite FTS5 with the `trigram` tokenizer. This provides substring matching for both Korean and English text without requiring language-specific tokenization (no morpheme analysis needed).

How it works:
- FTS5 trigram breaks text into overlapping 3-character sequences
- Query terms are matched against these trigrams
- Case-insensitive matching is enabled (`case_sensitive 0`)
- Indexing happens automatically when messages are inserted via `insert_messages_batch()`

Advantages over the previous custom inverted index:
- No external dependencies (removed `lindera` and `unicode-segmentation` crates)
- ~1000 fewer lines of code
- Native SQLite performance optimizations
- Automatic index maintenance (no separate indexing step)

### 5.2 Search Query Processing

```
User input: "삼성전자 주가"
                │
                ▼
        [Split by whitespace]
        → Tokens: ["삼성전자", "주가"]
                │
                ▼
        [Check minimum length]
        ├─ All tokens >= 3 chars → FTS5 MATCH query
        └─ Any token < 3 chars  → LIKE fallback
                │
                ▼
        [FTS5 path]
        Build query: "삼성전자" "주가"  (quoted terms, AND'd by default)
        Execute: SELECT ... WHERE messages_fts MATCH ?
                │
                ▼
        [LIKE fallback path]
        Execute: SELECT ... WHERE text_plain LIKE '%삼성%' AND text_plain LIKE '%주가%'
                │
                ▼
        [Sort by timestamp DESC + cursor pagination]
        → Return top 30 results
```

Multiple keywords use AND operation (FTS5 default behavior).

### 5.3 Search Highlighting

Display matching token positions in result preview (2-3 lines):

```
Original: "어제 삼성전자 주가가 크게 상승했습니다"
Search:   "삼성전자 주가"
Display:  "어제 [삼성전자] [주가]가 크게 상승했습니다"
```

Highlighting finds match positions using case-insensitive substring search in the original `text_plain` and wraps them with `<mark>` tags. Overlapping ranges are merged.

---

## 6. Deep Link Generation (Linker)

### 6.1 Link Rules

| Chat Type | Link Format | Message Jump |
|-----------|-------------|--------------|
| Public channel/group | `https://t.me/{username}/{msg_id}` | Supported |
| Private channel/group | `tg://privatepost?channel={channel_id}&post={msg_id}` | Supported (partially unstable) |

### 6.2 Link Generation Logic

```rust
fn build_link(chat_id: i64, username: Option<&str>, message_id: i64) -> String {
    match username {
        Some(u) if !u.is_empty() => {
            format!("https://t.me/{}/{}", u, message_id)
        }
        _ => {
            let channel_id = (-chat_id) - 1_000_000_000_000;
            format!("tg://privatepost?channel={}&post={}", channel_id, message_id)
        }
    }
}
```

### 6.3 Opening Links

Uses Tauri's shell plugin:

```rust
app.shell().open(&link, None)?;
```

---

## 7. UI Design

### 7.1 Screen Layout

```
┌─────────────────────────────────────────────────┐
│  텔레그램 한국어 검색                     ⚙️ [X]  │
├─────────────────────────────────────────────────┤
│  [Channel ▾ | All]     Current: #crypto_korea   │
├─────────────────────────────────────────────────┤
│  🔍 [Enter search term                        ] │
├─────────────────────────────────────────────────┤
│  ● Syncing: #stock_talk (3/15)                  │ ← non-blocking sync bar
├─────────────────────────────────────────────────┤
│                                                 │
│  #crypto_korea · 2025-02-10 14:32               │
│  Yesterday [Samsung Electronics] [stock price]  │
│  rose significantly...                          │
│                                                 │
│  ──────────────────────────────────────────────  │
│                                                 │
│  #stock_talk · 2025-02-09 09:15                 │
│  [Samsung Electronics] outlook after earnings   │
│  [stock price]...                               │
│                                                 │
│  ──────────────────────────────────────────────  │
│                                                 │
│  (Load more on scroll...)                       │
│                                                 │
└─────────────────────────────────────────────────┘
```

### 7.2 Screen Details

**Login Screen (Unified)**
- Single form: API ID, API Hash, Phone number
- Saved credentials pre-filled on return visits
- Verification code input (separate step)
- 2FA password input (if applicable)
- Friendly error messages
- Back button to return to login form
- Splash screen with spinner during connection

**Search Panel (Main)**
- Scope toggle: `Channel` / `All`
- Search input: auto-focus
- Result list: scrollable, infinite scroll (30 at a time)
- Result item: channel name, time, body preview (2-3 lines, keyword highlight)
- Result click -> open Telegram via deep link
- Non-blocking sync bar: shows collection progress without blocking search

**Settings Screen**
- Hotkey configuration
- Chat management (collection on/off toggle)
- Logout
- Data reset (delete DB + re-collect)
- App info/version

### 7.3 Global Hotkey

- Default: `Cmd+Shift+F`
- Uses Tauri's `GlobalShortcut` plugin
- Panel open: show app window + focus search input
- Panel close: `Esc` key hides window

### 7.4 Interaction Flow

```
[User: Using Telegram Desktop]
     │
     │ Cmd+Shift+F
     ▼
[Panel opens / Search input auto-focus]
     │
     │ (Background: start incremental sync if last_sync > 5 min)
     │
     │ Type search query
     ▼
[Execute search (debounce 200ms)]
     │
     ▼
[Display results (newest first, 30 items)]
     │
     │ Scroll → load 30 more
     │
     │ Click result
     ▼
[Open deep link → Activate Telegram Desktop]
     │
     ▼
[Panel hides (Esc)]
```

---

## 8. Sync Policy

### 8.1 Sync Trigger Matrix

| Event | Action | Condition |
|-------|--------|-----------|
| First app launch | Start initial collection | No messages collected |
| App restart with valid session | Auto-login + start collection | `tg_authenticated = 1` |
| App restart with stale session | Delete session, show login | AUTH_KEY_UNREGISTERED |
| Panel open | Run incremental sync | `global_last_sync` > 5 minutes ago |
| Panel open | Skip sync | `global_last_sync` < 5 minutes ago |
| Background | None | - |

### 8.2 Incremental Sync Details

```
Max 100 messages per chat
     │
     ├─ New messages: save to messages (FTS5 auto-indexed)
     ├─ New chat discovered: add to chats + start collecting
     └─ Collection complete: update sync_state
```

Search responds immediately with the previous snapshot.

### 8.3 Chat Exclusion Handling

- Set `is_excluded = 1` in settings
- Excluded chats are removed from incremental sync targets
- Excluded chats are filtered out in search results via `WHERE c.is_excluded = 0`

---

## 9. Performance Design

### 9.1 Goals

| Metric | Target | Notes |
|--------|--------|-------|
| Search response | < 300ms (perceived) | FTS5 trigram + cursor pagination |
| Memory | < 100MB (normal usage) | SQLite cache + minimal resident |
| Disk | ~200MB for 100K messages | FTS5 trigram index is more compact than manual inverted index |
| Collection speed | Maximum within rate limits | 400ms delay between chats |

### 9.2 Optimization Strategy

**Search Optimization**
- FTS5 trigram index -> efficient substring matching
- Cursor pagination -> consistent performance even with deep scrolling
- LIKE fallback for short queries (< 3 chars) where FTS5 cannot produce trigrams
- Search debounce 200ms -> prevent unnecessary queries

**Collection Optimization**
- Non-blocking: store lock held only for brief DB writes, not during network I/O
- Batch inserts: 100 messages per chat in a single transaction
- WAL mode: read/write concurrency

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA cache_size = -64000;  -- 64MB cache
```

**Collection Rate Limiting**
- 400ms delay between chats
- FLOOD_WAIT: wait for specified duration then retry

---

## 10. File System Structure

```
~/Library/Application Support/telegram-korean-search/
├── session.bin              # Telegram session (AES-256-GCM encrypted)
├── tg-korean-search.db      # SQLite main DB (includes FTS5 index)
├── tg-korean-search.db-wal  # WAL file (auto-generated)
├── tg-korean-search.db-shm  # Shared memory (auto-generated)
└── config.json              # App settings (hotkeys etc., non-DB settings)
```

---

## 11. Error Handling

### 11.1 Error Classification

| Error Type | Handling | User Display |
|------------|----------|--------------|
| Network disconnection | Pause collection, resume on reconnect | Toast: "Check network connection" |
| FLOOD_WAIT | Wait for specified duration then retry | Show wait status in status bar |
| Session expired (AUTH_KEY_UNREGISTERED) | Delete session file, show login | Redirect to login with message |
| DB corruption | Guide DB deletion + re-collection | Dialog: "Data reset required" |
| Deep link failure | No fallback | Toast: "Unable to open message" |
| Collection failure | Log warning, continue with next chat | None (silent, shown in sync bar) |

### 11.2 Friendly Error Messages

Common Telegram API errors are translated to user-friendly messages:

| API Error | User Message |
|-----------|-------------|
| PHONE_NUMBER_INVALID | "Invalid phone number. Please include the country code (e.g. +82)." |
| PHONE_CODE_INVALID | "Incorrect verification code. Please try again." |
| PHONE_CODE_EXPIRED | "Verification code expired. Please request a new one." |
| PASSWORD_HASH_INVALID | "Incorrect password. Please try again." |
| FLOOD_WAIT | "Too many attempts. Please wait a few minutes and try again." |
| AUTH_KEY_UNREGISTERED | "Session expired. Please log in again." |

### 11.3 Logging

- Development mode: `RUST_LOG=debug` -> file + console log
- Production: `flexi_logger` with file rotation
- Users see only inline error messages and sync bar status

---

## 12. Updates

- Uses Tauri `tauri-plugin-updater` (planned)
- Auto update check based on GitHub Releases
- Notification when update available -> install after user approval

---

## 13. Security Considerations

| Item | Measure |
|------|---------|
| Session file | AES-256-GCM encryption, key in macOS Keychain |
| DB file | No encryption (local file, protected by macOS file permissions) |
| API ID/Hash | Stored in DB (user-provided via login form) |
| Memory | Session tokens zeroed after use |
| Network | MTProto encryption (provided by grammers) |
| Grammers panics | Suppressed via custom panic hook for stale session errors |

---

## 14. Dependencies

### 14.1 Rust Crates

| Crate | Purpose | Notes |
|-------|---------|-------|
| `tauri` | App framework | v2 |
| `grammers-client` | Telegram MTProto client | Session management, message fetch |
| `grammers-mtsender` | MTProto transport | |
| `grammers-session` | Session persistence | |
| `sqlite` | SQLite bindings | FTS5 support |
| `serde` / `serde_json` | Serialization | |
| `tokio` | Async runtime | grammers dependency |
| `tauri-plugin-global-shortcut` | Global hotkey | |
| `tauri-plugin-shell` | Open deep links | |
| `aes-gcm` | Session encryption | |
| `security-framework` | macOS Keychain access | |
| `flexi_logger` | Logging with file rotation | |
| `log` | Logging facade | |

### 14.2 Frontend

| Library | Purpose |
|---------|---------|
| React + TypeScript | UI rendering |
| Vite | Build tooling |
| `@tauri-apps/api` | Rust backend calls |
| Bun | Package manager |

---

## 15. Out of Scope

- Telegram Desktop plugin/hooking/overlay/packet interception
- Cloud sync, collaboration, multi-device shared index
- Message edit/delete history tracking
- Relevance-based ranking
- 1:1 DM search
- Media/file search (text only)

---

## 16. Future Expansion (v2+)

- DM search support (considering deep link limitations)
- Relevance-based ranking (TF-IDF etc.)
- Media message metadata search
- Chat grouping/tagging
- Search history / bookmarks
