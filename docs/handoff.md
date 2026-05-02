# Session Handoff
> Generated: 2026-05-02 23:00

## Task
Phase 8 (trending refresh) shipped + pushed to origin/main after 1
codex auto-review correctness pass. Next: phase 9 (digest) per spec
§6.5 OR phase 8 follow-ups (sustained reason_code, pinned_active
surface), or remaining phase 6/7 follow-ups.

## Status
### Completed (9 commits, branch `main` in sync with `origin/main`)
- `9c305d7c1` `feat(sidecar): wiki_rewrite_queue ops`
  → `sidecar/src/store/wiki_queue.rs`: `RewriteQueueItem`,
  `enqueue_rewrite` (lease-preserving upsert), `claim_rewrite_batch`,
  `mark_rewrite_done`, `mark_rewrite_retry`,
  `recover_stale_rewrite_claims`, `get_rewrite_stats`
- `fb9e44670` `feat(sidecar): rewrite evidence selection + apply_rewrite_v2`
  → `sidecar/src/store/wiki_page.rs`: `EvidenceForRewrite`,
  `PageForRewrite`, `RewriteApply`, `maybe_enqueue_rewrite`,
  `get_page_for_rewrite`, `select_rewrite_evidence`, `apply_rewrite_v2`
- `9960ad0a3` `feat(sidecar): v2 rewrite prompt + per-kind facts validator`
  → `sidecar/src/wiki/llm.rs`: `V2RewriteInput/Output`, `rewrite_page`,
  `validate_v2_rewrite`
- `e18fe7c7f` `feat(sidecar): rewrite worker loop + classify trigger`
  → `sidecar/src/wiki/worker.rs`: `process_rewrite_one`,
  `maybe_enqueue_rewrite` wired inside `apply_classify_v2` txn
- `5f57e6b14` `fix: pass 1` — re-enqueue-during-processing not lost,
  select errors propagate (no silent done), `rewrite_per_hour_cap`
  honored via in-memory sliding window
- `2363f43cd` `fix: pass 2` — delta uses `created_at` not `ts`,
  same-second clock fix attempt #1, validator strict for
  entity (canonical_name/relations/last_seen required)
- `81c93ba72` `fix: pass 3` — `last_rewrite_at` carries select-time
  snapshot (not apply now()), int validators use `as_i64()` (rejects
  floats), `apply_classify_v2` propagates `maybe_enqueue_rewrite`
  errors via `?`
- `9badd8f25` `fix: pass 4` — schema v9 idempotent extension adds
  `wiki_pages_v2.last_rewrite_max_evidence_id`. Rewrite delta now
  keyed on monotonic `id` (no more same-second loss). Top-K uses
  `id NOT IN (selected)` so delta-overflow rows surface.
- `e52afc5ef` `fix: pass 5` — retention sweep bounded by
  `id <= max_evidence_id`; post-snapshot inserts can't be deleted.

### Phase 8 commits (4 commits, in sync with origin/main)
- `6f1a3c1aa` `feat(sidecar): trending store layer + reason_code (phase 8)`
  → `sidecar/src/store/wiki_page.rs`: `TrendingWindow`, `TrendingSnapshot`,
  `TrendingCandidate`, `TrendingApplyRow`, `current_max_evidence_id`,
  `read_trending_watermark`, `shortlist_trending` (Rust scoring,
  bounded by snapshot id), `trending_sample_excerpts`,
  `compute_sparkline` (24 buckets), `apply_trending` (atomic
  DELETE+INSERT cache + UPSERT MAX-monotonic watermark),
  `derive_reason_code`. 14 new tests.
- `929559d48` `feat(sidecar): trending rerank + validator (phase 8)`
  → `sidecar/src/wiki/llm.rs`: `V2TrendingInput/Output`,
  `V2RankedItem`, `rerank_trending`, `validate_trending`. 11 tests.
- `7f1899cba` `feat(sidecar): trending refresh in worker (phase 8)`
  → `sidecar/src/wiki/worker.rs`: `pick_dirty_window` (most-overdue
  + gap-eligible, snapshots once), `maybe_refresh_trending` wired
  into idle path (post-rewrite) and classify-batch tail. 4 tests.
