# telegram-seoyu

> macOS Telegram client with Korean substring search and a local LLM-powered wiki

**Status: pre-alpha, under active restructuring.** The project is a fork of
[overtake/TelegramSwift](https://github.com/overtake/TelegramSwift) paired
with a local Rust sidecar for Korean search and wiki generation. Upstream
Swift source has been merged in; the search-bar hook and wiki UI wiring are
next.

## Why

Telegram's search ignores Korean substrings. `삼성` does not match `삼성전자`.
Inserted or missing whitespace (`삼성 전자` vs `삼성전자`) misses further.
Jamo (`ㅅㅏㅁ`) searches do not work at all. This project replaces the search
bar behavior with a Korean-aware local index and adds a side-panel wiki that
summarizes chatter across your channels.

## Features

- Korean substring search — `삼성` finds `삼성전자`
- Whitespace-insensitive matching — `삼성 전자` finds `삼성전자`
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
│   │   / nospace)                         │
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
Telegram-Mac/          TelegramSwift app target (upstream, GPLv2)
Telegram-Mac.xcworkspace
packages/, submodules/ TelegramSwift modules and their own submodules
core-xprojects/, ...   upstream build infrastructure

sidecar/               Rust crate, MIT (search, store, wiki, security)
docs/legacy/           Tauri-era design docs (reference only)
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

The Swift app target, Xcode workspace, and submodules live at the repository
root alongside `sidecar/`. Build instructions follow upstream TelegramSwift's
[INSTALL.md](INSTALL.md) for the Swift side, plus `cd sidecar && cargo build`
for the Rust side.

## Attribution

This project is a fork of
[overtake/TelegramSwift](https://github.com/overtake/TelegramSwift). All Swift
source files, the Xcode workspace, and `submodules/` are upstream's work,
licensed under GPLv2. Our additions (`sidecar/`, `docs/legacy/`,
search/wiki integration to come) build on top of it.

Per upstream's fork requirements:

1. This fork uses its own Telegram API ID (configured at first run).
2. This fork is **not** called "Telegram". The app and repository are
   `telegram-seoyu`.
3. This fork does **not** use Telegram's standard logo. A distinct icon is
   in progress.
4. We aim to follow Telegram's
   [MTProto security guidelines](https://core.telegram.org/mtproto/security_guidelines).
5. Source is public per GPLv2.

## License

- Swift shell + all upstream code: **GPLv2** (see [LICENSE](LICENSE))
- `sidecar/` Rust crate: **MIT** (see `sidecar/Cargo.toml`)

## Scope

Personal, local-only, macOS-only. No code signing, no notarization, no DMG
distribution, no Mac App Store. Clone and build from source.
