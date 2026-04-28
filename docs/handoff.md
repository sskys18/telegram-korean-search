# Session Handoff
> Generated: 2026-04-28 16:36

## Task
Plan 0 (Swift delete observer) shipped to repo + wiki UI revamp shipped.
Manual UI verify of delete observer still NOT performed.

## Status
### Completed this session
- Committed Plan 0 (was pending from prior session):
  - Submodule `submodules/telegram-ios` @ `60b026c90b`:
    `feat(postbox): expose global delete-messages observer`
  - Main `cd622ca39`:
    `feat(seoyu): wire Postbox deletions into sidecar delete_messages`
- Wiki tab UI revamp committed @ `0f28619c1`
  (`feat(wiki): inline search field, settings menu, auto-load 24h trending`):
  - Toolbar: NSSearchField (left) + gear NSMenu (right). Removed old
    `langButton`, modal search sheet, queue/processed status label.
  - Settings menu: "24h Trending" + Language submenu (KO/EN).
  - Search field self-registers as TGUIKit window responder at `.modal`
    priority but only when wiki view tree owns first responder, so chat
    input is unaffected.
  - List: opens to 24h Trending by default (no auto-trigger via menu
    needed). Empty query shows "Type to search wiki" placeholder.
  - NSStackView distribution=.fill + scroll width pinned to root so rows
    fill height instead of collapsing.
  - Digest card: dropped topic/msg counters; hides entirely when no
    hot topics.
- All 4 commits ahead of `origin/main`. **Push pending user instruction.**

### Not committed (intentionally left dirty)
- `Telegram-Mac/Info.plist`, `TelegramShare/Info.plist` — pre-existing
  unknown-origin mods. Investigate before staging.
- `submodules/tgcalls` pointer (`m` in status) — unrelated to this work.
- Submodule `Package.swift` modifications across many submodules — local
  Xcode artifact churn.
- `.backup/`, `.claude/` — local artifacts.

### Blocking
- **Manual UI verify of delete observer still not done.** Nothing in
  Plan 0's Task 0.8 has been confirmed by hand. Code/build green; needs
  user to:
  1. Send `SEOYU-DELETE-TEST-<token>` in any cloud chat.
  2. Search Seoyu, confirm hit.
  3. Long-press → "Delete for me".
  4. Re-search, expect zero hits.
  5. SQL check:
     ```bash
     sqlite3 ~/Library/Application\ Support/telegram-korean-search/tg-korean-search.db \
       "SELECT COUNT(*) FROM messages WHERE text_plain LIKE '%SEOYU-DELETE-TEST-<token>%';"
     ```
     Expect `0`.

## Resume Here
1. **Push to origin** (4 commits ahead):
   - Submodule first: `git -C submodules/telegram-ios push origin HEAD:main`
     (or wherever the upstream branch is — verify before forcing).
   - Then main: `git push origin main`.
   Confirm with the user before push since Plan 0 wiring is unverified.
2. Manual delete-observer verify per checklist above. If verify fails:
   check Console for `[seoyu] delete failed`; inspect Postbox.swift:2492
   fire site (line numbers shifted post-commit; grep for `.Remove`).
3. Next: Spike 1 — `cross build --target aarch64-unknown-linux-gnu --release`
   on `sidecar/`; cfg-gate `security-framework` to `target_os = "macos"`.
4. Then: v9 reindex/wiki + cloud worker spec; rewrite wiki worker on
   UniFFI events (replace remaining Tauri-coupled stubs).

## Decisions (locked)
- Default backend: cloud. Hybrid scope. No paid Oracle. SQLite FTS5 stays.
- Order: Plan 0 (done in repo, verify pending) → Spike 1 → v9 migration.
- Wiki UX (this session): blank-on-open replaced with auto-load 24h
  trending (user requested explicit landing state). Search bar in toolbar
  beside gear, not modal. Trending list also accessible via gear menu so
  user can re-load without restarting wiki tab.

## Gotchas (still apply)
- **UniFFI bindings stale** — after Rust UniFFI surface changes, run
  `./scripts/build-seoyu-xcframework.sh` manually before `build-dev.sh`.
- **`MessageHistoryOperation.Remove` is internal** — fire site must
  remain inside Postbox module.
- **Submodule + main repo are two repos** — Postbox change committed
  inside `submodules/telegram-ios`; main repo bumped pointer.
- **TGUIKit responder model intercepts NSSearchField** — must register
  via `window.set(responder:)` for keystrokes to land. Wiki search field
  does this self-scoped (only owns keys when its view tree has focus).
- Schema migrations are one-shot. Resetting wiki data locally:
  `sqlite3 tg-korean-search.db "DELETE FROM wiki_categories; DELETE FROM wiki_topics; DELETE FROM wiki_classify_queue;"`

## Context
- **Branch (main repo)**: `main` @ `0f28619c1` (4 ahead of origin/main).
- **Branch (submodule)**: `60b026c90b` (Postbox observer committed; many
  unrelated Package.swift mods still dirty in working tree).
- **Tests**: sidecar untouched this session; last green @ `07d552e6b`.
  `cargo test`/`fmt`/`clippy` not re-run (no Rust changes). `xcodebuild`
  Debug arm64 green; final build at 2026-04-28 16:28.
- **Build artifact**: `Telegram.app` at
  `~/Library/Developer/Xcode/DerivedData/Telegram-Mac-dev/Build/Products/Debug/Telegram.app`.