- `4f34c0e89` `fix(sidecar): trending correctness pass (codex review)`
  → stale-snapshot guard in `apply_trending` (returns `Ok(false)`
  when a newer concurrent tick already advanced the watermark; also
  flips return type to `Result<bool>`); validator now rejects empty
  `ranked` so worker falls back to shortlist instead of writing an
  empty cache; `last_computed_at` also MAX-monotonic. 2 new tests.

### Verification
- `cd sidecar && cargo test`: **203 passed, 1 ignored**
  (was 171; +32 phase 8)
- `cargo clippy --all-targets -- -D warnings`: clean
- `cargo fmt --check`: clean
- All 13 commits compile clean individually.

### In Progress
- None. Phase 8 in sync with origin/main.

## Resume Here
1. **Phase 9 — digest** per spec §6.5 (`docs/specs/2026-04-24-...md`).
   Pure SQL, no LLM. Per-chat group-by since `wiki_last_open[chat_id]`,
   filter `state != 'hidden' AND state != 'resolved'`,
   `HAVING n >= 3`. `wiki_last_open` table already in v9 schema.
   Add `wiki_mark_read(chat_id)` upsert. Optional codex digest summary
   (1/chat/day cap) deferred until UI lands. Ship.

2. **Phase 8 follow-ups** (defer until phase 9 lands):
   - `sustained` reason_code: needs persistent rolling-median
     across ≥3 prior refreshes; not in v9 schema. Add when phase 9
     digest also needs cross-tick state.
   - `pinned_active` reason_code: spec puts pinned pages in a
     separate UI slot above ranked top-10. Shortlist already filters
     `pinned = 0`; surface pinned in a separate read fn when the
     Swift trending panel lands.
   - Codex per-hour cap (`max_codex_calls_per_hour_total=500` seeded
     in v9): still **unenforced**. Trending adds ~24 calls/hr at
     full throttle (1h+24h × 12/hr). Phase 7 rewrite hour-cap is
     local; a global codex budget gate is a separate phase.

3. **Soft-delete read filters** (still deferred from prior session):
   add `deleted_at IS NULL` to all 6 read paths in
   `sidecar/src/store/message.rs` + `sidecar/src/search/engine.rs`
   FTS join. Spec line 160. Independent ship.

4. **Sender display name** (phase-6 leftover, still cosmetic):
   `V2InputMessage.sender` in `sidecar/src/wiki/worker.rs` is
   still empty. No users table → either build sender lookup or
   accept "anonymous" in classify prompts.

## Decisions (do NOT revisit)
- **Trending score in Rust, not SQL**. SQLite `LN` requires
  `SQLITE_ENABLE_MATH_FUNCTIONS` compile flag (not portable);
  `LEAST` is non-standard (use multi-arg `MIN`/`MAX`). SQL emits
  aggregates only; Rust computes the final score per spec §6.4.
- **`TrendingSnapshot` mirrors the rewrite phase fix**. `max_evidence_id`
  is captured once at the top of a refresh tick and threaded
  through shortlist + sparkline + sample_excerpts + apply, so any
  evidence inserted after the snapshot has `id > max_evidence_id`,
  surfaces in the next dirty-check, and is never silently consumed.
- **Trending fallback bumps watermark**. On rerank failure or
  validator miss, write top-10 shortlist with `hook=""` AND advance
  the watermark. Otherwise repeated bad LLM output hot-loops the
  same dirty window every tick (spec §6.4 line 849).
- **Most-overdue dirty window wins per tick**. When multiple windows
  are eligible, pick `argmax(now - last_computed_at - min_gap)`.
  Never-computed (`last_ts == 0`) wins over any computed window.
  One refresh per tick keeps codex calls bounded.
- **v9 deviation: `_v2` suffix kept** for `wiki_pages_v2`,
  `wiki_classify_queue_v2`. Spec wants unsuffixed. Phase 13
  `drop_v1_wiki` does the rename.
