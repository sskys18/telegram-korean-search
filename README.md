# telegram-korean-search

> Local-first Telegram Korean/English search system for macOS

A desktop app that provides **Korean partial search** and **whitespace-agnostic search** across your Telegram group/channel messages. Open a mini search panel with a global hotkey, find messages instantly, and jump directly to them in Telegram Desktop.

## Features

- **Korean-optimized search** -- FTS5 trigram tokenizer handles Korean substring matching without language-specific tokenization
- **Whitespace-agnostic** -- Finds "삼성전자" whether the original text has "삼성 전자" or "삼성전자"
- **English search** -- Same trigram-based substring matching works for English text
- **Instant results** -- FTS5 index with cursor-based pagination, < 300ms response
- **Global hotkey** -- `Cmd+Shift+F` opens/closes the search panel from anywhere
- **Deep linking** -- Click a result to jump directly to the message in Telegram Desktop
- **Local-only** -- All data stays on your machine. No cloud, no external servers
- **Non-intrusive** -- Does not hook into or modify Telegram Desktop in any way
- **Non-blocking sync** -- Search while messages sync in the background

## Screenshots

_Coming soon_

## Tech Stack

- **Backend**: Rust (Tauri v2)
- **Frontend**: React + TypeScript + Vite
- **Telegram**: grammers (MTProto client)
- **Search**: SQLite FTS5 with trigram tokenizer (LIKE fallback for short queries)
- **Storage**: SQLite with WAL mode
- **Security**: AES-256-GCM encrypted sessions, macOS Keychain

## Getting Started

### Prerequisites

- macOS 12+
- [Rust](https://rustup.rs/) (1.75+)
- [Bun](https://bun.sh/) (1.0+)
- Telegram account
- Telegram API credentials from [my.telegram.org](https://my.telegram.org)

### Build from Source

```bash
# Clone the repo
git clone https://github.com/sskys18/telegram-korean-search.git
cd telegram-korean-search

# Install frontend dependencies
bun install

# Run in development mode
cargo tauri dev

# Build for production
cargo tauri build
```

### Install

Download the latest release from [GitHub Releases](https://github.com/sskys18/telegram-korean-search/releases).

## How It Works

1. **Login** with your Telegram API credentials and phone number (one-time setup)
2. **Collection** starts automatically in the background -- fetches messages from your groups/channels
3. **Search** anytime with `Cmd+Shift+F` -- type your query, see results instantly (even while syncing)
4. **Click** a result to jump to the message in Telegram Desktop

## Architecture

See [docs/architecture.md](docs/architecture.md) for the full design document.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

[MIT](LICENSE)
