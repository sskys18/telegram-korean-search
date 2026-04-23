# CLAUDE.md

Guidance for Claude Code (claude.ai/code) when working in this repository.

## Project Overview

**telegram-seoyu** is a local, macOS-only Telegram client built by forking
[overtake/TelegramSwift](https://github.com/overtake/TelegramSwift) and
pairing it with a Rust sidecar that owns a Korean-aware search index and an
LLM-driven wiki.

Status: **dev build live** (v0.4.0-dev, unsigned, macOS 26 arm64). The
TelegramSwift fork, sidecar, and UniFFI bridge are all in tree and ship a
launchable `Telegram.app`. The wiki panel is next.

- **archive/tauri-v0** (git tag `archive/tauri-v0`) — the previous Tauri-based
  companion app. Reference only.
- **main** — TelegramSwift fork + Rust sidecar + `packages/Seoyu` UniFFI
  bridge. Korean search works end-to-end.

## Architecture

```
TelegramSwift fork  ──(in-process UniFFI FFI)──►  telegram-seoyu sidecar
 (AppKit, Swift)                                   (Rust static lib)
  · chat UI, login                                  · SQLite + FTS5 mirror
  · SearchController hook                           · Korean normalizer
  · SeoyuBridge singleton                           · wiki pipeline
                                                    · codex exec subprocess
```

Messages enter Postbox via existing TelegramSwift sync. `SeoyuBridge`
installs a global Postbox observer that mirrors every stored-or-updated
message into the sidecar's SQLite store and keeps the Korean index current.
Search queries from the Swift search bar fan out: native `messages.search`
in parallel with `SeoyuBridge.search(query:)`. Results merge inside
`SearchController.prepareEntries()`.

## Tech Stack

- **Shell**: Swift, AppKit (via TelegramSwift fork, GPLv2)
- **Sidecar**: Rust, standalone crate under `sidecar/` (MIT), linked into the
  app as a static library via `packages/Seoyu` (Swift package, UniFFI-generated
  bindings). No Unix socket, no separate process.
- **Storage (sidecar)**: SQLite with WAL mode
- **Search (sidecar)**: SQLite FTS5 trigram tokenizer + Korean-specific
  auxiliary columns (jamo, nospace)
- **LLM**: `codex exec` CLI subprocess (o4-mini for classification, gpt-5.4
  for summaries) — default. Other backends pluggable later.
- **Session security**: AES-256-GCM, macOS Keychain (retained from the
  previous shell, lives under `sidecar/src/security/`)

## Repository layout

```
Telegram-Mac/          # Swift app target (upstream + Seoyu additions)
  Seoyu/
    SeoyuBridge.swift       # singleton that opens the store + exposes search
    SeoyuIngestObserver.swift
  AppDelegate.swift         # calls SeoyuBridge.bootstrap()
  AccountContext.swift      # calls SeoyuBridge.attach(postbox:)
  SearchController.swift    # augments remote search with Seoyu hits

packages/Seoyu/        # Swift package: UniFFI bindings over the sidecar

sidecar/               # Rust crate (telegram-seoyu-sidecar)
  Cargo.toml
  src/
    lib.rs              # UniFFI surface: Seoyu::new, search, ingest, …
    bin/main.rs         # CLI stub (opens store, logs, exits)
    search/
      engine.rs           # FTS5 multi-column search (content+jamo+nospace)
      highlight.rs
    store/
      schema.rs           # DB schema & migrations (v5 with jamo/nospace)
      message.rs          # messages + FTS5 inserts
      chat.rs, sync_state.rs, app_meta.rs
      wiki_category.rs, wiki_page.rs, wiki_queue.rs,
      wiki_stats.rs, wiki_topic.rs
    security/
      crypto.rs, keychain.rs
    wiki/
      llm.rs              # codex exec subprocess wrapper
      worker.rs           # stub; full impl in archive/tauri-v0
      trending.rs

scripts/
  build-dev.sh        # one-shot unsigned arm64 Debug build
  ld-cryptex-shim.sh  # Xcode 26 Metal-cryptex linker workaround
  fix-shallow-frameworks.sh
docs/
  handoff.md          # current session snapshot
  XCODE26-BLOCKER.md  # documented Xcode 26 build workaround
  SQLCIPHER-TRIGRAM-BLOCKER.md
  legacy/             # Tauri-era design docs, reference only

.github/              # issue + PR templates (workflows rewritten later)
```

## Build & Run

Full app, from a clean checkout:

```bash
./scripts/build-dev.sh --run     # builds + launches unsigned Debug app
./scripts/build-dev.sh --dmg     # builds + packages dist/Telegram-seoyu.dmg
```

Sidecar standalone:

```bash
cd sidecar
cargo build                       # debug
cargo build --release             # produces tg-seoyu-sidecar
cargo test                        # must be green before commit
cargo fmt --check                 # must be clean before commit
cargo clippy -- -D warnings       # must be clean before commit
```

The sidecar binary is a CLI stub used for manual store inspection; the real
consumer of the Rust library is the Swift app via the `packages/Seoyu`
UniFFI bindings.

## Architecture Rules

- **Sidecar compiles clean at every commit.** `cargo fmt --check`,
  `cargo clippy -D warnings`, and `cargo test` all green. Do not break the
  build even for intermediate states.
- **Sidecar stays Tauri-free.** No `tauri::*` imports, no event bus coupling.
  Progress reporting goes through the UniFFI surface.
- **No grammers, no TDLib in the sidecar.** TelegramSwift owns MTProto. The
  sidecar only receives already-parsed messages through the UniFFI bridge.
- **FTS5 auto-indexing**: messages indexed into `messages_fts` on insert via
  `insert_messages_batch()`. No separate indexing step.
- **Cursor pagination**: use `(timestamp, chat_id, message_id)` cursors.
  Never `OFFSET`.
- **Never hold the store mutex across `.await`.** Short critical sections
  only.
- **One source of truth per piece of state.** If you are tempted to duplicate
  state to fix a rendering or reentrancy issue, you are solving the wrong
  problem.

## Korean search (shipped)

`messages_fts` has three columns: `content`, `jamo`, `nospace`. Ingest
computes `jamo` (decomposed Hangul via in-crate codepoint math, no external
lib) and `nospace` (whitespace-stripped) on every insert/update. Search
issues `MATCH` against each column in a single query, UNIONs the row ids,
and ranks by `bm25`. Migration v5 backfills existing rows.

## Wiki plan

The wiki pipeline (categories, topics, pages, trending, queue) is preserved
in the sidecar and is functionally intact. The worker is currently stubbed
because it was coupled to the Tauri event bus; once the Swift-side wiki
panel lands the worker is rewritten to emit progress through the UniFFI
bridge instead of Tauri events.

## Gotchas

- **Xcode 26 Metal-cryptex linker bug.** Every Swift-linking target fails
  at Ld with a missing `libswiftAppKit.dylib` under a Metal cryptex mount.
  Fixed by `scripts/ld-cryptex-shim.sh`, wired in via `LD=` / `LDPLUSPLUS=`
  in `scripts/build-dev.sh`. See `docs/XCODE26-BLOCKER.md` for the full
  recipe. If you build via Xcode GUI, the shim is not applied — Xcode's
  internal driver is mostly unaffected, but run the CLI build if you hit
  this error from GUI after a system update.
- **sqlcipher lacks the FTS5 trigram tokenizer** by default. We build
  sqlcipher with `SQLITE_ENABLE_FTS5` + the trigram source patched in.
  See `docs/SQLCIPHER-TRIGRAM-BLOCKER.md`. If ingest logs
  `no such tokenizer: trigram`, the patched sqlcipher did not land.
- Stale `telegram.session` files previously caused grammers panics. Under
  TelegramSwift this is no longer relevant — Postbox manages its own session.
- `codex exec` subprocess is the only LLM path. Users without a ChatGPT
  subscription will see the wiki tab disabled; pluggable backends come later.
- Schema migrations are one-shot. Resetting wiki data locally:
  `sqlite3 tg-korean-search.db "DELETE FROM wiki_categories; DELETE FROM wiki_topics; DELETE FROM wiki_classify_queue;"`
- Clippy enforces `format!()` not `&format!()`, `.is_multiple_of(N)` over
  `% N == 0`, and the 1.92-era `std::slice::from_ref` fixit.
- Dev builds are unsigned (`CODE_SIGNING_ALLOWED=NO`); Keychain-backed
  session encryption in `sidecar/src/security/keychain.rs` falls back to
  plaintext without a real signing identity. Sign via Xcode GUI with a
  Developer ID for full feature validation.

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
