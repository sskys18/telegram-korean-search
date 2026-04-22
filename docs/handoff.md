# Session Handoff
> Generated: 2026-04-22 (after sqlcipher 4.6.1 drop-in, Korean search
> shipping live on the TelegramSwift fork).

## Task
Build `telegram-seoyu`: macOS Telegram client with Korean substring search and
an LLM-generated wiki. Fork of `overtake/TelegramSwift` + Rust sidecar
(`sidecar/`), bridged via UniFFI (`packages/Seoyu/`).

## Status

### Proven end-to-end
- Fork builds on macOS 26 + Xcode 26.4.1, launches, authenticates,
  syncs Postbox.
- Seoyu bridge opens its sqlite store on app launch.
- Postbox observer fires for every `addMessages`, sidecar rows grow in
  real time. Observed during login: **0 → 14k+ rows in minutes**.
- `SearchController.prepareEntries()` merges Seoyu hits with Telegram's
  native search, de-dupes by `MessageId`, sorts the merged list by
  message timestamp descending, and surfaces extra 한글 hits the server
  alone does not produce.

### Sidecar
- 100 lib + 4 integration tests, clippy clean.
- Schema at **v7**.
- Search planner branches: plain → nospace → jamo. Explicit jamo and
  nospace routes.
- FTS5 trigram runs on sqlcipher 4.6.1 (SQLite 3.46.1) — enabled via
  the vendored amalgamation at `vendor/sqlcipher-4.6.1/`.
- `insert_messages_batch` now rolls back on mid-batch error and
  auto-inserts a stub chat row so the FK check never stalls ingest.

### Swift ↔ sidecar
- `packages/Seoyu` is a local SPM dep of the Telegram target.
- `Telegram-Mac/Seoyu/SeoyuBridge.swift` — singleton + lazy bootstrap
  from `AppDelegate.applicationDidFinishLaunching`.
- `Telegram-Mac/Seoyu/SeoyuIngestObserver.swift` — `StoreOrUpdateMessageAction`
  conformer that mirrors every message into `seoyu.indexMessages`.
- `SeoyuBridge.attach(postbox:)` installs the observer from
  `AccountContext.init`, gated with `#if !SHARE`.
- `SearchController.prepareEntries()` wraps the remote search signal
  with a mapToSignal that asks `SeoyuBridge.search(...)` for extra
  hits, materializes them through `postbox.transaction.getMessage`,
  and merges into the entry list in timestamp-descending order.
  It also filters out chat ids that are not valid `PeerId.toInt64`
  outputs (stale Tauri-era rows would crash `PeerId(Int64)` in Debug).

### Build plumbing
- `scripts/ld-cryptex-shim.sh` — strips the Swift driver's bogus
  Metal-cryptex back-deploy dylib path (Xcode 26 bug).
- `scripts/fix-shallow-frameworks.sh` — converts Firebase /
  GoogleAppMeasurement xcframework `macos-*` slices from shallow to
  versioned bundles (needed because macOS 26 enforces versioned
  layout).
- `scripts/patch-submodules.sh` — bumps submodule `Package.swift`
  deploy targets, installs the Postbox global observer patch, and
  replaces the vendored sqlcipher amalgamation with 4.6.1 (all
  idempotent).
- `scripts/build-seoyu-xcframework.sh` — builds the Rust sidecar and
  bundles it as `packages/Seoyu/SeoyuFFI.xcframework`.

## Resume Here — the wiki panel is next

Everything up to and including Korean search is live. The remaining
product work is the wiki feature. Pipeline is already implemented on
the sidecar side (see `sidecar/src/wiki/`):

- `wiki/llm.rs` wraps `codex exec` subprocess (o4-mini for topic
  classification, gpt-5.4 for summary rendering).
- `wiki/worker.rs` classifies queued messages into categories / topics;
  the Tauri event-bus coupling is gone (`EventEmitter` trait instead).
- `wiki/trending.rs` maintains daily topic stats.
- UniFFI already exposes `seoyu.wikiTrending(limit:offset:category:)`
  and `seoyu.wikiTopicDetail(topicId:)` — the Swift side just has to
  call them.

### Concrete next steps

1. **Worker loop**. The worker is class-only today — schedule it from
   `SeoyuBridge.attach` on a background thread so newly indexed
   messages are classified without user intervention. Drop any
   result / progress onto a Seoyu-side `Signal` (or broadcast log) so
   the Swift side can show "N messages processed" without polling.
2. **Wiki panel UI**. Add a sidebar / tab inside the fork that lists
   trending topics, opens a topic detail view with the bilingual
   article, and lets the user click through to the source messages
   (already linked via `wiki_topic_messages` foreign keys).
   - Suggested Swift path:
     `Telegram-Mac/Seoyu/Wiki/WikiPanelController.swift` +
     a SwiftUI / NSViewController split.
   - `seoyu.wikiTrending` + `seoyu.wikiTopicDetail` are the only
     UniFFI calls needed for the MVP.