- **Rewrite delta watermark = monotonic `wiki_evidence.id`**, not
  `created_at` or `ts`. Stored in
  `wiki_pages_v2.last_rewrite_max_evidence_id` (added v9
  idempotently). Same race fix spec §6.4 used for trending.
- **`last_rewrite_at` = select-time snapshot**, not apply-time
  `now()`. Drives the 24h trigger fallback only; delta selection
  uses the id watermark.
- **Post-sweep `last_rewrite_evidence_count`** (LREC anchored to
  count after retention prunes). Pre-sweep LREC would deadlock the
  trigger after retention takes a page from 200 → 50 rows.
- **Retention sweep bounded by `id <= max_evidence_id`**. Inserts
  that arrive between select snapshot and apply (e.g. classify
  during the LLM round-trip) are NOT eligible for deletion — they
  go to the next rewrite cycle.
- **Top-K reads from `id NOT IN (selected)`**, not just older rows.
  Delta-overflow rows on hot pages used to vanish; spec §6.3
  "remainder" = "anything not yet selected".
- **Rewrite enqueue preserves processing lease**, and
  `mark_rewrite_done` re-arms `pending` if `enqueued_at >
  claimed_at` (re-enqueue arrived during processing). Upsert forces
  strict-greater enqueued_at to survive same-second clock.
- **`rewrite_per_hour_cap` (default 30)** enforced by in-memory
  sliding window in worker (counts every claim, including retries).
- **Facts validator stricter than spec text**: int fields require
  `as_i64()` (rejects `1.5`); entity requires non-empty
  canonical_name + relations[].{name,type} + last_seen int;
  event.started_at defaults null when missing (legitimately
  unknown). Frozen/hidden states forbidden as outputs.

## Gotchas
- Schema v9 is **extensible in place**: v9 migration uses
  `column_exists` + ALTER TABLE guards (see
  `migrate_to_v9` in `sidecar/src/store/schema.rs:84`). Adding
  `last_rewrite_max_evidence_id` followed that pattern. No
  schema_version bump needed for v9 additions.
- `enqueue_rewrite` SQL upsert is the safety net for the re-enqueue
  race: must keep `MAX(?, prev_enqueued_at + 1, claimed_at + 1)` —
  do NOT simplify to `enqueued_at = ?`.
- Worker holds the rewrite-hour-cap window in-memory only (resets
  on restart). Acceptable for now; a persistent counter would need
  yet another schema column.
- Codex auto-review runs on every push and is strict — expect 2-5
  iterations on multi-file features. Each round catches genuine
  race/correctness issues. Worth the time. Bypass with
  `SSPOWER_AUTO_REVIEW=off` only for emergencies (not used here).
- `submodules/telegram-ios` shows ahead but no push perms — leave
  alone. `Telegram-Mac/Info.plist`, `TelegramShare/Info.plist`,
  `submodules/tgcalls` pre-existing dirty — don't touch.

## Context
- **Branch**: `main` @ `4f34c0e89` (in sync with `origin/main`).
- **Schema**: v9, no new tables added (trending_cache +
  trending_watermark seeded in v9 already). No migration ran for
  phase 8.
- **Specs**:
  - `docs/specs/2026-04-24-reindex-and-wiki-v2-design.md` (phases 5–14)
  - `docs/specs/2026-04-27-cloud-wiki-architecture.md` (cloud worker)
- **Tests**: 203 passed, 1 ignored (lib 198, integration 5).
  clippy + fmt clean.

<!-- contract:start -->
done: phase 9 digest shipped per spec §6.5. wiki_last_open + wiki_mark_read writes; per-chat group-by since last_open_at filtered to active state, HAVING n >= 3. cargo test + clippy -D warnings green.
files:
  - sidecar/src/store/wiki_page.rs
  - sidecar/src/store/schema.rs
budget: 50 turns
forbidden:
  - submodules/**
  - Telegram-Mac/Info.plist
  - TelegramShare/Info.plist
  - docs/specs/**
<!-- contract:end -->
