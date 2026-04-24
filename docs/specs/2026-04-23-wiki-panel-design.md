# Wiki Panel Design

> 2026-04-23 — covers handoff step #2 (wiki panel UI). Depends on
> `docs/plans/2026-04-23-wiki-worker-autostart.md` landing first, since
> that plan stands up the worker and the `LogEmitter` this design
> replaces with a Swift-facing observer.

## Goals

- Surface LLM-classified topics inside the Telegram fork without
  blocking or polluting native chat UX.
- Let the user jump from any wiki topic back to the source messages in
  their original chat with no copy-paste.
- Keep Korean as a first-class bilingual surface with a user toggle.
- Push model: UI updates the instant the sidecar worker finishes a
  classification batch.

## Non-goals

- Article editing, feedback loops, or user-authored wiki pages.
- Per-topic notifications or badges.
- Topic export, deletion, retraining.
- Chat-scoped wiki (wiki filtered by a single peer).
- Full keyboard navigation / accessibility polish — minimum is
  focusable rows + Enter-to-open.

## Dependencies

- **Worker autostart plan** — merged; `LogEmitter` stays as the
  fallback when no Swift observer is set.
- **UniFFI 0.31** foreign-trait callbacks (`#[uniffi::export(with_foreign)]`).
- TelegramSwift fork's `AccountContext` + chat-navigation API
  (`navigateToMessage(id:)` or equivalent — verify exact selector in
  task 1 of the downstream plan).

## User-facing surface

### Placement

New **sidebar tab** between Contacts and Calls in the fork's main
sidebar. Tab icon: SF Symbol `book`. Label: "Wiki" (English) / "위키"
(Korean locale).

### Tab scope — "rich MVP"

- Trending topic list (primary view).
- Category filter chips.
- Topic article view (push-nav'd from trending).
- Source-message list inside article view.
- Wiki FTS search bar in toolbar.
- Daily digest card at top of trending list.
- Manual "classify now" button in toolbar.
- Bilingual toggle button in toolbar.

### Layout

Single-column push-nav. No split view. Mirrors iOS-style stack inside
a single `NSViewController` container.

```
┌─────────────────── Wiki ──────────────────┐
│ [🌐 EN/KO] [🔍 search] [↻ classify]       │  toolbar
├───────────────────────────────────────────┤
│ ┌─ Today ────────────────────────────┐    │  digest card
│ │ 12 topics, 340 msgs                │    │
│ │ Hot: iOS 27 · 환율 · OpenAI        │    │
│ └────────────────────────────────────┘    │
├───────────────────────────────────────────┤
│ [All] [Tech] [정치] [Crypto] [+ more ⌄]   │  category chips
├───────────────────────────────────────────┤
│ ▸ iOS 27 beta 4            23 msgs  ★★★  │
│ ▸ 환율 1400원 돌파          18 msgs  ★★☆  │
│ ▸ OpenAI GPT-6             15 msgs  ★★☆  │
│ ...                                       │
└───────────────────────────────────────────┘
```

Tap a topic → push `WikiArticleViewController`:

```
┌───────────────────────────────────────────┐
│ [◀ Back] iOS 27 beta 4          [EN/KO]   │
├───────────────────────────────────────────┤
│ # iOS 27 beta 4                           │
│                                           │
│ bilingual markdown content...             │
│                                           │
│ ## Sources (23)                           │
│ ▸ @iosdev · 2h                            │
│   "beta 4 fixes the battery drain..."     │
│ ▸ @macrumors · 5h                         │
│   ...                                     │
└───────────────────────────────────────────┘
```

Tap a source row → main Telegram window opens the peer, scrolls to
`messageId`, flashes the message. Uses the fork's existing chat-nav
entry point (no new MTProto calls).

### Bilingual toggle

- Toolbar button, two states: `EN` / `KO`.
- Persisted in `UserDefaults` under key `seoyu.wiki.language`.
- Default = `Locale.current.language.languageCode == "ko" ? .ko : .en`.
- Fallback: when selected language missing for a topic (e.g. `title_ko`
  is nil), render the available language and tag it with a subtle
  `(en)` / `(ko)` suffix so the user knows nothing was lost.

### Empty / error states

- **First run, no topics yet**: center-aligned placeholder
  "Classifying your messages…" with an inline progress bar driven by
  `on_progress(processed, pending, total)`.
- **`codex` CLI missing** (`on_error(..., recoverable: false)`): top
  banner "ChatGPT subscription required for wiki. Korean search still
  works." Dismissible; dismissal is per-session (not persisted, so a
  later install of `codex` is noticed).