3. **Historical backfill**. The Postbox observer only sees messages
   that are added or updated *after* the observer attaches. Older
   messages that live in Postbox but never get touched again stay
   invisible to Seoyu. Write a one-shot crawler that walks every
   peer's history via Postbox and pushes batches through the same
   `seoyu.indexMessages` path the observer uses.
4. **Chat-scoped search**. `SearchController` currently only merges
   for the global (chat-list) search. Per-chat search (inside a single
   chat window) still falls back to native. Wiring is the same
   pattern with `SeoyuBridge.search(..., scope: .chat(peerId.toInt64()))`.

Run-before-code checklist once per fresh working tree:

```
./scripts/patch-submodules.sh
./scripts/fix-shallow-frameworks.sh
./scripts/build-seoyu-xcframework.sh
xcodebuild build -workspace Telegram-Mac.xcworkspace -scheme Telegram \
  -configuration Debug -destination 'generic/platform=macOS' \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO \
  LD=$(pwd)/scripts/ld-cryptex-shim.sh \
  LDPLUSPLUS=$(pwd)/scripts/ld-cryptex-shim.sh
```

## Decisions
- **UniFFI over IPC as primary bridge**: picked in commit `48d215712`.
  IPC stays in tree for debugging.
- **`.macOS(.v12)` everywhere**: `.v13` would require
  `swift-tools-version:5.7`; upstream manifests are on `:5.5`.
- **981 Bazel `BUILD` files deleted from submodule working tree**:
  APFS case-insensitive clash with Xcode's `build/` intermediate dir.
- **Keep IPC server** in parallel with UniFFI.
- **`CODE_SIGNING_ALLOWED=NO`** for every xcodebuild invocation (no
  paid Apple Dev account yet).
- **Work around the Xcode 26 cryptex bug locally** via `ld-cryptex-shim`.
  See `docs/XCODE26-BLOCKER.md`.
- **Timestamp-descending sort** for merged search: Telegram server
  orders by relevance, which interleaved year-old hits with today's
  results. The merged output sorts message entries newest-first.
- **Ship sqlcipher 4.6.1**: upstream vendors 3.33.0, which predates
  FTS5 trigram. The amalgamation under `vendor/sqlcipher-4.6.1/` is
  the drop-in; `patch-submodules.sh` applies it.

## Gotchas
- `submodules/tg_owt/src/api/candidate.h` patch was undone during a
  submodule reset. Webrtc builds clean without the patch thanks to
  `-Wno-error` in `core-xprojects/webrtc/webrtc/build.sh`. If webrtc
  breaks again on `lifetimebound`, the patch history is around commit
  `e904583b6`.
- `packages/ApiCredentials/Sources/ApiCredentials/Config.swift` is
  gitignored and holds real credentials. Do not regenerate.
- `Config.example.swift` was renamed to `Config.swift.example` on
  purpose — two `.swift` files declared the same type.
- `scripts/rebuild` file is left at `no`. The cleanup loop in
  `configure_frameworks.sh` is gated on `yes`; the main build loop
  runs regardless.
- `scripts/fix-shallow-frameworks.sh` must be re-run after every
  `rm -rf DerivedData` or SPM re-resolve.
- `scripts/ld-cryptex-shim.sh` hardcodes `/Applications/Xcode.app`.
- All submodule-working-tree patches (24 `Package.swift` bumps,
  Postbox global observer, sqlcipher amalgamation) are reapplied
  idempotently by `scripts/patch-submodules.sh` after any
  `git submodule update`.
- Pre-observer rows in the Seoyu DB use raw Tauri-era chat ids that
  fail `PeerId.init(Int64)` round-trip. The search merge filters
  them out by bit-pattern check. A clean reset is
  `sqlite3 "$DB" "DELETE FROM messages WHERE chat_id NOT IN (SELECT DISTINCT chat_id FROM messages WHERE timestamp > <observer-install-ts>);"`.

## Context
- **Branch**: `main`.
- **Tag**: `archive/tauri-v0` → pre-fork Tauri companion app state.
- **Tests**: sidecar 100 lib + 4 integration passing, 1 ignored.
  Telegram.app builds, launches, authenticates. Ingest + search merge
  verified against a live account.
- **Unpushed**: all commits on local `main` beyond `bd00180` are
  unpushed to `origin`.
- **Untracked**: `submodules/telegram-ios/third-party/td/TdBinding/SharedHeaders/`
  generated headers (configure_frameworks.sh output inside submodule —
  harmless).
