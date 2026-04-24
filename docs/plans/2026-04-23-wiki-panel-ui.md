# Plan: Wiki Panel UI

> 2026-04-23 — implements `docs/specs/2026-04-23-wiki-panel-design.md`.
> Depends on `docs/plans/2026-04-23-wiki-worker-autostart.md` being
> merged first (worker is running + `start_wiki_worker` exposed).

## Scope

Single subsystem: wiki panel UI + supporting sidecar FFI. Broken into
five phases, each touching ≤5 files, each producing a working tree
(compiles + tests green). Separate commits per task.

Out of scope: historical backfill, chat-scoped wiki, article editing,
push notifications, feedback loops.

## Phases at a glance

| # | Phase | Files | Ships |
|---|-------|-------|-------|
| 1 | Sidecar FFI surface | 5 | WikiObserver trait, new records, new methods, ForeignEmitter, worker wake flag, tests |
| 2 | Swift observer + tab shell | 5 | WikiObserverBridge, WikiLocale, empty WikiTabController, MainViewController hookup, xcodeproj entries |
| 3 | Swift trending list | 4 | WikiListViewController, digest card, category chips, table data source |
| 4 | Swift article view | 4 | WikiArticleViewController, MarkdownRenderer, WikiSourceCellView, source→chat nav |
| 5 | Polish + verify | 3 | language toggle, manual-classify wiring, empty/error states, smoke test |

After each phase: `cargo fmt --check && cargo clippy -- -D warnings &&
cargo test` + Xcode build. No phase merges until green.

---

## Phase 1 — Sidecar FFI surface

Goal: Rust side of the contract. Swift cannot compile yet (generated
bindings update at task P1.T7), but `cargo test` passes.

### P1.T1 — Add records + foreign trait

**File:** `sidecar/src/uniffi_api.rs`

Insert after the existing `WikiTopicDetail` record:

```rust
#[derive(uniffi::Record, Clone)]
pub struct WikiDigest {
    pub date_ymd: String,
    pub topic_count: i64,
    pub message_count: i64,
    pub hot_topics: Vec<WikiTopicSummary>,
}

#[derive(uniffi::Record, Clone)]
pub struct WikiCategory {
    pub id: i64,
    pub name: String,
    pub name_ko: Option<String>,
    pub topic_count: i64,
}

#[uniffi::export(with_foreign)]
pub trait WikiObserver: Send + Sync {
    fn on_progress(&self, processed: u64, pending: u64, total: u64);
    fn on_error(&self, message: String, recoverable: bool);
    fn on_topics_changed(&self);
}
```

Verify: `cd sidecar && cargo build`. Expect compile OK.

Commit: `feat(ffi): WikiObserver trait + digest/category records`

### P1.T2 — Extend `Seoyu` state

**File:** `sidecar/src/uniffi_api.rs`

Replace the `Seoyu` struct (already has `store` + `wiki_worker` from
autostart plan):

```rust
#[derive(uniffi::Object)]
pub struct Seoyu {
    store: Arc<Mutex<Store>>,
    wiki_worker: Mutex<Option<WorkerHandle>>,
    wiki_observer: Arc<Mutex<Option<Arc<dyn WikiObserver>>>>,
    wiki_wake: Arc<AtomicBool>,
}
```

Update `new(...)`:

```rust
        Ok(Arc::new(Seoyu {
            store: Arc::new(Mutex::new(store)),
            wiki_worker: Mutex::new(None),
            wiki_observer: Arc::new(Mutex::new(None)),
            wiki_wake: Arc::new(AtomicBool::new(false)),
        }))
```

Add imports at top of file:

```rust
use std::sync::atomic::AtomicBool;
```

(And verify `std::sync::{Arc, Mutex}` already imported — it is.)

Commit: `feat(ffi): Seoyu holds observer slot + wake flag`

### P1.T3 — ForeignEmitter adapter

**File:** `sidecar/src/wiki/worker.rs`

Append under `LogEmitter`:

```rust
use std::sync::atomic::AtomicI64;

/// Emitter that forwards events to a foreign (Swift) observer. Falls
/// back to `LogEmitter` behavior when no observer is set. Debounces
/// `on_topics_changed` to at most one call per 500 ms.
pub struct ForeignEmitter {
    pub observer: Arc<Mutex<Option<Arc<dyn crate::uniffi_api::WikiObserver>>>>,
    last_topics_emit_ms: AtomicI64,
}

impl ForeignEmitter {
    pub fn new(
        observer: Arc<Mutex<Option<Arc<dyn crate::uniffi_api::WikiObserver>>>>,
    ) -> Self {
        Self {
            observer,
            last_topics_emit_ms: AtomicI64::new(0),
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn observer(&self) -> Option<Arc<dyn crate::uniffi_api::WikiObserver>> {
        self.observer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

impl EventEmitter for ForeignEmitter {
    fn wiki_progress(&self, processed: u64, pending: u64, total: u64) {
        if let Some(o) = self.observer() {
            o.on_progress(processed, pending, total);
        } else {
            log::info!(
                "wiki progress: processed={processed} pending={pending} total={total}"
            );
        }
    }

    fn wiki_error(&self, message: &str, recoverable: bool) {
        if let Some(o) = self.observer() {
            o.on_error(message.to_string(), recoverable);
        } else {
            log::warn!("wiki error (recoverable={recoverable}): {message}");
        }
    }

    fn wiki_stopped(&self, reason: &str) {
        log::info!("wiki stopped: {reason}");
    }
}

impl ForeignEmitter {
    /// Called by the worker after each successful batch. Debounced.
    pub fn topics_changed(&self) {
        let now = Self::now_ms();
        let last = self.last_topics_emit_ms.load(Ordering::Relaxed);
        if now - last < 500 {
            return;
        }
        self.last_topics_emit_ms.store(now, Ordering::Relaxed);
        if let Some(o) = self.observer() {
            o.on_topics_changed();
        }
    }
}
```

