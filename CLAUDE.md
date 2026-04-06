# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

텔레그램 한국어 검색 (telegram-korean-search) is a local-only Telegram message search + wiki app for macOS. It supports Korean partial/substring search and English search using SQLite FTS5 trigram tokenizer, and auto-generates a wiki knowledge base from collected messages using LLM. Built with Tauri v2 (Rust backend) + React/TypeScript frontend.

## Tech Stack

- **Backend**: Rust, Tauri v2
- **Frontend**: React + TypeScript + Vite
- **Package manager**: Bun (not npm)
- **Telegram client**: grammers (MTProto) — planned migration to TDLib (see `docs/handoff/tdlib-port.md`)
- **Search**: SQLite FTS5 trigram tokenizer, LIKE fallback for queries < 3 chars
- **LLM**: `codex exec` CLI (o4-mini for classification, gpt-5.4 for summaries)
- **Storage**: SQLite with WAL mode
- **Session security**: AES-256-GCM encryption, macOS Keychain

## Project Structure

```
src-tauri/src/
  commands.rs          # Tauri command handlers (auth, search, collection, wiki)
  lib.rs               # App state, initialization, graceful shutdown
  main.rs              # Entry point
  error.rs             # Error types
  logging.rs           # flexi_logger setup
  collector/
    mod.rs             # Telegram client connection + session management
    auth.rs            # Telegram authentication (login, 2FA)
    link.rs            # Deep link generation (tg:// and t.me URLs)
    messages.rs        # Message fetching from Telegram
  search/
    mod.rs             # Module root
    engine.rs          # FTS5 search engine
    highlight.rs       # Search result highlighting (backend)
  security/
    mod.rs             # Module root
    crypto.rs          # AES-256-GCM session encryption
    keychain.rs        # macOS Keychain integration
  store/
    mod.rs             # Module root
    schema.rs          # DB schema & migrations (currently v4)
    app_meta.rs        # App metadata (schema_version, etc.)
    chat.rs            # Chat CRUD operations
    message.rs         # Message CRUD & FTS5 indexing
    sync_state.rs      # Per-chat sync state tracking
    wiki_category.rs   # Dynamic categories with alias dedup (20 known alias groups)
    wiki_queue.rs      # Classification job queue (enqueue, atomic dequeue, crash recovery)
    wiki_topic.rs      # Topics, aliases, topic-message links, reconciliation
    wiki_page.rs       # Wiki pages, citations, FTS5 search, source_hash cache
    wiki_stats.rs      # Daily rollup tables, channel membership, trending queries
  wiki/
    mod.rs             # Module root
    llm.rs             # LLM via codex exec CLI (batch classify + summarize)
    worker.rs          # Background classification worker (batch of 20 per call)
    trending.rs        # Trending score calculation

src/
  main.tsx             # React entry point
  App.tsx              # Root component, tab routing (Search/Wiki)
  App.css              # Global styles (dark theme)
  api/
    tauri.ts           # Tauri invoke wrappers + event listeners
  components/
    TabBar.tsx         # Search/Wiki tab switcher
    ChannelSelector.tsx
    SearchBar.tsx
    ResultList.tsx     # Search result list with infinite scroll
    ResultItem.tsx     # Individual result — click opens MessageModal
    MessageModal.tsx   # Full message preview with "Open in Telegram" + "Don't show next time"
    wiki/
      TrendingDashboard.tsx  # Landing: trending topics + category filter
      TopicCard.tsx          # Individual topic card
      WikiArticle.tsx        # Bilingual article with citations
      SourceMessages.tsx     # Collapsible source message list
      CategoryFilter.tsx     # Category pill selector
      WikiSearch.tsx         # Wiki search bar + results
      WikiSettings.tsx       # Codex CLI status, worker controls
  hooks/
    useAuth.ts         # Auth state machine + .env auto-login
    useSearch.ts       # Search with debounce + cursor pagination
    useInfiniteScroll.ts
    useWiki.ts         # Wiki browsing, topic selection, category filter
    useWikiWorker.ts   # Worker status, progress events, controls
  pages/
    LoginPage.tsx      # Multi-step auth (API creds, phone, code, 2FA)
    SearchPage.tsx     # Search interface
    WikiPage.tsx       # Wiki tab container
  types/
    index.ts           # All TypeScript interfaces
  utils/
    format.ts          # Timestamp formatting
    highlight.ts       # Byte-offset to char-offset highlight conversion
    markdown.ts        # Simple markdown-to-HTML converter for wiki articles
```

## Build & Run

```bash
bun install          # Install frontend deps
cargo tauri dev      # Run in dev mode (login auto-fills from .env if present)
cargo tauri build    # Production build
```

### .env auto-login

Create `.env` in project root to skip manual credential entry:
```
TG_API_ID=12345678
TG_API_HASH=a1b2c3d4e5f6...
TG_PHONE=+821012345678
```
If all 3 values present, app auto-connects and goes straight to verification code step.

## Testing

