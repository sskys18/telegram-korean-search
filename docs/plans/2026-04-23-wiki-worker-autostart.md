# Plan: Wiki Worker Autostart

> 2026-04-23 — implements handoff step #1: schedule `wiki::worker` from
> `SeoyuBridge.attach` so newly indexed messages auto-classify without
> user intervention. MVP uses a log-only emitter; progress plumbing to
> Swift lands in a follow-up plan.

## Scope

Single subsystem: sidecar worker lifecycle + enqueue hook + Swift start
call. Out of scope:

- Wiki panel UI (separate plan)
- Progress events surfacing to Swift (follow-up)
- Historical backfill (separate plan)
- Chat-scoped search merge (separate plan)

## Spec

**Behavior after merge:**

1. Every `Seoyu.indexMessages(...)` call enqueues the inserted rows into
   `wiki_classify_queue`.
2. `Seoyu.startWikiWorker()` spawns one background worker thread with its
   own tokio runtime. Idempotent: second call is a no-op.
3. Worker pulls from the queue, calls `codex exec`, writes topics/pages.
4. `Seoyu.stopWikiWorker()` signals shutdown and joins. Called from
   `Drop` so ARC release on the Swift side cleans up.
5. `SeoyuBridge.attach(postbox:)` calls `startWikiWorker()` once after the
   ingest observer is installed.

**Non-goals:** progress UI, user-pause/resume, retry backoff tuning.

## Files touched

| File | Change |
|------|--------|
| `sidecar/src/store/message.rs` | `insert_messages_batch` enqueues each successfully inserted row into `wiki_classify_queue` inside the existing txn. |
| `sidecar/src/store/message.rs` (tests) | One new test: insert batch, assert queue rows appear. |
| `sidecar/src/uniffi_api.rs` | Hold `Option<WorkerHandle>` on `Seoyu`; add `start_wiki_worker`, `stop_wiki_worker` UniFFI methods; implement `Drop` that stops. Use `LogEmitter` (new, tiny). |
| `sidecar/src/wiki/mod.rs` | Re-export `WorkerHandle`, `start_worker`, `EventEmitter` for crate root. (Already module, just confirm.) |
| `sidecar/src/wiki/worker.rs` | Add `pub struct LogEmitter;` impl that routes events to `log::info!`/`log::warn!`. |
| `Telegram-Mac/Seoyu/SeoyuBridge.swift` | Call `seoyu.startWikiWorker()` at end of `attach(postbox:)`. Stop on `deinit` (singleton rarely deallocs but wire for completeness). |
| `scripts/build-seoyu-xcframework.sh` | No change — regen picks up new exports. |
| `docs/handoff.md` | Flip step #1 from "Resume Here" to "Done"; next session resumes on step #2. |

Schema: **no migration**. Queue already exists at v7.

## Risk notes

- `insert_messages_batch` runs under `BEGIN ... COMMIT`. Enqueue stays
  inside that txn so rollback on insert failure unwinds queue rows too.
- `Arc<Mutex<Store>>` on Seoyu is already shared with the worker call
  site; worker will contend with Swift ingest/search calls. Lock held
  only around short DB ops — acceptable per CLAUDE.md rule.
- No `codex` subscription → `LlmClient::classify_batch` errors →
  `wiki_error` logged, queue rows marked failed, worker keeps looping.
  Not a crash.
- Worker `thread::Builder::spawn` can fail with `io::Error`. Current
  `start_worker` calls `.expect(...)`. Swap to return `Result` so
  `start_wiki_worker` reports failure up to Swift.

---

## Task 1 — Change `start_worker` to return `Result`

Rationale: Swift should see a thrown error, not a process abort, if the
OS refuses a new thread.

**File:** `sidecar/src/wiki/worker.rs`

Replace the existing `start_worker` signature and body. Keep the inner
thread body unchanged.

```rust
pub fn start_worker<E>(
    store: Arc<Mutex<Store>>,
    emitter: Arc<E>,
) -> std::io::Result<WorkerHandle>
where
    E: EventEmitter,
{
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = Arc::clone(&shutdown);

    let thread = std::thread::Builder::new()
        .name("seoyu-wiki-worker".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    log::error!("wiki worker: failed to build runtime: {e}");
                    return;
                }
            };
            rt.block_on(run_worker(store, emitter, shutdown_clone));
        })?;

    Ok(WorkerHandle { shutdown, thread })
}
```

Verify:

```bash
cd sidecar && cargo check
```

Expect: compiles. One caller site (tests, if any) flagged — fix inline.

Commit: `refactor(wiki): start_worker returns io::Result`

---

## Task 2 — Add `LogEmitter`

**File:** `sidecar/src/wiki/worker.rs`, append under `NoopEmitter`.