Then in `run_worker`, after the existing successful-batch block
(specifically after `processed_count += results.len();`), call
`emitter.topics_changed()` — but `emitter: Arc<E>` is generic.
Introduce a method on `EventEmitter`:

```rust
pub trait EventEmitter: Send + Sync + 'static {
    fn wiki_progress(&self, processed: u64, pending: u64, total: u64);
    fn wiki_error(&self, message: &str, recoverable: bool);
    fn wiki_stopped(&self, reason: &str);
    fn wiki_topics_changed(&self) {}  // default no-op
}
```

Add override on `ForeignEmitter`:

```rust
impl EventEmitter for ForeignEmitter {
    // ...other methods as above...
    fn wiki_topics_changed(&self) {
        self.topics_changed();
    }
}
```

Call site in `run_worker` (immediately after
`processed_count += results.len();`):

```rust
                emitter.wiki_topics_changed();
```

Verify: `cd sidecar && cargo build`.

Commit: `feat(wiki): ForeignEmitter forwards to Swift observer`

### P1.T4 — Worker wake flag

**File:** `sidecar/src/wiki/worker.rs`

Add wake parameter to `start_worker`:

```rust
pub fn start_worker<E>(
    store: Arc<Mutex<Store>>,
    emitter: Arc<E>,
    wake: Arc<AtomicBool>,
) -> std::io::Result<WorkerHandle>
where
    E: EventEmitter,
{
    // unchanged apart from passing `wake` into `run_worker`
    let wake_clone = Arc::clone(&wake);
    // ...
    rt.block_on(run_worker(store, emitter, shutdown_clone, wake_clone));
```

Update `run_worker` signature:

```rust
async fn run_worker<E>(
    store: Arc<Mutex<Store>>,
    emitter: Arc<E>,
    shutdown: Arc<AtomicBool>,
    wake: Arc<AtomicBool>,
)
```

Replace the empty-queue sleep:

```rust
        if items.is_empty() {
            // Break idle sleep early if someone flipped the wake flag
            for _ in 0..20 {
                if shutdown.load(Ordering::Relaxed)
                    || wake.load(Ordering::Relaxed)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            wake.store(false, Ordering::Relaxed);
            emit_progress(&emitter, &store);
            continue;
        }
```

Callers of `start_worker` need updating. The only caller so far is
the autostart plan's `start_wiki_worker`. Update it (T5).

Verify: `cargo build` (expect compile error in `uniffi_api.rs`; fixed
in T5).

Commit: `feat(wiki): worker honors wake flag to skip idle sleep`

### P1.T5 — Wire observer + wake into `Seoyu`

**File:** `sidecar/src/uniffi_api.rs`

Replace `start_wiki_worker`:

```rust
    pub fn start_wiki_worker(&self) -> Result<(), SeoyuError> {
        let mut guard = self
            .wiki_worker
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            return Ok(());
        }
        let emitter = Arc::new(crate::wiki::worker::ForeignEmitter::new(
            Arc::clone(&self.wiki_observer),
        ));
        let handle = crate::wiki::worker::start_worker(
            Arc::clone(&self.store),
            emitter,
            Arc::clone(&self.wiki_wake),
        )
        .map_err(|e| SeoyuError::Other(format!("spawn wiki worker: {e}")))?;
        *guard = Some(handle);
        Ok(())
    }
```

Add:

```rust
    pub fn set_wiki_observer(&self, observer: Option<Arc<dyn WikiObserver>>) {
        let mut slot = self
            .wiki_observer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *slot = observer;
    }

    pub fn wiki_run_pending_now(&self) {
        self.wiki_wake.store(true, Ordering::Relaxed);
    }
```

Import:

```rust
use std::sync::atomic::Ordering;
```

Update the `Drop` impl to clear the observer before stopping the
worker:

```rust
impl Drop for Seoyu {
    fn drop(&mut self) {
        // Clear observer first so the worker cannot fire a callback
        // into Swift after Seoyu begins tearing down.
        self.set_wiki_observer(None);
        self.stop_wiki_worker();
    }
}
```

Verify: `cd sidecar && cargo build`.

