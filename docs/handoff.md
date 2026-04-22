# Session Handoff
> Generated: 2026-04-22 (updated after Xcode 26 unblock)

## Task
Build `telegram-seoyu`: macOS Telegram client with Korean substring search and an LLM-generated wiki. Fork of `overtake/TelegramSwift` + Rust sidecar (`sidecar/`), bridged via UniFFI (`packages/Seoyu/`).

## Status

### Completed
- **Rust sidecar** at `sidecar/` — 102 lib + 4 integration tests green, clippy clean
  - Korean search index (`sidecar/src/search/hangul.rs`, `engine.rs`) — jamo / 초성 / nospace, schema v6
  - Wiki classifier worker (`sidecar/src/wiki/worker.rs`) decoupled from Tauri via `EventEmitter` trait
  - IPC over Unix socket (`sidecar/src/ipc/`) — ping, index, search, wiki methods
  - UniFFI surface (`sidecar/src/uniffi_api.rs`) — `Seoyu` object + Records
- **Swift Package** at `packages/Seoyu/` — `Package.swift` two-phase (works pre- and post-xcframework); `swift build` passes
- **Build scripts** — `scripts/build-seoyu-xcframework.sh` cross-builds arm64+x86_64, lipos, generates bindings, assembles xcframework
- **TelegramSwift fork merged at root** — bundle ids renamed to `com.seoyu.telegram-seoyu`, `ffmpeg 7.1.1` pin, webrtc `-Wno-error`, deployment target → 13.0, Package.swift platforms → `.v12`, AppCenter/Sparkle URLs stripped
- **Xcode 26 cryptex blocker resolved** — Telegram.app builds end-to-end and launches on macOS 26 + Xcode 26.4.1. See `docs/XCODE26-BLOCKER.md` for the patch.
  - `scripts/ld-cryptex-shim.sh` strips Swift driver's bogus Metal-cryptex back-deploy arg
  - `scripts/fix-shallow-frameworks.sh` converts Firebase/GoogleAppMeasurement xcframework macos slices to versioned layout
  - 24 `submodules/telegram-ios/submodules/*/Package.swift` manifests bumped from `.v10_13` to `.v12`
  - `core-xprojects/OpenH264/build/arm64/libopenh264.a` copied to `…/build/output/lib/` as the xcodeproj expects
- All 10 C/C++ frameworks compiled (`core-xprojects/{OpenH264,openssl,libopus,libvpx,Mozjpeg,libwebp,dav1d,ffmpeg,webrtc,tde2e}/build/`)

### Swift ↔ sidecar integration (live)
- `packages/Seoyu` is a local SPM dep of the Telegram target; UniFFI symbols link into Telegram.app (commit `e1dbb4469`).
- `Telegram-Mac/Seoyu/SeoyuBridge.swift` — singleton, opens the sqlite store under Application Support on app launch via `AppDelegate` (commit `6276657c2`).
- `SearchController.prepareEntries()` wraps `remoteSearch` with a mapToSignal that asks `SeoyuBridge.search(...)`, materializes `Message`s via `postbox.transaction.getMessage`, de-dupes by `MessageId`, and appends as extra `ChatListSearchEntry.message` rows (commit `02cee2a53`).
- `SeoyuIngestObserver` is attached via `installGlobalStoreOrUpdateMessageAction` from `AccountContext.init`, so every stored or updated Postbox message is mirrored into Seoyu's FTS5 index (commit `f23e5aaad`). The DB file grows in real time on app launch.

### Not yet wired
- **Wiki panel UI** — no Swift code yet.
- **Chat-scoped search** — the merge is global; per-chat search still falls back to native only.
- **Historical backfill** — the observer only sees messages that flow through `addMessages` after the observer is installed. Older messages are only indexed if they pass through Postbox again (edit, repair sync, etc.). A one-shot crawler over existing peers would be valuable.

## Resume Here

