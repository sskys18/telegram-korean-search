# Session Handoff
> Generated: 2026-04-23 (Phase 3-5 inline execution)

## Task
Ship wiki panel UI Phase 3-5 on `wiki-feature` branch. User chose
**option 2** from prior handoff: write Swift in worktree, defer Xcode
build verification to merge time on the main tree.

## Status

### Completed on branch `wiki-feature` (24 commits ahead of `main`)

**Phase 3 — trending list** (commit `feat(wiki): trending list + digest + category chips`):
- `Telegram-Mac/Seoyu/Wiki/WikiListViewController.swift` — table backed by
  `seoyu.wikiTrending(...)`, throttled reload on `.seoyuWikiTopicsChanged`,
  language-swap on `.seoyuWikiLanguageChanged`, optional `seed:` init for
  search results, empty-state label, progress placeholder.
- `Telegram-Mac/Seoyu/Wiki/WikiDigestCardView.swift` — "N topics · M msgs
  today" + hot-topic strip; hides when digest empty.
- `Telegram-Mac/Seoyu/Wiki/WikiCategoryChipsView.swift` — chip row with
  "All" + first 6 + overflow expander, accent-tinted selection.
- `Telegram-Mac/Seoyu/Wiki/WikiTabController.swift` — replaced placeholder
  with push-based nav stack (`push(_:)`, `popToRoot()`).

**Phase 4 — article view** (commit `feat(wiki): article view + source-message navigation`):
- `Telegram-Mac/Seoyu/Wiki/MarkdownRenderer.swift` — in-tree md →
  `NSAttributedString`. Supports `#`/`##`, `**bold**`, `*italic*`,
  inline `` `code` ``, bullets (`-`/`*`), paragraphs. No deps.
- `Telegram-Mac/Seoyu/Wiki/WikiSourceCellView.swift` — two-line source
  cell using `RelativeDateTimeFormatter`.
- `Telegram-Mac/Seoyu/Wiki/WikiArticleViewController.swift` — title +
  scrollable rendered article + sources table. Click source → invokes
  `openChat(chatId, messageId)` closure.
- `Telegram-Mac/MainViewController.swift:776` — passes an `openChat`
  closure that pushes `ChatController(... focusTarget: .init(messageId:))`
  via `context.bindings.rootNavigation()`.

**Phase 5 — polish** (commit `feat(wiki): toolbar + progress/error banners`):
- WikiTabController toolbar: EN/KO toggle, Search (modal sheet → seeded
  `WikiListViewController`), Run classify (`seoyu.wikiRunPendingNow()`,
  enabled iff `pendingCount > 0`).
- WikiListViewController: top dismissible error banner (recoverable=false),
  bottom auto-dismissing 3s toast (recoverable=true).

### NOT done (deferred, requires real build env)

- **`Telegram.xcodeproj/project.pbxproj` updates** — six new files added
  in this session are NOT in the pbxproj yet:
  - `Telegram-Mac/Seoyu/Wiki/WikiListViewController.swift`
  - `Telegram-Mac/Seoyu/Wiki/WikiDigestCardView.swift`
  - `Telegram-Mac/Seoyu/Wiki/WikiCategoryChipsView.swift`
  - `Telegram-Mac/Seoyu/Wiki/MarkdownRenderer.swift`
  - `Telegram-Mac/Seoyu/Wiki/WikiSourceCellView.swift`
  - `Telegram-Mac/Seoyu/Wiki/WikiArticleViewController.swift`
- **Xcode build verification** — worktree env still missing libwebp /
  ffmpeg / webrtc xcframeworks per prior handoff. Build moves to the
  main tree post-merge.
- **P5.T3 smoke test** — needs running app.

## Resume Here

1. **Switch to main tree** `/Users/sskys/Mine/telegram-korean-search`,
   working Xcode env there.
2. **Merge `wiki-feature` → `main`** (fast-forward expected, 24 commits).
3. In Xcode, **drag the six new files** from `Telegram-Mac/Seoyu/Wiki/`
   into the Telegram target (or run a `xcodeproj` ruby script following
   the pattern from the prior `feat(wiki): WikiTabController scaffold`
   pbxproj diff).
4. **Build the Telegram scheme** (not `All`). Verify mtime fresh:
   ```
   stat -f "%Sm" ~/Library/Developer/Xcode/DerivedData/Telegram-Mac-basjkgxsmvqzctcrxcuexrxbttgq/Build/Products/Debug/Telegram.app/Contents/MacOS/Telegram
   ```
5. **Smoke test** (P5.T3): launch, log in, wait for ingest, watch logs:
   ```
   /usr/bin/log stream --predicate 'process == "Telegram" AND eventMessage CONTAINS "seoyu"'
   ```
   Expect `[seoyu] wiki worker started` + `[seoyu] wiki observer attached`.
6. Open Wiki tab → digest + chips + trending visible. Click topic →
   article + sources. Click source row → main window focuses the chat at
   that message.
7. Toggle EN/KO → titles + article body switch language.
8. Search button → modal → enter "삼성" → seeded results list.
9. Restart app → language preference persists.

## Decisions

- **Inline execution** of Phase 3-5 in the worktree, no per-phase build
  gate — accepted by user explicitly. All four sub-phase commits land on
  `wiki-feature` already.
- **Forward reference allowed**: WikiTabController in Phase 3 commit
  references `WikiArticleViewController` (added in Phase 4 commit). Each
  commit is internally consistent within the branch but `wiki-feature@P3`
  alone won't compile — fine because we never check out that intermediate
  state.
- **PeerId/MessageId construction in MainViewController**: uses
  `PeerId(chatId)` + `MessageId(peerId:, namespace: Namespaces.Message.Cloud, id: Int32(messageId))`,
  matching the pattern at `AppDelegate.swift:1346`.

## Gotchas

- **pbxproj is the manual step**. The six Swift files are on disk +
  committed but Xcode won't see them until you add file refs. Without
  this, the build fails with `cannot find type 'WikiListViewController'`
  etc.
- **NSTextField.action firing**: the search sheet wires
  `field.action = #selector(submitSearch(_:))` so Enter submits. If you
  don't see it fire, ensure the field is the window's
  `initialFirstResponder` (already set).
- **ForeignEmitter debounces topics-changed at 500ms** AND
  WikiListViewController throttles `reload()` at 500ms — redundant but
  cheap, leave it.
- **Run-classify button disabled until first progress event with
  `pending > 0`**. On a fresh empty queue it stays disabled — expected.

## Context

- **Branch**: `wiki-feature` in `/Users/sskys/Mine/telegram-korean-search/.worktrees/wiki-feature`.
- **Main branch ref**: `c216dbb9d` (24 commits behind `wiki-feature`).
- **Tests**: sidecar `cargo test --lib` last green at 107 passed (Phase 1
  baseline). Not re-run this session — Phase 3-5 is Swift-only, sidecar
  untouched.
- **Plans**: `docs/plans/2026-04-23-wiki-panel-ui.md` — P1, P2, P3, P4,
  P5.T1, P5.T2 shipped; P5.T3 (smoke test) and pbxproj manual step
  remain.
- **Spec**: `docs/specs/2026-04-23-wiki-panel-design.md` (approved).
- **Unpushed**: entire `wiki-feature` branch.
