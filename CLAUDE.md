# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

텔레그램 한국어 검색 (telegram-korean-search) is a local-only Telegram message search app for macOS. It supports Korean partial/substring search and English search using SQLite FTS5 trigram tokenizer. Built with Tauri v2 (Rust backend) + React/TypeScript frontend.

## Tech Stack

- **Backend**: Rust, Tauri v2
- **Frontend**: React + TypeScript + Vite
- **Package manager**: Bun (not npm)
- **Telegram client**: grammers (MTProto)
- **Search**: SQLite FTS5 trigram tokenizer, LIKE fallback for queries < 3 chars
- **Storage**: SQLite with WAL mode
- **Session security**: AES-256-GCM encryption, macOS Keychain

## Project Structure

```
src-tauri/src/
  commands.rs          # Tauri command handlers (auth, search, collection)
  lib.rs               # App state, initialization, graceful shutdown
  main.rs              # Entry point
  error.rs             # Error types
  logging.rs           # flexi_logger setup
  collector/
    mod.rs             # Module root
    auth.rs            # Telegram authentication logic
    link.rs            # Link/invite handling
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
    schema.rs          # DB schema & migrations
    app_meta.rs        # App metadata (schema_version, etc.)
    chat.rs            # Chat CRUD operations
    message.rs         # Message CRUD & FTS5 indexing
    sync_state.rs      # Per-chat sync state tracking
    wiki_category.rs   # Wiki category storage
    wiki_queue.rs      # Wiki classification queue storage
    wiki_topic.rs      # Wiki topic storage
    wiki_page.rs       # Wiki page storage
    wiki_stats.rs      # Wiki stats storage
  wiki/
    mod.rs             # Module root
    llm.rs             # codex exec integration
    worker.rs          # Background wiki processing worker
    trending.rs        # Wiki trending calculations

src/
  main.tsx             # React entry point
  App.tsx              # Root component, auth/search routing
  App.css              # Global styles
  api/
    tauri.ts           # Tauri invoke wrappers
  components/
    ChannelSelector.tsx
    SearchBar.tsx
    ResultList.tsx     # Search result list
    ResultItem.tsx     # Individual search result
    TabBar.tsx         # Top-level tab navigation
    wiki/              # Wiki UI components
  hooks/
    useAuth.ts
    useSearch.ts
    useInfiniteScroll.ts
    useWiki.ts
    useWikiWorker.ts
  pages/
    LoginPage.tsx
    SearchPage.tsx
    WikiPage.tsx
  types/
    index.ts           # TypeScript type definitions
  utils/
    format.ts          # Formatting helpers
    highlight.ts       # Search highlight utility
```

## Build & Run

```bash
bun install          # Install frontend deps
cargo tauri dev      # Run in dev mode
cargo tauri build    # Production build
```

## Testing

```bash
cargo test                                  # Run all Rust tests
cargo fmt --check                           # Check formatting
cargo clippy -- -D warnings                 # Lint check
```

- All three checks must pass before pushing (CI runs them)
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

## Wiki Feature

- **LLM backend**: codex exec CLI (not direct API). No API keys needed -- uses ChatGPT subscription via codex auth.
- **Classification model**: o4-mini (fast). **Summary model**: gpt-5.4 (quality).
- **Batch processing**: 20 messages per codex call. Worker runs on dedicated thread with tokio runtime.
- **Categories are dynamic**: LLM picks freely, backend deduplicates via known aliases (20 groups) + fuzzy matching. No hardcoded seed categories.
- **Decoupled from sync**: Collection enqueues message IDs into `wiki_classify_queue`. Worker processes independently.
- **Schema migrations are one-shot**: Changing migration code does NOT affect existing databases. To reset wiki data: `sqlite3 tg-korean-search.db "DELETE FROM wiki_categories; DELETE FROM wiki_topics; DELETE FROM wiki_classify_queue;"`

## Gotchas

- `cargo tauri dev` runs from `src-tauri/` directory, not project root. File lookups (like `.env`) must check parent dir.
- Stale `telegram.session` causes grammers panics ("cannot commit", "readonly database"). Fix: delete `~/Library/Application Support/telegram-korean-search/telegram.session`.
- ChatGPT OAuth tokens do NOT work with `api.openai.com/v1/chat/completions` (requires separate API billing). Use `codex exec` subprocess instead.
- Clippy enforces `format!()` not `&format!()` for args accepting `impl AsRef<str>`, and `.is_multiple_of(N)` instead of `% N == 0`.

## Data Location

```
~/Library/Application Support/telegram-korean-search/
  telegram.session         # Telegram session (grammers SQLite)
  tg-korean-search.db      # SQLite DB (includes FTS5 index)
```

## Release Process

- **Auto-release on PR merge**: When a PR is merged to `main`, the `auto-release.yml` workflow automatically bumps the version, commits it, and creates a git tag (which triggers the `release.yml` build).
- **PR labels control version bump**:
  - `major` label -> major bump (e.g., `0.2.0` -> `1.0.0`)
  - `minor` label -> minor bump (e.g., `0.2.0` -> `0.3.0`)
  - No label -> patch bump (e.g., `0.2.0` -> `0.2.1`)
- Version is stored in 3 files: `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` (all updated automatically by the workflow).
- Do not manually create tags or bump versions -- let the workflow handle it.

## Do Not

- Do not add `lindera` or `unicode-segmentation` -- removed in favor of FTS5 trigram
- Do not hold store mutex during network I/O -- causes UI freeze
- Do not use OFFSET pagination -- use cursor-based pagination
- Do not commit `.env`, `telegram.session`, `*.key`, or files in `.gitignore`
- Do not skip `cargo fmt` and `cargo clippy` before pushing
- Do not call OpenAI API directly -- use codex exec CLI for all LLM calls
- Do not hardcode wiki categories -- they are LLM-decided and auto-created