Commit: `feat(ffi): Seoyu wires observer + wake flag to worker`

### P1.T6 — Query methods

**File:** `sidecar/src/uniffi_api.rs`

Append inside `#[uniffi::export] impl Seoyu`:

```rust
    pub fn wiki_digest_today(&self) -> Result<WikiDigest, SeoyuError> {
        let store = self.lock_store();

        // Today = [00:00 local, now]. Use chrono-free local conversion
        // via `libc::localtime`; fall back to UTC if unavailable.
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (day_start, ymd) = local_day_start(now_secs);

        let (topic_count, message_count) = store.wiki_counts_since(day_start)?;
        let hot_topics = store
            .get_trending_topics(3, 0, None)?
            .into_iter()
            .map(wiki_topic_to_summary)
            .collect();
        Ok(WikiDigest {
            date_ymd: ymd,
            topic_count,
            message_count,
            hot_topics,
        })
    }

    pub fn wiki_topic_messages(
        &self,
        topic_id: i64,
        limit: u32,
    ) -> Result<Vec<SearchHit>, SeoyuError> {
        let store = self.lock_store();
        let rows = store.get_topic_messages(topic_id, limit as usize)?;
        Ok(rows.into_iter().map(topic_row_to_hit).collect())
    }

    pub fn wiki_categories(&self) -> Result<Vec<WikiCategory>, SeoyuError> {
        let store = self.lock_store();
        let cats = store.get_categories_with_counts()?;
        Ok(cats
            .into_iter()
            .map(|c| WikiCategory {
                id: c.id,
                name: c.name,
                name_ko: c.name_ko,
                topic_count: c.topic_count,
            })
            .collect())
    }
```

And helpers at the bottom of the file (internal, not exposed):

```rust
fn local_day_start(now_secs: i64) -> (i64, String) {
    // Portable: get the offset by calling libc::localtime vs gmtime.
    // Avoids pulling chrono just for one method.
    use std::mem::MaybeUninit;
    unsafe {
        let t: libc::time_t = now_secs as libc::time_t;
        let mut local: MaybeUninit<libc::tm> = MaybeUninit::uninit();
        if libc::localtime_r(&t, local.as_mut_ptr()).is_null() {
            return (now_secs - now_secs.rem_euclid(86_400), "1970-01-01".into());
        }
        let local = local.assume_init();
        let ymd = format!(
            "{:04}-{:02}-{:02}",
            local.tm_year + 1900,
            local.tm_mon + 1,
            local.tm_mday,
        );
        // Recompute timestamp for today's 00:00 local: subtract today's h/m/s
        // plus the struct's gmtoff-derived offset already baked into tm.
        let day_start = now_secs
            - (local.tm_hour as i64) * 3600
            - (local.tm_min as i64) * 60
            - (local.tm_sec as i64);
        (day_start, ymd)
    }
}

fn topic_row_to_hit(row: crate::store::wiki_topic::TopicMessageRow) -> SearchHit {
    SearchHit {
        chat_id: row.chat_id,
        message_id: row.message_id,
        timestamp: row.timestamp,
        text: row.text,
        link: row.link,
        chat_title: row.chat_title,
        highlight_starts: Vec::new(),
        highlight_ends: Vec::new(),
    }
}
```

Add `libc = "0.2"` to `sidecar/Cargo.toml` under `[dependencies]` if
absent.

### P1.T6b — Store helpers

**File:** `sidecar/src/store/wiki_stats.rs`

Add:

```rust
impl Store {
    /// Count distinct (topic_id) and (chat_id, message_id) pairs whose
    /// most recent link was recorded at or after `since_ts`.
    pub fn wiki_counts_since(&self, since_ts: i64) -> Result<(i64, i64), sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT
                (SELECT COUNT(DISTINCT topic_id)
                 FROM wiki_topic_messages
                 WHERE message_timestamp >= ?1),
                (SELECT COUNT(*)
                 FROM wiki_topic_messages
                 WHERE message_timestamp >= ?1)
            ",
        )?;
        stmt.bind((1, since_ts))?;
        stmt.next()?;
        Ok((stmt.read::<i64, _>(0)?, stmt.read::<i64, _>(1)?))
    }
}
```

**File:** `sidecar/src/store/wiki_topic.rs`

Add (near existing `link_message_to_topic`):

```rust
#[derive(Debug, Clone)]
pub struct TopicMessageRow {
    pub chat_id: i64,
    pub message_id: i64,
    pub timestamp: i64,
    pub text: String,
    pub link: Option<String>,
    pub chat_title: String,
}

impl Store {
    pub fn get_topic_messages(
        &self,
        topic_id: i64,
        limit: usize,
    ) -> Result<Vec<TopicMessageRow>, sqlite::Error> {
        let mut stmt = self.conn().prepare(format!(
            "SELECT m.chat_id, m.message_id, m.timestamp, m.text_plain, m.link,
                    COALESCE(c.title, '')
             FROM wiki_topic_messages wtm
             JOIN messages m ON m.chat_id = wtm.chat_id
                             AND m.message_id = wtm.message_id
             LEFT JOIN chats c ON c.chat_id = m.chat_id
             WHERE wtm.topic_id = ?
             ORDER BY m.timestamp DESC
             LIMIT {limit}",
        ))?;
        stmt.bind((1, topic_id))?;
        let mut out = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            out.push(TopicMessageRow {
                chat_id: stmt.read::<i64, _>(0)?,
                message_id: stmt.read::<i64, _>(1)?,
                timestamp: stmt.read::<i64, _>(2)?,
                text: stmt.read::<String, _>(3)?,
                link: stmt.read::<Option<String>, _>(4)?,
                chat_title: stmt.read::<String, _>(5)?,
            });
        }
        Ok(out)
    }
}
```

