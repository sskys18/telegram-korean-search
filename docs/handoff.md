# Session Handoff
> Generated: 2026-04-24 22:20

## Task
Rewrite indexing architecture end-to-end: single FTS5 table (3 cols), bm25 +
recency ranking, proper insert/update/delete semantics, accurate change count,
new `delete_messages` UniFFI surface, schema migration v8. NOT a cosmetic patch.

## Status

### Completed (this session, already on `main`)
- Wiki panel UI landed: titlebar toggle, window expand/collapse, list with
  trending topics, digest card, article view with back button. Commits up
  through `0f6b25c22` (`fix(wiki): ...`, `feat(wiki): ...`).
- Categories removed from UI per user request — trending-only list.
- Worker auto-wakes on launch via `seoyu.wikiRunPendingNow()` in
  `Telegram-Mac/Seoyu/SeoyuBridge.swift:59`.
- Analysis of current indexing architecture done (see "Decisions" below).

### In Progress / Not Started
- Indexing architecture rewrite — **not started**. This is the deliverable
  for the fresh session.

## Resume Here

Execute in order. Each step is a commit boundary; run `cargo fmt --check`,
`cargo clippy -- -D warnings`, `cargo test` between phases.

1. **Phase 1 — Schema migration v8** (`sidecar/src/store/schema.rs`):
   Add `migrate_v8_unified_fts`. Drops `messages_fts`, `messages_fts_nospace`,
   `messages_fts_jamo`. Creates ONE external-content FTS5 table:
   ```sql
   CREATE VIRTUAL TABLE messages_fts USING fts5(
       text_plain, text_stripped, text_jamo,
       content='messages', content_rowid='rowid',
       tokenize='trigram case_sensitive 0'
   );
   ```
   Backfill via `INSERT INTO messages_fts(rowid, text_plain, text_stripped,
   text_jamo) SELECT rowid, text_plain, text_stripped, text_jamo FROM
   messages;` in batches of 5000.

2. **Phase 2 — Ingest semantics** (`sidecar/src/store/message.rs:53`):
   Replace `INSERT OR IGNORE` with explicit upsert. On text change, also
   UPDATE the FTS row (`DELETE FROM messages_fts WHERE rowid = ?` then
   `INSERT`). Cache prepared stmts outside the loop. Return `(inserted,
   updated)` counts from `insert_messages_batch`. Kill the `SELECT changes()`
   + `SELECT last_insert_rowid()` per-row pattern — use `INSERT ... ON
   CONFLICT ... RETURNING rowid` (SQLite ≥3.35).

3. **Phase 3 — Delete surface** (`sidecar/src/uniffi_api.rs`):
   Add `pub fn delete_messages(&self, refs: Vec<MessageRef>) -> Result<u64,
   SeoyuError>` where `MessageRef { chat_id, message_id }`. Implement in
   `Store::delete_messages_batch` — deletes from `messages`; FTS rows go
   with it because external-content. Wire into Swift
   `SeoyuIngestObserver.swift` (add a `deleted:` callback; Postbox has a
   removal signal).

4. **Phase 4 — Accurate return** (`sidecar/src/uniffi_api.rs:208-227`):
   `index_messages` returns `{ inserted, updated }` instead of input count.

5. **Phase 5 — Ranking** (`sidecar/src/store/message.rs:179`,
   `sidecar/src/search/engine.rs`): Replace 3-table UNION with single-table
   bm25:
   ```sql
   SELECT ... FROM messages_fts JOIN messages ...
   WHERE messages_fts MATCH ?
   ORDER BY bm25(messages_fts, 1.0, 0.7, 0.5)
          + (strftime('%s','now') - m.timestamp) / 86400.0 * 0.05
   ```
   Keep cursor pagination shape `(timestamp, chat_id, message_id)`.

6. **Phase 6 — LIKE fallback coverage** (`sidecar/src/store/message.rs:393`):
   Extend short-query LIKE to also match `text_stripped` and `text_jamo`
   columns. Currently only `text_plain`.

7. **Phase 7 — Tests**. Add to `sidecar/src/search/engine.rs` test module
   and `sidecar/tests/uniffi_surface.rs`:
   - update-reindex-round-trip (edit text, search finds new)
   - delete removes from search
   - bm25 beats pure recency for strong match
   - LIKE fallback hits jamo col for 2-char Korean query
   - index_messages returns correct inserted/updated split

8. **Phase 8 — Docs**. Update `CLAUDE.md:143-149` and `README.md:57-60` —
   replace "three columns" / "bm25" drift with actual new architecture.

## Decisions (locked — do not re-debate)

- **Single FTS5 table with 3 columns, external-content**. Not 3 separate
  tables (current). Reason: 1/3 write amplification, enables bm25 with
  per-column weights, cleaner update/delete via external-content.
- **bm25 + linear recency decay**, not `priority, timestamp DESC`. Recency
  coefficient `0.05/day` — tune later via eval.
- **Migration v8 is one-shot**. Drop old FTS tables, rebuild. 51k existing
  messages → backfill completes in <10s at trigram tokenization speed.
- **No Lindera, no unicode-segmentation** — Hangul decomposition stays
  codepoint-math in `sidecar/src/search/hangul.rs`.
- **No chosung revival** — `migrate_drop_chosung` (v7) is permanent.
- **UniFFI-only surface for writes**. IPC path stays dormant; don't wire
  `delete` via IPC.

## Gotchas

- `SeoyuIngestObserver.addOrUpdate` is called for BOTH inserts and edits.
  Current `INSERT OR IGNORE` silently drops edits. The rewrite must detect
  text change and reindex.
- External-content FTS5 deletes propagate when you delete from base table.
  Inserts DO NOT — must manually `INSERT INTO messages_fts(rowid, ...)`.
  Currently done in `fts_insert` at `message.rs:5-19`.
- Postbox delete signal: find the right observer API in
  `submodules/telegram-ios/submodules/Postbox/Sources/Postbox.swift` — the
  fork already added `installGlobalStoreOrUpdateMessageAction` (see
  `4083-4088`); deletion probably needs another hook or a different
  callback.
- SQLCipher build must include FTS5 trigram. See
  `docs/SQLCIPHER-TRIGRAM-BLOCKER.md`. If `no such tokenizer: trigram`
  appears, the patched sqlcipher didn't land.
- `cargo test` must stay green at every commit — including the 220k queue
  items in user's dev DB (migration must not time out).
- Swift build: `./scripts/build-dev.sh`. Launch via
  `~/Library/Developer/Xcode/DerivedData/Telegram-Mac-dev/Build/Products/Debug/Telegram.app/Contents/MacOS/Telegram`.

## Context
- **Branch**: `main` @ `0f6b25c22` (pushed).
- **Tests**: sidecar 107 passed / 1 ignored at last run. Swift builds green.
- **User DB**: 51k messages, 220k wiki queue items, 116k classified topics.
  Do not lose this during migration testing — copy the DB before running
  v8 locally.
- **Prior analysis**: full weak-spot list + priority order given in-session
  before this handoff (bm25 beats priority ranking, 3x write
  amplification, LIKE fallback misses jamo, `changes()` per-row is slow,
  stale docs). Refer to conversation transcript.