1. `./scripts/patch-submodules.sh` once per fresh `git submodule update` (bumps deploy targets + reinstalls the Postbox global-observer patch).
2. `./scripts/fix-shallow-frameworks.sh` once per fresh DerivedData or SPM resolve (Firebase/Google xcframework macos slices are shallow on disk).
3. `./scripts/build-seoyu-xcframework.sh` to (re)build the Rust side.
4. `xcodebuild build -workspace Telegram-Mac.xcworkspace -scheme Telegram -configuration Debug -destination 'generic/platform=macOS' ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO LD=$(pwd)/scripts/ld-cryptex-shim.sh LDPLUSPLUS=$(pwd)/scripts/ld-cryptex-shim.sh`.

Next substantive work is the wiki panel and a historical backfill over existing peers so Seoyu sees messages that predate the observer install.

## Decisions
- **UniFFI over IPC as primary bridge**: picked in commit `48d215712`. IPC stays in tree for debugging but Swift shell calls UniFFI direct.
- **`.macOS(.v12)` everywhere**: `.v13` requires `swift-tools-version:5.7`, upstream Package.swift files are on `:5.5`. Don't bump tools-version without a reason.
- **981 Bazel `BUILD` files deleted from submodule working tree**: APFS case-insensitive clash with Xcode's `build/` intermediate dir. Uncommitted (submodule content). Expect to redelete after `git submodule update`.
- **Keep IPC server** even though UniFFI path is primary — parallel tooling, debugging.
- **`CODE_SIGNING_ALLOWED=NO`** for every xcodebuild invocation (no paid Apple Dev account).
- **Work around the Xcode 26 cryptex bug locally** instead of waiting on Apple. See `docs/XCODE26-BLOCKER.md`.

## Gotchas
- `submodules/tg_owt/src/api/candidate.h` patch from Chunk B was undone during the final submodule reset. Webrtc now builds clean without the patch because we pass `-Wno-error` via cmake flags in `core-xprojects/webrtc/webrtc/build.sh`. If webrtc fails again on `lifetimebound`, the patch is in git history at commit `e904583b6` area.
- `packages/ApiCredentials/Sources/ApiCredentials/Config.swift` is gitignored and holds real `.env` credentials. Do not regenerate from template unless the user rotated the pair.
- `Config.example.swift` was renamed to `Config.swift.example` on purpose — two `.swift` files declared the same type.
- `scripts/rebuild` file is left at `no`. The cleanup loop in `configure_frameworks.sh` is gated on `yes`; the main build loop runs regardless.
- `scripts/fix-shallow-frameworks.sh` must be re-run after every `rm -rf DerivedData` or SPM re-resolve. The fixup targets the materialized copies under `DerivedData/…/SourcePackages/artifacts/`.
- The linker shim (`scripts/ld-cryptex-shim.sh`) assumes Xcode lives at `/Applications/Xcode.app`. Hardcoded path inside the script; update if Xcode moves.
- The 24 bumped `submodules/telegram-ios/submodules/*/Package.swift` files are uncommitted submodule edits — `git submodule update` will revert them. Re-apply via `grep -rl "\.v10_1[0-4]" submodules/telegram-ios/submodules/*/Package.swift | xargs sed -i '' 's/\.v10_1[0-4]/.v12/g'`.

## Context
- **Branch**: `main` (swift-fork-bootstrap fast-forward-merged)
- **Tag**: `archive/tauri-v0` → pre-fork Tauri companion app state
- **Tests**: sidecar 102 lib + 4 integration passing, 1 ignored; Telegram.app builds and launches (smoke only — no integration test yet)
- **Unpushed**: all commits on local `main` beyond `bd00180` are unpushed to `origin`
- **Untracked**: `submodules/telegram-ios/third-party/td/TdBinding/SharedHeaders/` generated headers (configure_frameworks.sh output inside submodule — harmless)