- **Network / transient failure** (`recoverable: true`): ephemeral
  toast, current data preserved.
- **No topics match a selected category**: inline "No topics in this
  category yet" + link to reset to `All`.

### Manual classify button

- Toolbar button, SF Symbol `arrow.clockwise`.
- Click → `seoyu.wikiRunPendingNow()`. Sidecar wakes the worker if it
  is in its sleep-after-empty loop.
- Does **not** re-enqueue already-classified messages. Re-classification
  is out of scope for this design.
- Button disabled while `on_progress.pending == 0` (nothing to do).

## Sidecar changes (FFI surface)

New foreign trait + new Seoyu methods. All land in
`sidecar/src/uniffi_api.rs`.

```rust
#[uniffi::export(with_foreign)]
pub trait WikiObserver: Send + Sync {
    fn on_progress(&self, processed: u64, pending: u64, total: u64);
    fn on_error(&self, message: String, recoverable: bool);
    fn on_topics_changed(&self);
}

#[derive(uniffi::Record, Clone)]
pub struct WikiDigest {
    pub date_ymd: String,          // "YYYY-MM-DD" local time
    pub topic_count: i64,          // topics touched today
    pub message_count: i64,        // messages classified today
    pub hot_topics: Vec<WikiTopicSummary>,  // top 3 by trending_score
}

#[derive(uniffi::Record, Clone)]
pub struct WikiCategory {
    pub id: i64,
    pub name: String,
    pub name_ko: Option<String>,
    pub topic_count: i64,
}
```

New methods on `Seoyu`:

```rust
pub fn set_wiki_observer(&self, observer: Option<Arc<dyn WikiObserver>>);
pub fn wiki_digest_today(&self) -> Result<WikiDigest, SeoyuError>;
pub fn wiki_topic_messages(&self, topic_id: i64, limit: u32)
    -> Result<Vec<SearchHit>, SeoyuError>;
pub fn wiki_categories(&self) -> Result<Vec<WikiCategory>, SeoyuError>;
pub fn wiki_run_pending_now(&self);
```

### Worker plumbing

- Replace default `LogEmitter` with a `ForeignEmitter` that holds
  `Arc<Mutex<Option<Arc<dyn WikiObserver>>>>`. When the slot is
  `None`, it falls through to `LogEmitter` behavior. This keeps the
  worker usable before Swift attaches an observer and in tests.
- `on_topics_changed()` fires once per successful LLM batch, after the
  `mark_queue_done` loop finishes. Debounce: skip if previous emit
  was under 500 ms ago; the skipped emit is coalesced into the next.
- `on_progress` fires on the existing cadence (once per idle loop +
  once per batch) — no change to the call sites, only the
  implementation.
- `wiki_run_pending_now()` flips an `AtomicBool` the worker checks at
  the top of its 2-second idle sleep; if set, it breaks the sleep
  early and re-reads the queue.

### Threading + lifetime

- `set_wiki_observer(None)` must be safe to call at any point; the
  worker picks up the change on the next emit.
- `Drop` on `Seoyu` clears the observer slot before `stop_wiki_worker`
  joins the thread — so no Swift callback fires during Swift's own
  deinit path.

## Swift file layout

```
Telegram-Mac/Seoyu/Wiki/
  WikiTabController.swift          — NSViewController, holds the push-nav stack
  WikiListViewController.swift     — trending list + digest header + chips
  WikiArticleViewController.swift  — article body + sources table
  WikiSourceCellView.swift         — source-message row view
  WikiObserverBridge.swift         — class: WikiObserver, main-queue dispatch + NotificationCenter post
  WikiLocale.swift                 — toggle state, UserDefaults
  MarkdownRenderer.swift           — md → NSAttributedString
```

All land under the existing `Telegram-Mac/Seoyu/` directory. No
submodule changes.

### Markdown renderer

