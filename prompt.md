# telegram-korean-search — Architecture Design Document

> Local-first Telegram Korean/English search system
> Version: 1.0 Draft
> Date: 2025-02-10

---

## 1. Overview

### 1.1 Purpose

telegram-korean-search is a local-only search tool for Telegram messages that supports Korean partial search and whitespace-agnostic search. Users open a mini search panel via a global hotkey while using Telegram Desktop, search instantly, and selecting a result returns them to Telegram Desktop.

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
│                telegram-korean-search App                  │
│                                                          │
│  ┌────────────┐    ┌────────────┐    ┌────────────┐      │
│  │  Tauri UI   │◄──►│  Core Engine │◄──►│  SQLite DB  │  │
│  │ (WebView)  │    │   (Rust)   │    │            │      │
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
| **UI Layer** | Search panel, settings screen, login flow | Tauri + HTML/CSS/JS (or TS) |
| **Core Engine** | Search, indexing, sync orchestration | Rust |
| **Collector** | Telegram message collection (MTProto) | Rust + grammers |
| **Indexer** | Tokenization, n-gram generation, inverted index construction | Rust |
| **Store** | Data storage/retrieval | SQLite (rusqlite) |
| **Linker** | Search result -> Telegram deep link generation | Rust |

### 2.3 Data Flow

```
[Telegram Servers]
       │ MTProto (grammers)
       ▼
[Collector] ── message fetch ──► [Store] ── save to messages table
                                   │
                                   ▼
                              [Indexer] ── tokenize + n-gram ──► [Store] ── save to index tables
                                   │
                                   ▼
[UI] ── search query ──► [Core Engine] ── inverted index lookup ──► [Store]
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
    text_stripped TEXT NOT NULL,      -- whitespace-removed version (for whitespace-agnostic search)
    link          TEXT,               -- tg:// or https://t.me/... deep link
    PRIMARY KEY (chat_id, message_id),
    FOREIGN KEY (chat_id) REFERENCES chats(chat_id)
);

CREATE INDEX idx_messages_timestamp ON messages (timestamp DESC);
CREATE INDEX idx_messages_chat_timestamp ON messages (chat_id, timestamp DESC);

-- Inverted index: token/n-gram dictionary
CREATE TABLE index_terms (
    term_id       INTEGER PRIMARY KEY AUTOINCREMENT,
    term          TEXT NOT NULL UNIQUE,
    source_type   TEXT NOT NULL CHECK (source_type IN ('token', 'ngram', 'stripped_ngram'))
);

CREATE INDEX idx_terms_term ON index_terms (term);

-- Inverted index: posting list
CREATE TABLE postings (
    term_id       INTEGER NOT NULL,
    chat_id       INTEGER NOT NULL,
    message_id    INTEGER NOT NULL,
    timestamp     INTEGER NOT NULL,  -- denormalized for sorting/cursor
    PRIMARY KEY (term_id, timestamp DESC, chat_id, message_id),
    FOREIGN KEY (term_id) REFERENCES index_terms(term_id),
    FOREIGN KEY (chat_id, message_id) REFERENCES messages(chat_id, message_id)
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
-- Example keys: 'schema_version', 'index_version', 'global_last_sync', 'initial_bootstrap_done'
```

### 3.2 Index Version Management

Index swap approach during full re-indexing:

```
Currently active: index_v1 (index_terms, postings)
Re-indexing:      index_v2 (index_terms_v2, postings_v2) created separately
After completion: DROP v1 → RENAME v2 to v1
                  Update app_meta.index_version
```

Search always references the active index only. Search remains available during re-indexing.

### 3.3 Cursor-based Pagination

Use cursors instead of offsets for infinite scroll:

```sql
-- First page
SELECT m.chat_id, m.message_id, m.text_plain, m.timestamp, m.link, c.title
FROM postings p
JOIN messages m ON p.chat_id = m.chat_id AND p.message_id = m.message_id
JOIN chats c ON m.chat_id = c.chat_id
WHERE p.term_id IN (/* search tokens */)
  AND c.is_excluded = 0
ORDER BY m.timestamp DESC
LIMIT 30;

-- Next page (cursor: last_timestamp, last_chat_id, last_message_id)
... AND (m.timestamp < :last_ts
         OR (m.timestamp = :last_ts AND m.chat_id > :last_chat_id)
         OR (m.timestamp = :last_ts AND m.chat_id = :last_chat_id AND m.message_id > :last_msg_id))
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
[Phone number input screen]
     │ User input
     ▼
[grammers: send_code_request]
     │
     ▼
[SMS code input screen]
     │ User input
     ▼
[grammers: sign_in]
     │
     ├─── Success ──► [Main screen / Start initial collection]
     │
     └─── 2FA required ──► [Password input screen]
                              │ User input
                              ▼
                        [grammers: check_password]
                              │
                              ▼
                        [Main screen / Start initial collection]
```

