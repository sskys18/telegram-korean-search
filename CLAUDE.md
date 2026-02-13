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
  commands.rs       # Tauri command handlers (auth, search, collection)
  lib.rs            # App state, initialization, graceful shutdown
  main.rs           # Entry point
  error.rs          # Error types
  logging.rs        # flexi_logger setup
  collector/        # Telegram message fetching (grammers)
  search/           # FTS5 search engine with highlighting
  security/         # AES-256-GCM session encryption, Keychain
  store/            # SQLite data layer (schema, CRUD, FTS5 indexing)

src/
  App.tsx           # Root component, auth/search routing
  App.css           # Global styles
  hooks/            # useAuth, useSearch hooks
  pages/            # LoginPage, SearchPage
  components/       # ChannelSelector, SearchResults, etc.
  api/              # Tauri invoke wrappers
  types/            # TypeScript type definitions
  utils/            # Helpers
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
- Schema migrations are versioned via `app_meta.schema_version` (currently v2)
- Collection functions are split: `fetch_chats`/`fetch_messages` (network-only) + brief lock for DB writes
- Stale sessions detected by `AUTH_KEY_UNREGISTERED` error, auto-recovered by deleting session and reconnecting

## Data Location

```
~/Library/Application Support/telegram-korean-search/
  session.bin              # Encrypted Telegram session
  tg-korean-search.db      # SQLite DB (includes FTS5 index)
```

## Do Not

- Do not add `lindera` or `unicode-segmentation` -- removed in favor of FTS5 trigram
- Do not hold store mutex during network I/O -- causes UI freeze
- Do not use OFFSET pagination -- use cursor-based pagination
- Do not commit `.env`, `session.bin`, `*.key`, or files in `.gitignore`
- Do not skip `cargo fmt` and `cargo clippy` before pushing