```rust
/// Emitter that logs every event via `log::*`. This is the default
/// used by the UniFFI-exposed worker before a real Swift-facing
/// progress callback channel exists.
pub struct LogEmitter;

impl EventEmitter for LogEmitter {
    fn wiki_progress(&self, processed: u64, pending: u64, total: u64) {
        log::info!(
            "wiki progress: processed={processed} pending={pending} total={total}"
        );
    }
    fn wiki_error(&self, message: &str, recoverable: bool) {
        log::warn!("wiki error (recoverable={recoverable}): {message}");
    }
    fn wiki_stopped(&self, reason: &str) {
        log::info!("wiki stopped: {reason}");
    }
}
```

Verify:

```bash
cd sidecar && cargo build
```

Commit: `feat(wiki): LogEmitter for pre-IPC worker output`

---

## Task 3 — Enqueue inserted rows inside `insert_messages_batch`

Only enqueue rows that actually insert (changes > 0), so repeated
ingests of the same message do not re-classify.

**File:** `sidecar/src/store/message.rs`

Inside the `for msg in messages` loop, right after the existing
`if changes > 0 { fts_insert(...); ... }` block, add the queue insert.

Locate the existing closure that ends with `fts_insert(... &jamo, ...)?`.
After that call, before the closing `}` of `if changes > 0 {`, insert:

```rust
                let mut q = self.conn.prepare(
                    "INSERT OR IGNORE INTO wiki_classify_queue (chat_id, message_id)
                     VALUES (?, ?)",
                )?;
                q.bind((1, msg.chat_id))?;
                q.bind((2, msg.message_id))?;
                q.next()?;
```

(Kept open-coded — using `self.enqueue_for_classification(&[...])` would
take a fresh prepare per call; inline prepare is cheaper inside the hot
loop and stays inside the same txn.)

Verify:

```bash
cd sidecar && cargo build
```

Commit: `feat(store): enqueue new messages for wiki classification`

---

## Task 4 — Test: insert batch → queue populated

**File:** `sidecar/src/store/message.rs`, inside the existing
`#[cfg(test)] mod tests`.

```rust
    #[test]
    fn insert_messages_batch_enqueues_classify() {
        let store = Store::open_in_memory().unwrap();
        let msgs = vec![
            MessageRow {
                message_id: 10,
                chat_id: 1,
                timestamp: 1_700_000_000,
                text_plain: "테스트 메시지".into(),
                text_stripped: "테스트메시지".into(),
                link: None,
            },
            MessageRow {
                message_id: 11,
                chat_id: 1,
                timestamp: 1_700_000_001,
                text_plain: "another".into(),
                text_stripped: "another".into(),
                link: None,
            },
        ];
        store.insert_messages_batch(&msgs).unwrap();

        let stats = store.get_queue_stats().unwrap();
        assert_eq!(stats.pending, 2);

        // Re-insert: INSERT OR IGNORE on messages means no new FTS row,
        // so no new queue row either.
        store.insert_messages_batch(&msgs).unwrap();
        let stats = store.get_queue_stats().unwrap();
        assert_eq!(stats.pending, 2);
    }
```

Verify:

```bash
cd sidecar && cargo test insert_messages_batch_enqueues_classify -- --nocapture
```

Expect: `test result: ok. 1 passed`.

Commit: `test(store): enqueue hook fires for new messages only`

---

## Task 5 — Expose start/stop on `Seoyu`

**File:** `sidecar/src/uniffi_api.rs`

Add imports near the top:

```rust
use crate::wiki::worker::{start_worker, LogEmitter, WorkerHandle};
```

Change the `Seoyu` struct:

```rust
#[derive(uniffi::Object)]
pub struct Seoyu {
    store: Arc<Mutex<Store>>,
    wiki_worker: Mutex<Option<WorkerHandle>>,
}
```

Update the constructor:

```rust
    #[uniffi::constructor]
    pub fn new(db_path: String) -> Result<Arc<Self>, SeoyuError> {
        let path = std::path::PathBuf::from(db_path);
        let store = Store::open(&path)?;
        Ok(Arc::new(Seoyu {
            store: Arc::new(Mutex::new(store)),
            wiki_worker: Mutex::new(None),
        }))
    }
```

Inside the `#[uniffi::export] impl Seoyu { ... }` block, append:

```rust
    /// Spawn the wiki classification worker if it is not already
    /// running. Idempotent. Errors only if the OS refuses a new
    /// thread.
    pub fn start_wiki_worker(&self) -> Result<(), SeoyuError> {
        let mut guard = self
            .wiki_worker
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if guard.is_some() {
            return Ok(());
        }
        let handle = start_worker(Arc::clone(&self.store), Arc::new(LogEmitter))
            .map_err(|e| SeoyuError::Other(format!("spawn wiki worker: {e}")))?;
        *guard = Some(handle);
        Ok(())
    }

    /// Signal shutdown and block until the worker thread exits.
    /// No-op if the worker was never started.
    pub fn stop_wiki_worker(&self) {
        let handle = {
            let mut guard = self
                .wiki_worker
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            guard.take()
        };
        if let Some(h) = handle {
            h.stop();
            h.join();
        }
    }
```