Session management:
- Session file: `~/Library/Application Support/telegram-korean-search/session.bin`
- Encryption: AES key stored in macOS Keychain, session file encrypted with AES-256-GCM
- Auto-login via session file on app restart (no re-authentication needed)

### 4.2 Initial Collection (Progressive Bootstrap)

```
Phase 1: Fetch entire chat list
         ├─ Exclude DMs (groups/supergroups/channels only)
         ├─ Save to chats table
         └─ Search-ready immediately after completion

Phase 2: Collect messages per chat (newest → oldest)
         ├─ Priority: most recently active first
         ├─ 1 batch per chat = 100 messages
         ├─ All chats round 1 → round 2 (next 100) → ...
         ├─ Index immediately after each batch
         └─ Rate limit: 300~500ms delay between chats

Phase 3: All chat histories exhausted → initial_bootstrap_done = true
```

State tracking:
- `sync_state.oldest_message_id`: collection depth for each chat
- `sync_state.initial_done`: whether chat history end has been reached
- `app_meta.initial_bootstrap_done`: overall initial collection completion

App termination during collection → resumes from `oldest_message_id` on restart.

UI display: "Collection in progress (23/87 chats complete)" + progress bar

### 4.3 Incremental Sync

Trigger: When panel opens, if `global_last_sync` is more than 5 minutes old.

```
1. Query active chat list (chats WHERE is_excluded = 0)
2. For each chat:
   - Fetch messages after sync_state.last_message_id
   - Maximum 100 per chat (resource protection)
   - Save new messages + index immediately
   - Update last_message_id, last_sync_at
3. Update global_last_sync
```

Collection runs asynchronously in the background. Search is immediately available using the previous snapshot.
Latest data is reflected on the next panel open.

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

## 5. Search Engine (Indexer + Search)

### 5.1 Indexing Pipeline

Indexing process when a single message is stored:

```
Original: "Samsung Electronics stock price rose"
(Korean: "삼성 전자 주가가 상승했다")
            │
            ├──► [Korean language detection]
            │
            ▼
    ┌───────────────────────────────────┐
    │  Step 1: Original text tokenization│
    │  Morpheme processing: remove       │
    │  particles/endings                 │
    │  Result: ["삼성", "전자", "주가",   │
    │           "상승"]                  │
    │  source_type: 'token'             │
    └───────────────┬───────────────────┘
                    │
    ┌───────────────▼───────────────────┐
    │  Step 2: Per-token bigram          │
    │  generation                        │
    │  "삼성" → ["삼성"]                 │
    │  "전자" → ["전자"]                 │
    │  "주가" → ["주가"]                 │
    │  "상승" → ["상승"]                 │
    │  source_type: 'ngram'             │
    │  (2-char tokens: bigram = itself)  │
    └───────────────┬───────────────────┘
                    │
    ┌───────────────▼───────────────────┐
    │  Step 3: Whitespace-stripped text  │
    │  bigrams                           │
    │  "삼성전자주가가상승했다"             │
    │  → morpheme processing →           │
    │    "삼성전자주가상승"                │
    │  → bigrams: ["삼성", "성전", "전자",│
    │    "자주", "주가", "가상", "상승"]   │
    │  source_type: 'stripped_ngram'     │
    └───────────────┬───────────────────┘
                    │
                    ▼
    [Save to index_terms + postings tables]
```

### 5.2 Korean Morpheme Processing

The goal is "practical tokenization for search quality", not linguistic completeness.

Strategy:
- **Lightweight morpheme analysis**: `lindera` (Rust native, mecab-compatible dictionary)
  - POS tagging → extract nouns/numerals/proper nouns only
  - Remove particles (은/는/이/가/을/를/의/에/서/로 etc.)
  - Remove endings (-했다, -하는, -됐다 etc.)
- **Fallback**: If morpheme analysis fails, generate bigrams from the raw text

Example:
```
Input: "텔레그램에서 검색이 안됐다"
       ("Search didn't work on Telegram")
Morphemes: ["텔레그램", "검색"]  (에서/이/안/됐다 removed)
Bigrams: ["텔레", "레그", "그램", "검색"]
```

### 5.3 English Processing

- Whitespace-based tokenization
- Lowercase normalization
- Punctuation removal
- No n-grams (word-level search)
- Prefix matching: SQLite LIKE 'term%' (optional)

### 5.4 Search Query Processing