**File:** `sidecar/src/store/wiki_category.rs`

Add:

```rust
#[derive(Debug, Clone)]
pub struct CategoryWithCount {
    pub id: i64,
    pub name: String,
    pub name_ko: Option<String>,
    pub topic_count: i64,
}

impl Store {
    pub fn get_categories_with_counts(&self)
        -> Result<Vec<CategoryWithCount>, sqlite::Error>
    {
        let mut stmt = self.conn().prepare(
            "SELECT c.category_id, c.name, c.name_ko,
                    (SELECT COUNT(*) FROM wiki_topics t WHERE t.category_id = c.category_id)
             FROM wiki_categories c
             ORDER BY topic_count DESC",
        )?;
        let mut out = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            out.push(CategoryWithCount {
                id: stmt.read::<i64, _>(0)?,
                name: stmt.read::<String, _>(1)?,
                name_ko: stmt.read::<Option<String>, _>(2)?,
                topic_count: stmt.read::<i64, _>(3)?,
            });
        }
        Ok(out)
    }
}
```

Verify:

```bash
cd sidecar
cargo build
cargo test wiki_counts_since
cargo test get_topic_messages
cargo test get_categories_with_counts
```

(If those test names don't exist yet, P1.T6c adds them.)

Commit: `feat(store): wiki counts/messages/categories queries`

### P1.T6c — Store tests

**File:** `sidecar/src/store/wiki_stats.rs`, inside existing `#[cfg(test)]`.

```rust
    #[test]
    fn wiki_counts_since_filters_by_timestamp() {
        let store = Store::open_in_memory().unwrap();
        seed_minimal_topic_links(&store, &[
            (1, 10, 100),  // (topic_id, msg_id, ts)
            (1, 11, 200),
            (2, 12, 300),
        ]);
        let (topics, msgs) = store.wiki_counts_since(150).unwrap();
        assert_eq!(topics, 2);
        assert_eq!(msgs, 2);
    }
```

Helper `seed_minimal_topic_links` lives in the test module — copy the
pattern from existing wiki_stats tests (they already seed topics +
messages).

Mirror tests for `get_topic_messages` and `get_categories_with_counts`
in their respective files, each asserting a happy path + empty case.

Verify: `cargo test` — all green.

Commit: `test(store): cover new wiki queries`

### P1.T7 — Regen bindings + verify shell build

**File:** none (runs a script).

```bash
cd /Users/sskys/Mine/telegram-korean-search
./scripts/build-seoyu-xcframework.sh
```

Expect: script succeeds, `packages/Seoyu/Sources/Seoyu/Generated/seoyu.swift`
now contains `protocol WikiObserver`, `struct WikiDigest`, etc. Inspect:

```bash
grep -c "WikiObserver\|WikiDigest\|WikiCategory\|wikiRunPendingNow" \
  packages/Seoyu/Sources/Seoyu/Generated/seoyu.swift
```

Expect: non-zero count.

Xcode build now (Swift side not yet using the new surface — should
still compile identically to pre-Phase-1):

```bash
xcodebuild build -workspace Telegram-Mac.xcworkspace -scheme Telegram \
  -configuration Debug -destination 'generic/platform=macOS' \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO \
  LD=$(pwd)/scripts/ld-cryptex-shim.sh \
  LDPLUSPLUS=$(pwd)/scripts/ld-cryptex-shim.sh
```

Expect: `** BUILD SUCCEEDED **`.

Commit: `chore(seoyu): regenerate bindings for WikiObserver surface`

---

## Phase 2 — Swift observer + tab shell

Goal: Swift classes wired in, empty tab visible in sidebar, no data yet.

### P2.T1 — WikiLocale

**File:** `Telegram-Mac/Seoyu/Wiki/WikiLocale.swift` (new)

```swift
import Foundation

public enum WikiLanguage: String {
    case en
    case ko

    public static func systemDefault() -> WikiLanguage {
        if let code = Locale.current.language.languageCode?.identifier,
           code == "ko" {
            return .ko
        }
        return .en
    }
}

public enum WikiLocale {
    private static let key = "seoyu.wiki.language"

    public static var current: WikiLanguage {
        get {
            if let raw = UserDefaults.standard.string(forKey: key),
               let lang = WikiLanguage(rawValue: raw) {
                return lang
            }
            return .systemDefault()
        }
        set {
            UserDefaults.standard.set(newValue.rawValue, forKey: key)
            NotificationCenter.default.post(name: .seoyuWikiLanguageChanged, object: nil)
        }
    }
}

public extension Notification.Name {
    static let seoyuWikiLanguageChanged = Notification.Name("seoyu.wiki.language.changed")
    static let seoyuWikiTopicsChanged = Notification.Name("seoyu.wiki.topics.changed")
    static let seoyuWikiProgress = Notification.Name("seoyu.wiki.progress")
    static let seoyuWikiError = Notification.Name("seoyu.wiki.error")
}
```

Commit: `feat(wiki): WikiLocale + Notification names`

### P2.T2 — WikiObserverBridge

**File:** `Telegram-Mac/Seoyu/Wiki/WikiObserverBridge.swift` (new)

```swift
import Foundation
import Seoyu

/// Bridges the UniFFI `WikiObserver` callback trait to NotificationCenter
/// posts on the main queue. A single instance is held by SeoyuBridge
/// for the app's lifetime.
public final class WikiObserverBridge: WikiObserver {
    public init() {}

    public func onProgress(processed: UInt64, pending: UInt64, total: UInt64) {
        DispatchQueue.main.async {
            NotificationCenter.default.post(
                name: .seoyuWikiProgress,
                object: nil,
                userInfo: [
                    "processed": processed,
                    "pending": pending,
                    "total": total,
                ]
            )
        }
    }

    public func onError(message: String, recoverable: Bool) {
        DispatchQueue.main.async {
            NotificationCenter.default.post(
                name: .seoyuWikiError,
                object: nil,
                userInfo: [
                    "message": message,
                    "recoverable": recoverable,
                ]
            )
        }
    }

    public func onTopicsChanged() {
        DispatchQueue.main.async {
            NotificationCenter.default.post(name: .seoyuWikiTopicsChanged, object: nil)
        }
    }
}
```

Commit: `feat(wiki): WikiObserverBridge`

### P2.T3 — WikiTabController shell

**File:** `Telegram-Mac/Seoyu/Wiki/WikiTabController.swift` (new)

```swift
import Cocoa
import Seoyu

public final class WikiTabController: NSViewController {
    private let seoyu: Seoyu
    private let navigationStack = NSStackView()

    public init(seoyu: Seoyu) {
        self.seoyu = seoyu
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    public override func loadView() {
        let container = NSView(frame: .zero)
        container.wantsLayer = true
        container.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        self.view = container

        navigationStack.orientation = .vertical
        navigationStack.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(navigationStack)
        NSLayoutConstraint.activate([
            navigationStack.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            navigationStack.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            navigationStack.topAnchor.constraint(equalTo: container.topAnchor),
            navigationStack.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])

        let placeholder = NSTextField(labelWithString: "Wiki — coming online")
        placeholder.alignment = .center
        navigationStack.addArrangedSubview(placeholder)
    }
}
```

This is a scaffold. Phase 3 replaces the placeholder with the real
list controller.

Commit: `feat(wiki): WikiTabController scaffold`

### P2.T4 — Wire observer + tab

**File:** `Telegram-Mac/Seoyu/SeoyuBridge.swift`

Hold the bridge:

```swift
    private let wikiObserverBridge = WikiObserverBridge()
```

In `attach(postbox:)`, after `try seoyu.startWikiWorker()`:

```swift
            seoyu.setWikiObserver(observer: self.wikiObserverBridge)
            NSLog("[seoyu] wiki observer attached")
```

In `deinit`, before `stopWikiWorker`:

```swift
        self.seoyu?.setWikiObserver(observer: nil)
```

**File:** `Telegram-Mac/MainViewController.swift`

Locate the existing tab registration for Contacts/Calls — expect a
call site like `tabController.add(tabItem:controller:)` or similar.
If the fork uses a hard-coded switch / enum of tabs, extend it.

Minimal change: after the Contacts tab entry, add:

```swift
        if let seoyu = SeoyuBridge.shared.seoyu {
            let wiki = WikiTabController(seoyu: seoyu)
            // Use whatever the existing pattern is. Placeholder shape:
            tabController.add(tabItem: .wiki, controller: wiki)
        }
```

The exact method signature depends on the fork — downstream
investigator task: read 50 lines of context around existing tab adds,
mirror the shape exactly. If a new enum case is needed, add `.wiki`
with the `book` SF Symbol image and localized title "Wiki" / "위키".

### P2.T5 — xcodeproj + build

Add the four new files to `Telegram.xcodeproj/project.pbxproj` under
the Telegram target. Either:

- (preferred) open Xcode, drag `Telegram-Mac/Seoyu/Wiki/` into the
  project navigator, select the Telegram target, commit the diff; or
- run the project's `xcodeproj_add_file.rb` script if one exists; or
- edit `project.pbxproj` by hand following the pattern of the
  existing `Telegram-Mac/Seoyu/SeoyuBridge.swift` entries.

Verify:

```bash
xcodebuild build -workspace Telegram-Mac.xcworkspace -scheme Telegram \
  -configuration Debug -destination 'generic/platform=macOS' \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO \
  LD=$(pwd)/scripts/ld-cryptex-shim.sh \
  LDPLUSPLUS=$(pwd)/scripts/ld-cryptex-shim.sh
```

Expect: `** BUILD SUCCEEDED **`. Launching the app must now show a
Wiki tab whose content reads "Wiki — coming online".

Commit: `feat(wiki): sidebar tab placeholder live`

---

## Phase 3 — Trending list

Goal: real data in the tab. Article view still routes to a stub.

### P3.T1 — List view controller

**File:** `Telegram-Mac/Seoyu/Wiki/WikiListViewController.swift` (new)

```swift
import Cocoa
import Seoyu

public final class WikiListViewController: NSViewController,
    NSTableViewDataSource, NSTableViewDelegate
{
    private let seoyu: Seoyu
    private let tableView = NSTableView()
    private let digestView = WikiDigestCardView()
    private let chipsView = WikiCategoryChipsView()

    private var topics: [WikiTopicSummary] = []
    private var selectedCategory: String? = nil

    public var onTopicSelected: ((WikiTopicSummary) -> Void)?

    public init(seoyu: Seoyu) {
        self.seoyu = seoyu
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    public override func loadView() {
        // Root vertical stack:
        //  [digestView]
        //  [chipsView]
        //  [NSScrollView -> tableView]
        let root = NSStackView()
        root.orientation = .vertical
        root.spacing = 8
        root.edgeInsets = NSEdgeInsets(top: 8, left: 8, bottom: 0, right: 8)
        root.translatesAutoresizingMaskIntoConstraints = false

        root.addArrangedSubview(digestView)
        root.addArrangedSubview(chipsView)

        let column = NSTableColumn(identifier: .init("topic"))
        column.width = 400
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.rowHeight = 44
        tableView.dataSource = self
        tableView.delegate = self
        tableView.target = self
        tableView.action = #selector(onRowClicked)

        let scroll = NSScrollView()
        scroll.documentView = tableView
        scroll.hasVerticalScroller = true
        root.addArrangedSubview(scroll)

        let container = NSView()
        container.addSubview(root)
        NSLayoutConstraint.activate([
            root.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            root.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            root.topAnchor.constraint(equalTo: container.topAnchor),
            root.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        self.view = container

        chipsView.onCategorySelected = { [weak self] name in
            self?.selectedCategory = name
            self?.reload()
        }

        NotificationCenter.default.addObserver(
            forName: .seoyuWikiTopicsChanged,
            object: nil,
            queue: .main
        ) { [weak self] _ in self?.throttledReload() }

        NotificationCenter.default.addObserver(
            forName: .seoyuWikiLanguageChanged,
            object: nil,
            queue: .main
        ) { [weak self] _ in self?.tableView.reloadData() }
    }

    public override func viewDidAppear() {
        super.viewDidAppear()
        reload()
    }

    // MARK: data

    private var lastReload: Date = .distantPast

    private func throttledReload() {
        let now = Date()
        if now.timeIntervalSince(lastReload) < 0.5 {
            return
        }
        lastReload = now
        reload()
    }

    private func reload() {
        let seoyu = self.seoyu
        let cat = self.selectedCategory
        DispatchQueue.global(qos: .userInitiated).async {
            let topics = (try? seoyu.wikiTrending(limit: 40, offset: 0, category: cat)) ?? []
            let digest = try? seoyu.wikiDigestToday()
            let cats = (try? seoyu.wikiCategories()) ?? []
            DispatchQueue.main.async {
                self.topics = topics
                self.digestView.configure(with: digest)
                self.chipsView.configure(with: cats, selected: cat)
                self.tableView.reloadData()
            }
        }
    }

    @objc private func onRowClicked() {
        let row = tableView.clickedRow
        guard row >= 0, row < topics.count else { return }
        onTopicSelected?(topics[row])
    }

    // MARK: NSTableViewDataSource

    public func numberOfRows(in tableView: NSTableView) -> Int { topics.count }

    public func tableView(
        _ tableView: NSTableView,
        viewFor tableColumn: NSTableColumn?,
        row: Int
    ) -> NSView? {
        let topic = topics[row]
        let cell = NSTableCellView()
        cell.identifier = .init("topicCell")
        let title = NSTextField(labelWithString: titleForCurrentLanguage(topic))
        title.translatesAutoresizingMaskIntoConstraints = false
        title.lineBreakMode = .byTruncatingTail
        let count = NSTextField(labelWithString: "\(topic.messageCount) msgs")
        count.textColor = .secondaryLabelColor
        count.translatesAutoresizingMaskIntoConstraints = false
        let row = NSStackView(views: [title, NSView(), count])
        row.orientation = .horizontal
        row.translatesAutoresizingMaskIntoConstraints = false
        cell.addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 12),
            row.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -12),
            row.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
        ])
        return cell
    }

    private func titleForCurrentLanguage(_ topic: WikiTopicSummary) -> String {
        switch WikiLocale.current {
        case .ko:
            if let ko = topic.titleKo, !ko.isEmpty { return ko }
            return topic.title
        case .en:
            return topic.title
        }
    }
}
```

### P3.T2 — Digest card

**File:** `Telegram-Mac/Seoyu/Wiki/WikiDigestCardView.swift` (new)

Small view: two labels ("N topics · M msgs today") + horizontal list
of top-3 hot topic titles, tappable to seed the selected category /
topic. Empty digest → hide the view. Target: 40 LOC.

### P3.T3 — Category chips

**File:** `Telegram-Mac/Seoyu/Wiki/WikiCategoryChipsView.swift` (new)

Horizontal `NSStackView` of round-rect NSButton, first 6 visible plus
an overflow button that expands. `All` chip is always present.
Selected chip has accent-tinted background. Target: 80 LOC.

### P3.T4 — Plug list into tab

**File:** `Telegram-Mac/Seoyu/Wiki/WikiTabController.swift`

Replace the placeholder body. Hold a `WikiListViewController` as the
current "page" of the nav stack; set its `onTopicSelected` callback
to push an article view (stub for now).

```swift
    private lazy var listController = WikiListViewController(seoyu: seoyu)
    private var currentPage: NSViewController?

    public override func loadView() {
        // ...container setup as before...
        push(listController, animated: false)
        listController.onTopicSelected = { [weak self] topic in
            let stub = NSViewController()
            let label = NSTextField(labelWithString: "Topic \(topic.id)")
            stub.view = label
            self?.push(stub, animated: true)
        }
    }

    private func push(_ child: NSViewController, animated: Bool) {
        // Simple push: remove current from navigationStack, add child.
        // Real animation in later polish pass.
        currentPage?.view.removeFromSuperview()
        currentPage?.removeFromParent()
        addChild(child)
        navigationStack.addArrangedSubview(child.view)
        currentPage = child
    }
```

Verify: Xcode build + launch. Tab now shows digest + chips + trending
rows with actual data. Click row → stub page swaps in. No crash.

Commit: `feat(wiki): trending list + digest + category chips`

---

## Phase 4 — Article view

Goal: tap topic → bilingual article + sources → click source opens
chat at that message.

### P4.T1 — MarkdownRenderer

**File:** `Telegram-Mac/Seoyu/Wiki/MarkdownRenderer.swift` (new)

Minimal md → `NSAttributedString`. Cover: `#`/`##` headings, `**bold**`,
`*italic*`, bullet lists, inline code, paragraphs. No links (sources
are separate list), no tables. Roll by hand (~120 LOC) to avoid new
dependency.

### P4.T2 — Source cell

**File:** `Telegram-Mac/Seoyu/Wiki/WikiSourceCellView.swift` (new)

Two-line `NSTableCellView`: `@chatTitle · relativeTime` above,
truncated message text below. Relative-time formatter pre-exists in
the TelegramSwift fork — reuse `DateUtils` / whatever the fork uses.

### P4.T3 — Article view controller

**File:** `Telegram-Mac/Seoyu/Wiki/WikiArticleViewController.swift` (new)

```swift
public final class WikiArticleViewController: NSViewController {
    private let seoyu: Seoyu
    private let topicId: Int64
    private let openChat: (Int64, Int64) -> Void  // (chatId, messageId)

    private let articleView = NSTextView()
    private let sourcesTable = NSTableView()
    private var sources: [SearchHit] = []

    public init(
        seoyu: Seoyu,
        topicId: Int64,
        openChat: @escaping (Int64, Int64) -> Void
    ) {
        self.seoyu = seoyu
        self.topicId = topicId
        self.openChat = openChat
        super.init(nibName: nil, bundle: nil)
    }

    // loadView, reload, NSTableView delegates, same shape as List.
    // On source row click: openChat(hit.chatId, hit.messageId).
}
```

Reload on `.seoyuWikiLanguageChanged`: swap between `detail.articleMd`
and `detail.articleMdKo`.

### P4.T4 — Wire article + nav

**File:** `Telegram-Mac/Seoyu/Wiki/WikiTabController.swift`

Replace the stub push in P3.T4:

```swift
        listController.onTopicSelected = { [weak self] topic in
            guard let self else { return }
            let article = WikiArticleViewController(
                seoyu: self.seoyu,
                topicId: topic.id,
                openChat: { [weak self] chatId, messageId in
                    self?.openChat(chatId: chatId, messageId: messageId)
                }
            )
            self.push(article, animated: true)
        }
```

Inject the chat-open callback. Plumb it through `WikiTabController`'s
`init`:

```swift
    public init(seoyu: Seoyu, openChat: @escaping (Int64, Int64) -> Void)
```

**File:** `Telegram-Mac/MainViewController.swift`

At the site that constructs `WikiTabController`, pass in a closure
that calls the fork's existing chat-nav. Based on `AppDelegate.swift`
line 1616 pattern:

```swift
let wiki = WikiTabController(seoyu: seoyu) { [weak accountContext] chatId, messageId in
    guard let context = accountContext else { return }
    let peerId = PeerId(chatId)
    navigateToChat(
        navigation: context.bindings.rootNavigation(),
        context: context,
        chatLocation: .peer(peerId),
        messageId: MessageId(peerId: peerId, namespace: 0, id: Int32(messageId))
    )
}
```

Exact signature of `navigateToChat` lives in `AppDelegate.swift`;
copy verbatim.

Verify: Xcode build + launch. Open wiki tab → tap a topic → article
renders → click a source row → main window focuses on that chat at
that message with a highlight flash.

Commit: `feat(wiki): article view + source-message navigation`

---

## Phase 5 — Polish + verify

Goal: language toggle, manual classify, empty/error states, smoke test.

### P5.T1 — Toolbar actions

**File:** `Telegram-Mac/Seoyu/Wiki/WikiTabController.swift`

Add three toolbar buttons above the nav stack:

- **Language toggle**: title = `WikiLocale.current == .en ? "EN" : "KO"`.
  Click → `WikiLocale.current = (current == .en ? .ko : .en)`. Updates
  propagate via `Notification.Name.seoyuWikiLanguageChanged` already
  observed by list + article.
- **Search**: opens a modal sheet with an `NSTextField`; on enter,
  calls `seoyu.wikiSearch(query:, limit: 50)` and pushes a result list
  (reuse `WikiListViewController` with a pre-populated `topics`
  override — add an `init(seoyu:, seed: [WikiTopicSummary])` variant).
- **Run classify**: icon `arrow.clockwise`. Click → calls
  `seoyu.wikiRunPendingNow()`. Enabled iff latest
  `seoyuWikiProgress` notification's `pending > 0`.

### P5.T2 — Progress + error banners

**File:** `Telegram-Mac/Seoyu/Wiki/WikiListViewController.swift`

Observe `seoyuWikiProgress` and `seoyuWikiError`:

- Progress event with `total > 0 && topics.isEmpty` → show full-pane
  placeholder with inline progress bar. Hide once `topics.isEmpty`
  flips false.
- Error `recoverable: false` → inline dismissible banner at top.
  Error `recoverable: true` → 3-second toast at bottom.

### P5.T3 — Smoke test

Procedural.

1. Build + launch Telegram.app, log in.
2. Wait for ingest (log-watch for "[seoyu] wiki worker started").
3. If no `codex` CLI installed → banner appears immediately with
   "ChatGPT subscription required for wiki".
4. With `codex` available: wait for first batch. Progress bar animates.
   Topics appear in the tab without switching tabs (push model).
5. Toggle EN/KO — topic titles switch; article view re-renders on
   open.
6. Click a source row → chat opens at that message, highlighted.
7. Click "Run classify" when pending queue present → progress ticks
   faster (sleep cut short).
8. Restart the app → language preference persists.

Commit after each sub-task verifies clean:

```bash
cd sidecar && cargo fmt --check && cargo clippy -- -D warnings && cargo test
# Xcode build from P1.T7 command
```

### P5.T4 — Update handoff

**File:** `docs/handoff.md`

Mark step #2 shipped. Next resume = step #3 (historical backfill).

Commit: `docs: handoff reflects wiki panel shipped`

---

## Final verification gate

Before declaring the feature done:

1. `cd sidecar && cargo fmt --check` clean.
2. `cargo clippy -- -D warnings` clean.
3. `cargo test` all green (expect ≥ 103 lib tests).
4. Xcode build `** BUILD SUCCEEDED **`.
5. Smoke test (P5.T3) passes on a live account.
6. No uncommitted state except the expected `submodules/telegram-ios`
   working-tree noise.

## Self-review

- **Spec coverage**: every section of the design spec maps to tasks.
  Observer → P1.T3/T5 + P2.T2. Digest → P1.T6 + P3.T2. Categories →
  P1.T6 + P3.T3. Bilingual → P2.T1 + P5.T1. Manual classify →
  P1.T5/T6 + P5.T1. Source nav → P4.T4.
- **Placeholder scan**: only deferrals are to the *next* task that
  explicitly implements the item (e.g. list tasks defer nav
  implementation to Phase 4, clearly flagged).
- **Type consistency**: `WikiObserver`, `WikiDigest`, `WikiCategory`,
  `WikiTopicSummary`, `SearchHit`, `WikiLanguage`, `WikiLocale`,
  `Notification.Name.seoyuWikiTopicsChanged` spelled identically
  across phases.
- **Phase sizes**: Phase 1 = 5 files (uniffi_api, worker, wiki_stats,
  wiki_topic, wiki_category). Phase 2 = 4 new + 2 edited =
  acceptable. Phases 3–5 ≤ 5 files each.

## Execution Handoff

**Plan complete. Three execution options:**

1. **Subagent-Driven (recommended)** → sspower:subagent-driven-development
2. **Inline Execution** → sspower:executing-plans
3. **Codex execute** → delegate via `codex-bridge.mjs implement --write`
   with explicit per-phase prompts to avoid another rogue full-spec run

**Which approach?**