Add a non-UniFFI `Drop` impl at the bottom of the file:

```rust
impl Drop for Seoyu {
    fn drop(&mut self) {
        self.stop_wiki_worker();
    }
}
```

Verify:

```bash
cd sidecar && cargo build && cargo clippy -- -D warnings && cargo fmt --check
```

All three must be clean.

Commit: `feat(ffi): expose start_wiki_worker / stop_wiki_worker`

---

## Task 6 — Start worker from Swift shell

**File:** `Telegram-Mac/Seoyu/SeoyuBridge.swift`

Change `attach(postbox:)`. Existing body installs the observer; append a
worker start and log failures.

```swift
    public func attach(postbox: Postbox) {
        guard let seoyu else { return }
        self.ingestDisposable?.dispose()
        let observer = SeoyuIngestObserver(seoyu: seoyu)
        self.ingestDisposable = postbox.installGlobalStoreOrUpdateMessageAction(action: observer)

        do {
            try seoyu.startWikiWorker()
            NSLog("[seoyu] wiki worker started")
        } catch {
            NSLog("[seoyu] wiki worker start failed: %@", String(describing: error))
        }
    }
```

Add a matching `deinit`:

```swift
    deinit {
        self.ingestDisposable?.dispose()
        self.seoyu?.stopWikiWorker()
    }
```

Verify:

```bash
cd /Users/sskys/Mine/telegram-korean-search
./scripts/build-seoyu-xcframework.sh
xcodebuild build \
  -workspace Telegram-Mac.xcworkspace \
  -scheme Telegram \
  -configuration Debug \
  -destination 'generic/platform=macOS' \
  ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO \
  LD=$(pwd)/scripts/ld-cryptex-shim.sh \
  LDPLUSPLUS=$(pwd)/scripts/ld-cryptex-shim.sh
```

Expect: `** BUILD SUCCEEDED **`.

Commit: `feat(shell): autostart wiki worker after ingest observer`

---

## Task 7 — Smoke test against a live account

No code change; procedural verification.

1. Launch Telegram.app, log in.
2. Wait 60 s after login for observer to fire (handoff records 14k rows
   within minutes).
3. From another terminal:

   ```bash
   sqlite3 ~/Library/Application\ Support/telegram-korean-search/tg-korean-search.db \
     "SELECT status, COUNT(*) FROM wiki_classify_queue GROUP BY status;"
   ```

   Expect: `pending` row > 0 (observer enqueues), and either `done`
   or `failed` if `codex` CLI is present. No `processing` rows older
   than 5 min (worker recovers stale claims).

4. Tail the log:

   ```bash
   log stream --predicate 'process == "Telegram"' --info 2>&1 | grep -E "wiki|seoyu"
   ```

   Expect periodic `wiki progress:` lines.

5. Kill Telegram.app cleanly; verify no `processing` rows are stranded
   (observer's shutdown path calls `stop_wiki_worker`).

If `codex` CLI is missing on the machine, `failed` rows with
"codex: command not found" in their `last_error` are acceptable for
this plan — user-surfaced handling comes in the wiki-panel plan.

Commit: N/A (verification only).

---

## Task 8 — Update handoff

**File:** `docs/handoff.md`

Under `### Concrete next steps`, replace step 1 with a one-liner marking
it shipped and point the next session at step 2:

```markdown
1. **Worker loop — done (2026-04-23)**. Worker autostarts from
   `SeoyuBridge.attach` with a `LogEmitter`. Progress plumbing to Swift
   is deferred to a follow-up plan; logs via `log::info!` only.
```

Renumber the remaining three steps (panel UI, backfill, chat-scoped
search) accordingly.

Commit: `docs: handoff reflects wiki worker autostart landed`

---

## Final verification gate

Before declaring done:

```bash
cd sidecar
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

All three green. Then the Xcode build from Task 6 must still pass.
Then the Task 7 live-account smoke test must show queue draining.

## Self-review

- Spec coverage: every spec bullet maps to a task (1→T5, 2→T5+T6, 3→T3,
  4→T5 Drop + T6 deinit, 5→T6).
- No placeholders: every code block is literal, commands are explicit.
- Type consistency: `WorkerHandle`, `LogEmitter`, `start_worker` names
  match across T1/T2/T5.

## Execution Handoff

**Plan complete. Three execution options:**

1. **Subagent-Driven (recommended)** → sspower:subagent-driven-development
2. **Inline Execution** → sspower:executing-plans
3. **Codex execute** → delegate via `codex-bridge.mjs implement --write`
   or `codex-bridge.mjs rescue --write`

**Which approach?**