- Evaluate whether TelegramSwift already ships a markdown-ish
  attributed-string renderer (it does for chat entity parsing). If a
  reasonable subset exists, adapt it. If not, vendor `cmark-gfm` via
  SPM (or a lighter single-file renderer). Decision deferred to the
  downstream plan's task 1 of the article view work — spike first.

### Sidebar integration

- Locate the fork's main sidebar controller (`MainViewController` or
  the equivalent — confirm in the downstream plan).
- Add a tab entry whose `viewController` lazy-inits a
  `WikiTabController(context: AccountContext)`.
- Gate the tab behind `#if !SHARE` like the other Seoyu additions.

### Source-message navigation

- Use `AccountContext`'s existing chat-open + scroll-to-message path.
  Handoff already notes `SearchController` uses the same pattern.
  Downstream plan must identify the exact call (expect
  `ChatController.makeChatController(chatLocation:messageId:)` or a
  thin wrapper in the fork).

## Data flow

1. App launch → `SeoyuBridge.bootstrap()` → `Seoyu.new(...)`.
2. `SeoyuBridge.attach(postbox:)` installs the ingest observer,
   starts the wiki worker (from the autostart plan), constructs a
   `WikiObserverBridge`, calls `seoyu.setWikiObserver(bridge)`.
3. User opens the Wiki tab → `WikiTabController.viewDidAppear(_:)`
   dispatches three FFI calls on a background queue:
   `wikiTrending(limit: 40, offset: 0, category: selected)`,
   `wikiDigestToday()`, `wikiCategories()`. Results bounce to main,
   populate views.
4. Worker finishes batch → `ForeignEmitter.on_topics_changed()` →
   `WikiObserverBridge` hops to main → posts
   `Notification.Name.seoyuWikiTopicsChanged`. List controller
   observes and reloads. Reload is throttled to at most 1 per 500 ms.
5. Tap a topic row → push `WikiArticleViewController(topicId:)` →
   background call to `wikiTopicDetail(topicId:)` +
   `wikiTopicMessages(topicId:, limit: 100)`.
6. Tap a source row → `AccountContext` opens the peer and scrolls to
   the message. The wiki tab stays in place; main chat window takes
   focus.
7. Language toggle → updates `WikiLocale.current`, triggers a
   reload-in-place (no re-fetch needed — both languages are already
   in the existing payloads).
8. Manual classify → `seoyu.wikiRunPendingNow()`; progress bar
   animates via `on_progress` events.

## Threading rules

- All FFI calls from Swift → `DispatchQueue.global(qos: .userInitiated)`.
- All `WikiObserver` callback bodies → `DispatchQueue.main.async`.
- No UI mutation off-main, no FFI call on-main beyond
  `wikiRunPendingNow()` (which is a fire-and-forget atomic flag).

## Testing

- **Rust**: unit test `ForeignEmitter` with a Rust-side fake observer
  that counts calls; verify debounce.
- **Rust**: integration test `wiki_digest_today` against a seeded
  store with `record_topic_stat` rows spanning yesterday + today;
  assert only today's window counts.
- **Rust**: integration test `wiki_run_pending_now` wakes the worker
  inside 100 ms of the flag flip.
- **Swift**: XCTest the `WikiLocale` UserDefaults round-trip and the
  debounce on `Notification.Name.seoyuWikiTopicsChanged`.
- **Manual smoke**: seed `wiki_classify_queue`, confirm tab populates,
  toggle language round-trips, tap source opens correct chat.

## Open questions (resolved by defaults, flag if wrong)

- **Digest window**: today = `[00:00 local, now]`. Rolling 24h
  rejected — too hard to explain in-UI.
- **Trending row capacity**: 40 visible, infinite scroll. Paginates
  via existing `wikiTrending(limit:offset:)` on scroll-near-bottom.
- **Category chips overflow**: first 6 visible + "more" disclosure
  that expands inline. No separate drawer.
- **Observer lifetime**: held by `SeoyuBridge` singleton for the app's
  lifetime; replaced on second `attach(...)` call.

## Rollout

- Ship behind no feature flag (single-user local client; no remote
  surface to gate).
- Tab visibility is unconditional; empty state covers pre-classification.
- On first launch after upgrade, the historical-backfill gap (handoff
  step #3) means the digest + trending list will look sparse until
  backfill ships. Acceptable — inline placeholder sets the expectation.