```
User input: "삼성전자 주가" ("Samsung Electronics stock price")
                │
                ▼
        [Language detection + tokenization]
        → Tokens: ["삼성전자", "주가"]
                │
                ▼
        [For each token]
        ├─ Exact token search (index_terms WHERE term = '삼성전자')
        ├─ Bigram search (ngram source_type)
        └─ Whitespace-stripped bigram search (stripped_ngram source_type)
                │
                ▼
        [Posting list intersection (AND)]
        → "삼성전자" matching message_ids ∩ "주가" matching message_ids
                │
                ▼
        [Sort by timestamp DESC + cursor pagination]
        → Return top 30 results
```

Multiple keywords use AND operation. Intersection of each keyword's posting list.

### 5.5 Search Highlighting

Display matching token positions in result preview (2-3 lines):

```
Original: "어제 삼성전자 주가가 크게 상승했습니다"
          ("Samsung Electronics stock price rose significantly yesterday")
Search:   "삼성전자 주가"
Display:  "어제 [삼성전자] [주가]가 크게 상승했습니다"
```

Highlighting calculates match positions in the original text_plain during UI rendering and wraps them with `<mark>` tags.

### 5.6 Indexing Modes

| Situation | Method | Description |
|-----------|--------|-------------|
| New message collection | Incremental indexing | Immediate tokenization → add to index_terms/postings |
| Schema/logic change | Full re-indexing | Create new index tables → swap |
| After app update | index_version comparison | Auto re-index on version mismatch |

---

## 6. Deep Link Generation (Linker)

### 6.1 Link Rules

| Chat Type | Link Format | Message Jump |
|-----------|-------------|--------------|
| Public channel/group | `https://t.me/{username}/{msg_id}` | Supported |
| Private channel/group | `tg://privatepost?channel={channel_id}&post={msg_id}` | Supported (partially unstable) |

### 6.2 Link Generation Logic

```rust
fn build_link(chat: &Chat, message_id: i64) -> String {
    match &chat.username {
        Some(username) => {
            // Public: https://t.me/username/msg_id
            format!("https://t.me/{}/{}", username, message_id)
        }
        None => {
            // Private: tg://privatepost?channel=ID&post=msg_id
            // channel_id: for supergroup/channel, remove -100 prefix from chat_id
            let channel_id = chat.chat_id.abs() - 1_000_000_000_000; // Telegram ID conversion
            format!("tg://privatepost?channel={}&post={}", channel_id, message_id)
        }
    }
}
```

### 6.3 Opening Links

Use the `open` command on macOS or Tauri's `shell::open` API:

```rust
tauri::api::shell::open(&app_handle.shell_scope(), &link, None)?;
```

---

## 7. UI Design

### 7.1 Screen Layout

```
┌─────────────────────────────────────────────────┐
│  telegram-korean-search                  ⚙️ [X]  │
├─────────────────────────────────────────────────┤
│  [Channel ▾ | All]     Current: #crypto_korea   │
├─────────────────────────────────────────────────┤
│  🔍 [Enter search term                        ] │
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
├─────────────────────────────────────────────────┤
│  Collection in progress: 45/87 chats ████░░ 52% │
└─────────────────────────────────────────────────┘
```

### 7.2 Screen Details

**Login Screen**
- Phone number input (country code selector)
- SMS code input
- 2FA password input (if applicable)
- Error display (wrong code, network error, etc.)

**Search Panel (Main)**
- Scope toggle: `Channel` / `All`
- Channel mode: display channel name + change button (type-to-search selection)
- Search input: auto-focus
- Result list: scrollable, infinite scroll (30 at a time)
- Result item: channel name, time, body preview (2-3 lines, keyword highlight)
- Result click → open Telegram + auto-close panel
- Bottom status bar: collection progress status (only during initial collection)

**Settings Screen**
- Hotkey configuration
- Chat management (collection on/off toggle)
- Logout
- Data reset (delete DB + re-collect)
- App info/version

**Channel Selection Modal**
- Text input → filter chat names (local chats table search)
- Selection locks scope to that channel
- Remembers last selected channel (stored in app_meta)

### 7.3 Global Hotkey

- Default: `Cmd+Shift+F`
- Customizable (change in settings, stored in app_meta)
- Uses Tauri's `GlobalShortcut` API
- Panel open: show app window + focus search input
- Panel close: auto on result click / Esc key / focus lost

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
[Panel auto-closes]
```

---

## 8. Sync Policy

### 8.1 Sync Trigger Matrix

| Event | Action | Condition |
|-------|--------|-----------|
| First app launch | Start initial collection | `initial_bootstrap_done = false` |
| App restart | Resume initial collection | `initial_bootstrap_done = false` |
| Panel open | Run incremental sync | `global_last_sync` > 5 minutes ago |
| Panel open | Skip sync | `global_last_sync` < 5 minutes ago |
| Background | None | - |
| After app update | index_version comparison → re-index | Version mismatch |

### 8.2 Incremental Sync Details

```
Max 100 messages per chat
     │
     ├─ New messages: save to messages + index immediately
     ├─ New chat discovered: add to chats + start collecting for that chat
     └─ Collection complete: update sync_state
