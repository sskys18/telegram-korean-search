# Session Handoff
> Generated: 2026-04-23 21:52

## Task
Wiki panel UI — Phase 3-5 landed; placement iterating. Currently a
right-side **window-expand** panel toggled by a titlebar button.
User feedback: **"the app is shrink and expand not just only expand
and collapse"**. Needs another round of visual polish / clarification
on next session.

## Status

### Completed (shipped to `main`)
- Phases 1-5 of `docs/plans/2026-04-23-wiki-panel-ui.md` (PR #27 merged).
- Sidecar chosung fully purged (`refactor(seoyu): purge chosung ...`).
  `cargo fmt`/`clippy -D warnings`/`cargo test` green. 107 lib +
  integration pass, 1 ignored.
- Xcode build green via `./scripts/build-dev.sh`. Verified via
  `/tmp/tg*.stderr`: `[seoyu] opened store`, `wiki worker started`,
  `wiki observer attached`.
- Child VC lifecycle workaround: `WikiListViewController` and
  `WikiArticleViewController` expose `forceReload()` called by
  `WikiTabController.push()` — viewDidAppear is unreliable without
  NSViewController parenting (TGUIKit.ViewController is NSObject).
- Design iteration commits on the wiki toggle / placement:
  - `5b276dd48` in-window right overlay (rejected: "broken inside")
  - `c3469121c` toggle moved to titlebar accessory
  - `306adb1a3` overlay mounted on `window.contentView`
  - `9f35d7ef5` standalone NSWindow (rejected: not what user wanted)
  - `43cf55b55` window grows by 380px, chat pinned (**current**)
  - `b0f5f8d9a` snap (no animation)

### In Progress / Open Question
- User says the visual still "shrinks and expands" wrongly. Options:
  (X) shrink chat in-place, window unchanged; (Y) window grows
  right, chat pinned; (Z) overlay chat with panel. User confirmed Y
  earlier, then pushed back. Likely wants **X** — chat compresses
  inward, window width stays the same. Ask to confirm and re-pivot.

## Resume Here
1. Clarify with user: "Should the Telegram window STAY the same size
   and chat area shrink to make room for wiki (option X), or
   something else?" Get explicit answer.
2. If X: in `Telegram-Mac/MainViewController.swift` `openWikiPanel()`,
   **drop** the `window.setFrame(wf...)` growth call and the
   `wikiMainSavedAutoresizing` pin — just add the 380px panel on the
   right and rely on `layoutWikiExpansion()` (already present) to
   narrow the main content. Mirror in `closeWikiPanel()`.
3. Rebuild: `pkill -f "Telegram-Mac-dev.*MacOS/Telegram" ; ./scripts/build-dev.sh`
4. Launch + verify: `~/Library/Developer/Xcode/DerivedData/Telegram-Mac-dev/Build/Products/Debug/Telegram.app/Contents/MacOS/Telegram > /tmp/tg.stderr 2>&1 &`
5. `grep seoyu /tmp/tg.stderr` — expect 3 `[seoyu]` lines.
6. Commit, push, update handoff.

## Decisions (locked — do not re-debate)
- **Chosung removed entirely**. `search/hangul.rs` has `decompose_jamo`
  only. Schema v7 `migrate_drop_chosung` stays for legacy installs.
- **Toggle lives in titlebar accessory** (`WikiTitlebarAccessory`
  at `MainViewController.swift:~400`), SF Symbol `book.closed`.
- **No standalone NSWindow**. User rejected a separate window.
- **FastSettings.wikiPanelShown** persists open/closed state across
  launches (`FastSettings.swift`).

## Gotchas
- `TGUIKit.ViewController` extends `NSObject`, not `NSViewController`
  → no `addChild`/`removeFromParent`, no automatic `viewDidAppear`.
  `WikiTabController.push()` must call `forceReload()` explicitly.
- `nm Telegram` (the 39K launcher stub) returns 0 wiki symbols.
  Real binary is `Telegram.debug.dylib` (440MB) — 1800+ wiki
  symbols live there.
- `build-dev.sh` uses `Telegram-Mac-dev` DerivedData (separate from
  Xcode GUI's `Telegram-Mac-basjkgxsmvqzctcrxcuexrxbttgq`). Always
  launch from the `-dev` path after a CLI build.
- 6 wiki Swift files registered in `Telegram.xcodeproj/project.pbxproj`
  via ruby xcodeproj gem. If anyone drops these, re-run the one-shot
  script logic from an earlier turn (added to group `/Telegram-Mac/Seoyu/Wiki`).
- `WikiListViewController`'s NSStackView-based layout looked fine in
  440x700 but has not been tested at sub-380 widths.

## Context
- **Branch**: `main` @ `b0f5f8d9a` (pushed).
- **Tests**: sidecar 107 passed / 1 ignored; Xcode `** BUILD SUCCEEDED **`
  at 21:50 (last build, `/tmp/build10.log`).
- **Plans**: `docs/plans/2026-04-23-wiki-panel-ui.md` (shipped),
  `docs/specs/2026-04-23-wiki-panel-design.md` (original tab design,
  superseded), `docs/specs/2026-04-23-wiki-side-panel.md` (panel
  decision).
- **Running app**: PID from `pgrep -lf Telegram-Mac-dev`. Logs via
  the stderr capture approach — NSLog here does not surface via
  `/usr/bin/log show`.
