# Session Handoff
> Generated: 2026-04-23 17:05

## Task
Rebuild TelegramSwift fork with latest Seoyu source, confirm Korean search
(Seoyu sidecar) is live in the running app binary.

## Status

### Completed
- Stale Apr 1 dev binary killed (was PID 25602).
- Diagnosed recurring Xcode 26 Metal-cryptex linker bug. Same bug
  previously resolved and documented in
  `docs/XCODE26-BLOCKER.md` — workaround is a linker shim at
  `scripts/ld-cryptex-shim.sh`, plus `scripts/fix-shallow-frameworks.sh`
  to re-version Firebase/GoogleAppMeasurement macos-slices each build.
- Rebuilt cleanly using documented recipe:
  ```
  ./scripts/fix-shallow-frameworks.sh
  xcodebuild build -workspace Telegram-Mac.xcworkspace -scheme Telegram \
    -configuration Debug -destination 'generic/platform=macOS' \
    ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO \
    LD=$PWD/scripts/ld-cryptex-shim.sh \
    LDPLUSPLUS=$PWD/scripts/ld-cryptex-shim.sh \
    -derivedDataPath ~/Library/Developer/Xcode/DerivedData/Telegram-Mac-basjkgxsmvqzctcrxcuexrxbttgq
  ```
- Binary fresh: `stat -f "%Sm"` on
  `~/Library/Developer/Xcode/DerivedData/Telegram-Mac-basjkgxsmvqzctcrxcuexrxbttgq/Build/Products/Debug/Telegram.app/Contents/MacOS/Telegram`
  → `Apr 23 17:00:46 2026`.
- Seoyu symbols linked: `nm Telegram.debug.dylib | grep -ic seoyu` → **2658**.
- App launched (PID 83825), sidecar store active:
  `~/Library/Application Support/telegram-korean-search/tg-korean-search.db`
  mtime Apr 23 17:02, WAL growing → ingest live.

### Verified indirectly
- No `[seoyu] opened store at ...` os_log lines captured. NSLog in Xcode
  debug builds routes to stderr, not `os_log` subsystem. DB mtime is the
  authoritative proof the Rust sidecar opened. OS log shows
  `com.seoyu.telegram-seoyu` bundle id on AVFoundation/UserNotifications
  events, confirming the running process is the fresh Seoyu build.

## Resume Here
1. App is already running (PID 83825). Focus the window.
2. Search "삼성전" in the search bar. Expect Seoyu-augmented hits in
   the global results list, merged in
   `Telegram-Mac/SearchController.swift:1254`.
3. If no Seoyu hits appear:
   - Verify ingest completed: FTS row count via
     `sqlite3 ~/Library/Application\ Support/telegram-korean-search/tg-korean-search.db "SELECT COUNT(*) FROM messages_fts;"`
   - Stream app stderr (not os_log):
     `log stream --process Telegram --debug --info` and watch for
     `[seoyu]` prefix, OR relaunch under Xcode debugger.
   - Bridge search path:
     `Telegram-Mac/Seoyu/SeoyuBridge.swift:57` — `search(query:limit:)`
     catches errors silently and returns `[]`. Add a breakpoint there
     if the augment merge looks empty.

## Decisions
- **CLI build, not GUI**: user had no signing cert in login keychain
  (`security find-identity -p codesigning -v` → 0 identities). CLI path
  with `CODE_SIGNING_ALLOWED=NO` + the documented ld-cryptex shim
  produced a working unsigned arm64 build that launches and reaches
  Seoyu bootstrap.
- **Single-arch**: `ARCHS=arm64 ONLY_ACTIVE_ARCH=YES` — halves link
  time, matches the XCODE26-BLOCKER recipe, no x86_64 needed on this
  box.

## Gotchas
- `xcodebuild build -workspace ... -scheme Telegram` without a
  `-destination` flag fails silently with "Supported platforms ...
  is empty". Always pass `-destination 'generic/platform=macOS'` (or
  `platform=macOS`).
- Two concurrent `xcodebuild` against the same DerivedData lock each
  other via `build.db is locked`. Previous-session zombie xcodebuild
  will block the current build. `pgrep -lf xcodebuild` before starting.
- Metal-cryptex linker error signature is unchanged from the previous
  incident. If you see it again, the shim was not applied — check that
  `LD=` and `LDPLUSPLUS=` both point at
  `scripts/ld-cryptex-shim.sh` and that the script is executable.
- Setting `TOOLCHAINS=com.apple.dt.toolchain.XcodeDefault` to "fix"
  the cryptex path breaks the Metal shader compile step (it needs the
  Metal toolchain). The shim is the correct fix — do not revert to the
  TOOLCHAINS workaround.
- `CODE_SIGNING_ALLOWED=NO` unsigned builds reach Seoyu bootstrap
  (NSLog + SQLite), but Keychain-backed features (session secrets in
  `sidecar/src/security/keychain.rs`) will fail silently if you exercise
  them. Sign properly in Xcode GUI for full feature validation.

## Context
- **Branch**: main, clean except pre-existing
  `M docs/handoff.md`, `M .gitignore`, untracked
  `docs/plans/`, `docs/specs/`, `default.profraw`.
- **Build artifacts**:
  `~/Library/Developer/Xcode/DerivedData/Telegram-Mac-basjkgxsmvqzctcrxcuexrxbttgq/Build/Products/Debug/Telegram.app`
- **Running proc**: dev build PID 83825.
  Official `/Applications/Telegram.app` also running — do not confuse.
- **Sidecar state**: `tg-korean-search.db` 194 MB, WAL 112 MB, live.
- **Prior handoff**: superseded. Git `docs/handoff.md@HEAD~1`.