```bash
cargo test                                  # Run all Rust tests
cargo fmt --check                           # Check formatting
cargo clippy -- -D warnings                 # Lint check
bun run build                               # Frontend type-check + build
```

- All checks must pass before pushing (CI runs cargo fmt/clippy/test)
- Run `cargo fmt` to auto-fix formatting issues

## Architecture Rules

- **Non-blocking collection**: Never hold `std::sync::Mutex<Store>` across `.await` points. Fetch from network first (no lock), then briefly lock to write to DB.
- **FTS5 auto-indexing**: Messages are automatically indexed into `messages_fts` on insert via `insert_messages_batch()`. No separate indexing step.
- **Search routing**: FTS5 MATCH for terms >= 3 chars, LIKE fallback for terms < 3 chars (trigram tokenizer needs at least 3 chars).
- **Cursor pagination**: Use `(timestamp, chat_id, message_id)` cursors, never OFFSET.
- **Telegram client**: Uses `TokioMutex<Option<Client>>` (not OnceCell) to support reconnection and stale session recovery.

## Key Patterns

- Tauri commands in `commands.rs` are the bridge between frontend and backend
- Frontend calls backend via `@tauri-apps/api` invoke
- `Store` is wrapped in `std::sync::Mutex` and shared via Tauri state
- Schema migrations are versioned via `app_meta.schema_version` (currently v4)
- Collection functions are split: `fetch_chats`/`fetch_messages` (network-only) + brief lock for DB writes
- Stale sessions detected by `AUTH_KEY_UNREGISTERED` error, auto-recovered by deleting session and reconnecting
- Background workers spawn `std::thread` with internal tokio runtime (see `run_collection` and wiki `start_worker`)

## Wiki Feature

- **LLM backend**: `codex exec` CLI subprocess (not direct API). No API keys needed — uses ChatGPT subscription via codex auth.
- **Classification model**: o4-mini (fast, ~3s/batch). **Summary model**: gpt-5.4 (quality).
- **Batch processing**: 20 messages per codex call. ~5,000 calls for 100K messages (~4-8 hours).
- **Categories are dynamic**: LLM picks freely, backend deduplicates via 20 known alias groups (EN/KO) + substring matching. New categories auto-create on first use.
- **Decoupled from sync**: Collection enqueues message IDs into `wiki_classify_queue`. Worker processes independently.
- **Message modal**: Clicking a search result shows full text in a modal. "Open in Telegram" button for deep link. "Don't show next time" checkbox (localStorage).

## Gotchas

- `cargo tauri dev` runs from `src-tauri/` directory, not project root. File lookups (like `.env`) must check parent dir.
- Stale `telegram.session` causes grammers panics ("cannot commit", "readonly database"). Fix: `rm ~/Library/Application\ Support/telegram-korean-search/telegram.session`
- ChatGPT OAuth tokens do NOT work with `api.openai.com/v1/chat/completions` (requires separate API billing). Use `codex exec` subprocess instead.
- Schema migrations are one-shot: changing migration code does NOT affect existing databases. To reset wiki data: `sqlite3 tg-korean-search.db "DELETE FROM wiki_categories; DELETE FROM wiki_topics; DELETE FROM wiki_classify_queue;"`
- Clippy enforces `format!()` not `&format!()`, and `.is_multiple_of(N)` instead of `% N == 0`.
- Empty chat titles (deleted accounts) are filtered out in `get_all_chats()`.

## Data Location

```
~/Library/Application Support/telegram-korean-search/
  telegram.session         # Telegram session (grammers SQLite)
  tg-korean-search.db      # SQLite DB (messages, FTS5 index, wiki tables)
```

## Release Process

- **Auto-release on PR merge**: When a PR is merged to `main`, the `auto-release.yml` workflow automatically bumps the version, commits it, and creates a git tag (which triggers the `release.yml` build).
- **PR labels control version bump**:
  - `major` label -> major bump (e.g., `0.2.0` -> `1.0.0`)
  - `minor` label -> minor bump (e.g., `0.2.0` -> `0.3.0`)
  - No label -> patch bump (e.g., `0.2.0` -> `0.2.1`)
- Version is stored in 3 files: `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` (all updated automatically by the workflow).
- Do not manually create tags or bump versions -- let the workflow handle it.

## Planned: TDLib Port

See `docs/handoff/tdlib-port.md` for the full handoff document. Summary:
- Replace grammers with TDLib for stable session management
- Scope: `src-tauri/src/collector/` only — search, wiki, frontend untouched
- Enables QR code login, better reconnection, complete API coverage

## Do Not

- Do not add `lindera` or `unicode-segmentation` -- removed in favor of FTS5 trigram
- Do not hold store mutex during network I/O -- causes UI freeze
- Do not use OFFSET pagination -- use cursor-based pagination
- Do not commit `.env`, `telegram.session`, `*.key`, or files in `.gitignore`
- Do not skip `cargo fmt` and `cargo clippy` before pushing
- Do not call OpenAI API directly -- use `codex exec` CLI for all LLM calls
- Do not hardcode wiki categories -- they are LLM-decided and auto-created
