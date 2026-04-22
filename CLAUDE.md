# CLAUDE.md

Guidance for Claude Code (claude.ai/code) when working in this repository.

## Project Overview

**telegram-seoyu** is a local, macOS-only Telegram client built by forking
[overtake/TelegramSwift](https://github.com/overtake/TelegramSwift) and
pairing it with a Rust sidecar that owns a Korean-aware search index and an
LLM-driven wiki.

The project is in a transition period:

- **archive/tauri-v0** (git tag `archive/tauri-v0`) — the previous Tauri-based
  companion app. Reference only.
- **current branch** — the Rust sidecar has been extracted from the Tauri
  shell, and the Tauri shell has been deleted. The TelegramSwift fork is not
  yet in tree.

## Target Architecture

```
TelegramSwift fork  ──(Unix socket JSON-RPC)──►  telegram-seoyu sidecar
 (AppKit, Swift)                                   (Rust)
  · chat UI, login                                  · SQLite mirror
  · search bar hook                                 · FTS5 + Korean index
  · wiki panel                                      · wiki pipeline
                                                    · codex exec subprocess
```

Messages enter Postbox via existing TelegramSwift sync. The Swift shell
pushes each new message to the sidecar, which mirrors it into its own SQLite
store and keeps the Korean index current. Search queries from the Swift
search bar fan out: native `messages.search` in parallel with the sidecar.
Results merge inside `SearchController.prepareEntries()`.

## Tech Stack

- **Shell**: Swift, AppKit (via TelegramSwift fork, GPLv2)
- **Sidecar**: Rust, standalone crate under `sidecar/` (MIT)
- **IPC**: Unix socket, JSON-RPC (not yet implemented)
- **Storage (sidecar)**: SQLite with WAL mode
- **Search (sidecar)**: SQLite FTS5 trigram tokenizer + Korean-specific
  auxiliary columns (jamo, nospace)
- **LLM**: `codex exec` CLI subprocess (o4-mini for classification, gpt-5.4
  for summaries) — default. Other backends pluggable later.
- **Session security**: AES-256-GCM, macOS Keychain (retained from the
  previous shell, lives under `sidecar/src/security/`)

## Repository layout

```
sidecar/              # Rust crate (telegram-seoyu-sidecar)
  Cargo.toml
  src/
    lib.rs
    bin/main.rs        # binary stub, gains IPC server later
    error.rs
    logging.rs
    search/
      engine.rs         # FTS5 search engine (Korean hooks pending)
      highlight.rs
    store/
      schema.rs         # DB schema & migrations (v4)
      message.rs        # messages + FTS5 inserts
      chat.rs
      sync_state.rs
      app_meta.rs
      wiki_category.rs, wiki_page.rs, wiki_queue.rs,
      wiki_stats.rs, wiki_topic.rs
    security/
      crypto.rs, keychain.rs
    wiki/
      llm.rs             # codex exec subprocess wrapper
      worker.rs          # stub; full impl in archive/tauri-v0
      trending.rs

telegram-swift/       # TelegramSwift subtree (pending Phase 5)

docs/
  legacy/             # Tauri-era design docs, kept for reference

.github/              # issue + PR templates (workflows rewritten later)
```

## Build & Run

Sidecar only, for now:

```bash
cd sidecar
cargo build                # debug
cargo build --release      # produces tg-seoyu-sidecar
cargo test                 # 85 tests, 1 ignored
cargo fmt --check          # must be clean before commit
cargo clippy -- -D warnings # must be clean before commit
```

The binary does nothing useful yet — it opens the store, logs a line, and
exits. The IPC server is the next piece to land.

## Architecture Rules

- **Sidecar compiles clean at every commit.** `cargo fmt --check`,
  `cargo clippy -D warnings`, and `cargo test` all green. Do not break the
  build even for intermediate states.
- **Sidecar stays Tauri-free.** No `tauri::*` imports, no event bus coupling.
  Progress reporting goes through IPC.
- **No grammers, no TDLib in the sidecar.** TelegramSwift owns MTProto. The
  sidecar only receives already-parsed messages through IPC.
- **FTS5 auto-indexing**: messages indexed into `messages_fts` on insert via
  `insert_messages_batch()`. No separate indexing step.
- **Cursor pagination**: use `(timestamp, chat_id, message_id)` cursors.
  Never `OFFSET`.
- **Never hold the store mutex across `.await`.** Short critical sections
  only.
- **One source of truth per piece of state.** If you are tempted to duplicate
  state to fix a rendering or reentrancy issue, you are solving the wrong
  problem.

## Korean search plan

The existing FTS5 trigram tokenizer handles English substring and basic
Korean substring, but not jamo decomposition or whitespace-insensitive
matching. The extension, landing in a dedicated phase, adds:

1. `messages_fts` auxiliary columns: `jamo`, `nospace`.
2. Rust normalizer (no external lib; Hangul codepoint math).
3. Multi-query search: issue `MATCH` against each column, UNION, rank.
4. Backfill existing rows in a schema migration (v5).

## Wiki plan

The wiki pipeline (categories, topics, pages, trending, queue) is preserved
in the sidecar and is functionally intact. The worker is currently stubbed
because it was coupled to the Tauri event bus; once the IPC contract lands
the worker is rewritten to emit progress over IPC instead of Tauri events.

## Gotchas

- Stale `telegram.session` files previously caused grammers panics. Under
  TelegramSwift this is no longer relevant — Postbox manages its own session.
- `codex exec` subprocess is the only LLM path. Users without a ChatGPT
  subscription will see the wiki tab disabled; pluggable backends come later.
- Schema migrations are one-shot. Resetting wiki data locally:
  `sqlite3 tg-korean-search.db "DELETE FROM wiki_categories; DELETE FROM wiki_topics; DELETE FROM wiki_classify_queue;"`
- Clippy enforces `format!()` not `&format!()`, `.is_multiple_of(N)` over
  `% N == 0`, and the 1.92-era `std::slice::from_ref` fixit.

## Data Location

```
~/Library/Application Support/telegram-korean-search/
  tg-korean-search.db       # SQLite (messages, FTS5, wiki tables)
```

TelegramSwift's Postbox lives under its own support directory; the sidecar
does not touch it.

## Do Not

- Do not re-introduce `tauri`, `tauri-plugin-*`, or `grammers-*` dependencies.
- Do not add `lindera` or `unicode-segmentation`. Korean normalization stays
  in-crate, codepoint-based, dependency-free.
- Do not hold the store mutex during I/O.
- Do not use `OFFSET` pagination.
- Do not commit `.env`, `*.session`, `*.key`, or anything in `.gitignore`.
- Do not skip `cargo fmt` or `cargo clippy -D warnings` before pushing.
- Do not call OpenAI's API directly. Use `codex exec`.
- Do not hardcode wiki categories. They are LLM-decided and auto-created via
  alias dedup in `wiki_category.rs`.
- Do not restore `src/` or `src-tauri/`. Those are gone on purpose. Use the
  `archive/tauri-v0` tag for historical reference only.
