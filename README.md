# telegram-seoyu

> macOS Telegram client with Korean substring search and a local LLM-powered wiki

**Status: pre-alpha, under active restructuring.** The project is moving from a
standalone Tauri companion app to a fork of
[overtake/TelegramSwift](https://github.com/overtake/TelegramSwift) paired with
a local Rust sidecar. The fork is not yet in tree (pending Phase 5); this
README documents the target architecture.

## Why

Telegram's search ignores Korean substrings. `삼성` does not match `삼성전자`.
Inserted or missing whitespace (`삼성 전자` vs `삼성전자`) misses further.
초성 (`ㅅㅅㅈㅈ`) and jamo (`ㅅㅏㅁ`) searches do not work at all. This project
replaces the search bar behavior with a Korean-aware local index and adds a
side-panel wiki that summarizes chatter across your channels.

## Features

- Korean substring search — `삼성` finds `삼성전자`
- Whitespace-insensitive matching — `삼성 전자` finds `삼성전자`
- 초성 search — `ㅅㅅㅈㅈ` finds `삼성전자`
- 자모 decomposition search — `ㅅㅏㅁ` finds `삼성`
- English substring search
- Local FTS5 index over every message stored in Postbox
- LLM-generated wiki (trending topics, bilingual summaries, cross-channel
  entity linking) via the `codex` CLI
- Local-only: nothing leaves the machine except the Telegram MTProto traffic
  TelegramSwift already makes and the `codex` calls you enable

## Architecture

```
┌──────────────────────────────────────────┐
│  TelegramSwift fork (AppKit, GPLv2)      │
│   ├── original chat UI, login, sync      │
│   └── SearchController hook + Wiki panel │
└─────────────────┬────────────────────────┘
                  │ Unix socket JSON-RPC
┌─────────────────▼────────────────────────┐
│  telegram-seoyu sidecar (Rust, MIT)      │
│   ├── SQLite mirror (messages + FTS5)    │
│   ├── Korean normalizer (trigram / jamo  │
│   │   / chosung / nospace)               │
│   ├── search engine                      │
│   └── wiki pipeline (codex exec)         │
└──────────────────────────────────────────┘
```

TelegramSwift pushes every new message to the sidecar as it hits Postbox.
The sidecar owns the Korean index and the wiki. Queries from the search bar
fan out in parallel: the existing server-side `messages.search` runs as usual,
and the sidecar returns local substring matches. Results merge inside
`SearchController.prepareEntries()` before rendering.

Full design doc lives at `docs/architecture.md` (to be written post-Phase 5).
Legacy docs for the old Tauri companion app live under `docs/legacy/`.

## Repository layout

```
sidecar/            Rust crate (search, store, wiki). Published under MIT.
telegram-swift/     TelegramSwift subtree (to be added). Inherits GPLv2.
docs/legacy/        Design docs from the old Tauri app, kept for reference.
```

## Build

Not buildable as a full app yet. The sidecar is buildable standalone:

```bash
cd sidecar
cargo build --release          # produces tg-seoyu-sidecar binary
cargo test                      # 85 tests, 1 ignored
cargo clippy -- -D warnings     # clean
cargo fmt --check               # clean
```

The Swift shell will be added at `telegram-swift/` in Phase 5. Build
instructions will follow once it lands.

## License

- Root / Swift shell: GPLv2 (inherited from TelegramSwift)
- `sidecar/`: MIT (see `sidecar/LICENSE` once added)

## Scope

Personal, local-only, macOS-only. No code signing, no notarization, no DMG
distribution, no Mac App Store. Clone and build from source.