```

Search responds immediately with the previous snapshot. Results are not refreshed after collection completes.
Latest data is reflected on the next panel open.

### 8.3 Chat Exclusion Handling

- Set `is_excluded = 1` in settings
- Excluded chats are removed from incremental sync targets
- Existing index is not deleted immediately (filtered only in search results)
- Excluded chat data is cleaned up during full re-indexing

---

## 9. Performance Design

### 9.1 Goals

| Metric | Target | Notes |
|--------|--------|-------|
| Search response | < 300ms (perceived) | Inverted index + cursor pagination |
| Memory | < 100MB (normal usage) | SQLite cache + minimal resident |
| Disk | 300~600MB for 100K messages | Including bigram + whitespace-stripped index |
| Collection speed | Maximum within rate limits | 300~500ms delay between chats |

### 9.2 Optimization Strategy

**Search Optimization**
- Inverted index based → O(1) token lookup
- Cursor pagination → consistent performance even with deep scrolling
- postings table PK includes timestamp DESC → zero sorting cost
- Search debounce 200ms → prevent unnecessary queries

**Index Optimization**
- Incremental indexing: transaction batch (100 messages per chat in a single transaction)
- WAL mode enabled: read/write concurrency

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA cache_size = -64000;  -- 64MB cache
```

**Collection Optimization**
- Initial collection: round-robin distribution across all chats
- FLOOD_WAIT: wait for specified duration then retry
- Network errors: exponential backoff (1s → 2s → 4s → ... max 60s)

---

## 10. File System Structure

```
~/Library/Application Support/telegram-korean-search/
├── session.bin              # Telegram session (AES-256-GCM encrypted)
├── tg-korean-search.db      # SQLite main DB
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
| Session expired | Prompt re-login | Show login screen |
| DB corruption | Guide DB deletion + re-collection | Dialog: "Data reset required" |
| Deep link failure | No fallback | Toast: "Unable to open message" |
| Indexing failure | Skip message, log | None (silent) |

### 11.2 Logging

- Development mode: `RUST_LOG=debug` → file + console log
- Production: errors only in file log (`tg-korean-search.log`, rotation 10MB x 3)
- Users see only toast notifications

---

## 12. Updates

- Uses Tauri `tauri-plugin-updater`
- Auto update check based on GitHub Releases
- Notification when update available → install after user approval
- After update, compare index_version → auto re-index if needed

---

## 13. Security Considerations

| Item | Measure |
|------|---------|
| Session file | AES-256-GCM encryption, key in macOS Keychain |
| DB file | No encryption (local file, protected by macOS file permissions) |
| API ID/Hash | Embedded in app binary (allowed per Telegram policy) |
| Memory | Session tokens zeroed after use |
| Network | MTProto encryption (provided by grammers) |

---

## 14. Dependencies

### 14.1 Rust Crates

| Crate | Purpose | Notes |
|-------|---------|-------|
| `tauri` | App framework | v2 |
| `grammers` | Telegram MTProto client | Session management, message fetch |
| `rusqlite` | SQLite bindings | WAL, FTS support |
| `lindera` | Korean morpheme analysis | mecab-compatible, IPADIC/ko-dic |
| `serde` / `serde_json` | Serialization | Config files etc. |
| `tokio` | Async runtime | grammers dependency |
| `tauri-plugin-global-shortcut` | Global hotkey | |
| `tauri-plugin-updater` | Auto updates | |
| `aes-gcm` | Session encryption | |
| `security-framework` | macOS Keychain access | |
| `log` + `env_logger` | Logging | |

### 14.2 Frontend

| Library | Purpose |
|---------|---------|
| Vanilla TS or Svelte (lightweight) | UI rendering |
| Tauri JS API | Rust backend calls |

---

## 15. Out of Scope

- Telegram Desktop plugin/hooking/overlay/packet interception
- Cloud sync, collaboration, multi-device shared index
- Message edit/delete history tracking
- Relevance-based ranking
- 1:1 DM search
- Media/file search (text only)
- Raycast integration (possible via future adapter)

---

## 16. Future Expansion (v2+)

- DM search support (considering deep link limitations)
- Relevance-based ranking (TF-IDF etc.)
- Media message metadata search
- Chat grouping/tagging
- Raycast adapter
- Search history / bookmarks
