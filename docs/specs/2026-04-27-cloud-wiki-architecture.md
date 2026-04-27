# Cloud Wiki Worker Architecture (Draft)

> Status: **DRAFT** — design only, no code committed
> Author: Claude (handoff-resume session)
> Date: 2026-04-27
> Revision: fourth pass; reviewed by codex three times. Each pass surfaced gaps; this revision closes the v3 set of nine.
> Supersedes earlier drafts of this same file
> Supersedes parts of: `docs/specs/2026-04-24-reindex-and-wiki-v2-design.md` (non-goals only)
> Related: `docs/handoff.md`, `docs/plans/2026-04-25-plan-0-swift-delete-observer.md`

## Revision history

- **First pass**: cloud-primary indexing + search. Codex tore it down: Mac owns MTProto, cloud cannot ingest while Mac asleep; cloud-primary search adds Tailscale latency + breaks Postbox rehydration; factual errors about existing schema/security.
- **Second pass**: hybrid (local FTS + cloud wiki only), `backend=local` default, real `msg_version` column, sqlcipher dropped from cloud v1. Codex caught 11 correctness issues.
- **Third pass**: addressed all 11 — two-txn outbox honesty, msg_version on every observable mutation, NDJSON pull, in-txn cursor, 11 explicit event types, `client_op_id` ACK, `chat_purge`, codex hardening promoted to required step.
- **This pass (fourth)**: addresses the nine spec-tightening gaps codex caught in pass three: cloud schema names matched to v9 verbatim, opt-out purge made privacy-actually-safe, per-chat reconciliation watermarks, tombstone sweep gated on cloud ACK, outbox schema/op-list reconciled w/ payloads, pull-log gap detection + snapshot rebackfill, full event-type table covering page aliases / FTS / trending watermark / ask lifecycle / page state / evidence id, `protocol_version` in every sample payload, `sender_id` NOT NULL on cloud + wired through UniFFI.

## Goal

Move only the wiki LLM pipeline (classify, rewrite, trending, Ask) to a remote always-on worker so that:

1. The 220k-item classify backlog drains while Mac is asleep.
2. Wiki artifacts stay fresh between Mac sessions.
3. Local search (FTS5) is unchanged — fast, offline, no network dependency.
4. Mac stays the only ingress for new messages (MTProto via TelegramSwift).

Explicit non-goal: cloud-primary search.

## Infrastructure

`~/Mine/oracle-cloud/`:

- Oracle Free Tier ARM box: `152.67.210.188` / Tailscale `100.91.111.53`
- 4 OCPU Ampere Altra, 24 GB RAM, 50 GB disk, Ubuntu 22.04 aarch64
- Codex auth installed; `codex exec` works server-side
- Tailscale tunnel up; OCI security-list ingress = SSH(22) + ICMP only
- Prometheus / Alertmanager / node_exporter on `127.0.0.1`

**Oracle Always Free reclamation**: 7-day CPU + net + memory < 20% triggers reclaim. Decision: **paid tier (~$5/mo) is the documented fallback**, not a CPU-burn cron. Promotion deferred until first reclaim event or sustained-idle alert.

## Errors corrected from earlier drafts

| Earlier claim | Reality | Source |
|---|---|---|
| `edit_version` exists at `message.rs:206` | No such column | `sidecar/src/store/schema.rs:17` |
| Mac DB is sqlcipher-encrypted via Keychain | Plain SQLite; Keychain only encrypts `session.bin` | `sidecar/src/store/mod.rs:20` |
| aarch64 cross-compile is small | `security-framework` unconditional dep; SQLCipher on Linux needs OpenSSL provider | `sidecar/Cargo.toml:38` |
| Cloud schema = "same migrations + account_id" | v9 is greenfield rewrite | `sidecar/src/store/schema.rs:337` |
| `wiki_settings` is singleton row | v9 uses `(key, value)` shape | `docs/specs/2026-04-24-reindex-and-wiki-v2-design.md:292` |
| Outbox enqueue is in Postbox txn | Postbox + sidecar are separate transactions | `submodules/telegram-ios/.../Postbox.swift:2113` |
| `msg_version` bumps only on text change | Must bump on every cloud-observable mutation | (this revision) |
| Pull stream "each chunk = JSON" | HTTP chunks ≠ JSON message boundaries; uses NDJSON | (this revision) |
| Spike uses `rusqlite` | Repo uses `sqlite = "0.37"` | `sidecar/Cargo.toml:31` |
| Search RPC "no UI change" | `SearchController.swift:1254,1272` rehydrates from local Postbox | source |
| Cloud schema names invented (`wiki_topics`, `wiki_categories`, `wiki_category_aliases`, `wiki_trending_cache`) | v9 actually defines `wiki_pages`, `wiki_page_aliases`, `trending_cache`, `trending_watermark`, `wiki_last_open`, `wiki_pages_index_fts`, `wiki_evidence_fts` | `docs/specs/2026-04-24-reindex-and-wiki-v2-design.md:168,196,261,275,297,330` |
| `sender_id` nullable on cloud | v9 `wiki_evidence.sender_id INTEGER NOT NULL` | `docs/specs/2026-04-24-reindex-and-wiki-v2-design.md:209` |
| Opt-out purge = privacy control | Pending outbox could resurrect, change_log retained text, ask_history not purged | (this revision; see Opt-out section) |

## Architecture

```
Mac (Telegram.app)               Tailscale          Oracle box
┌─────────────────────┐               │              ┌──────────────────────────────┐
│ TelegramSwift fork  │               │              │ tg-seoyu-worker (axum)       │
│  Postbox (MTProto)  │               │              │   ↓                          │
│   ↓                 │               │              │ SQLite (FTS5+JSON1)          │
│ SeoyuBridge         │               │              │   ├─ messages (full corpus)  │
│   ↓                 │               │              │   ├─ chats                   │
│ local SQLite        │── push ───────┼─ POST /v1/   │   ├─ wiki_pages              │
│  ├─ messages (full) │   (NDJSON,    │   wiki/jobs  │   ├─ wiki_page_aliases       │
│  ├─ messages_fts    │    durable    │              │   ├─ wiki_evidence           │
│  ├─ wiki_* (cache)  │    outbox)    │              │   ├─ wiki_classify_queue     │
│  └─ cloud_outbox    │               │              │   ├─ wiki_rewrite_queue      │
│                     │◄── pull ──────┼─ GET  /v1/   │   ├─ trending_cache          │
│  Search = local FTS │   (long-poll  │   changes    │   ├─ trending_watermark      │
│  Wiki UI = local    │    NDJSON,    │              │   ├─ wiki_pages_index + FTS  │
│   cache (read-only) │    gap-aware) │              │   ├─ wiki_evidence_fts       │
└─────────────────────┘               │              │   ├─ ask_history             │
                                      │              │   ├─ wiki_settings           │
                                      │              │   ├─ wiki_last_open          │
                                      │              │   └─ cloud_change_log        │
                                      │              │   ↓                          │
                                      │              │ wiki worker (codex 24h)      │
                                      │              └──────────────────────────────┘
```

Cloud schema = full v9 wiki tables verbatim (see `docs/specs/2026-04-24-reindex-and-wiki-v2-design.md:168-300`) plus three new tables defined here: `messages`, `chats`, `cloud_change_log`. No renames, no inventions.

## Flag model

`wiki_settings(key, value)`:

```sql
INSERT INTO wiki_settings(key, value) VALUES
  ('backend',          'cloud'),     -- user direction: 'all to remote'; local kept as fallback only
  ('cloud_endpoint',   ''),
  ('cloud_chat_optout','[]'),
  ('last_pull_seq',    '0'),
  ('cloud_min_seq_seen', '0'),     -- for gap detection (see Pull section)
  ('pause',            '0'),
  ('protocol_version', '1');
```

| Mode | Search | Wiki | Notes |
|---|---|---|---|
| `cloud` (default) | local FTS | cloud worker, 24h backlog | Per-chat opt-out applies; user direction "all to remote" |
| `local` | local FTS | local worker, app-open only | Fallback when cloud unreachable / opt-out / privacy mode |
| `off` | local FTS | disabled | Search-only |

Note: search stays local in all modes. Codex's physics objection stands — `SearchController.swift:1254,1272` rehydrates results from local Postbox, so cloud-only search would break the UI. Moving search remote requires a separate workstream not covered here.

## Schema

### Local additions (one v9 migration; lands independent of cloud)

```sql
ALTER TABLE messages ADD COLUMN msg_version  INTEGER NOT NULL DEFAULT 1;
ALTER TABLE messages ADD COLUMN deleted_at   INTEGER;            -- NULL = live
ALTER TABLE messages ADD COLUMN cloud_acked_version INTEGER;     -- last msg_version cloud confirmed; NULL = never
ALTER TABLE messages ADD COLUMN sender_id    INTEGER;            -- 0 = unknown (backfill default)

CREATE TABLE cloud_outbox (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    client_op_id  TEXT    NOT NULL UNIQUE,
    op            TEXT    NOT NULL CHECK (op IN
                    ('msg_upsert','msg_delete','chat_meta','chat_purge')),
    chat_id       INTEGER NOT NULL,
    message_id    INTEGER,                    -- NULL for chat_meta + chat_purge
    msg_version   INTEGER,                    -- NULL for chat_meta + chat_purge
    payload       BLOB    NOT NULL,           -- bincode
    created_at    INTEGER NOT NULL,
    attempts      INTEGER NOT NULL DEFAULT 0,
    last_error    TEXT
);
CREATE INDEX ix_outbox_chat ON cloud_outbox(chat_id);

CREATE TABLE IF NOT EXISTS wiki_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);

-- Per-chat reconciliation watermark (replaces v3's broken global timestamp scan)
CREATE TABLE postbox_recon_watermark (
    chat_id        INTEGER PRIMARY KEY,
    max_msg_id     INTEGER NOT NULL,           -- highest message_id sidecar has seen
    last_full_diff INTEGER NOT NULL DEFAULT 0  -- unix ts of last full diff for this chat
);
```

`msg_version` increments on **every cloud-observable mutation**:

- text change at `sidecar/src/store/message.rs:206`
- link change (already in same diff branch)
- timestamp change (now bumps; previously silently overwritten)
- delete at `sidecar/src/store/message.rs:255` (writes tombstone w/ `msg_version = old + 1`, `deleted_at = now`)

`cloud_acked_version` is set when server returns `applied` or `stale` for that message's `msg_version`. Tombstone sweep checks this before dropping rows (see Tombstone retention).

`sender_id` added to local `messages` to feed cloud (v9 `wiki_evidence.sender_id NOT NULL`). Backfill default `0` = unknown. UniFFI `IndexedMessage` gains `sender_id: i64` field (step 4 in cutover); observer at `SeoyuIngestObserver.swift:24` reads from `StoreMessage.author?.id.toInt64() ?? 0`.

### Soft-delete enforcement

Every read path filters `deleted_at IS NULL`:

- `sidecar/src/store/message.rs` search query
- `sidecar/src/search/engine.rs` FTS join
- `sidecar/src/wiki/worker.rs:303` classify dequeue
- Backfill enumeration

FTS5 row deletion in same sidecar txn as tombstone write.

### Tombstone retention (gated on cloud ACK)

Sweep job (daily, on bridge bootstrap):

```sql
DELETE FROM messages
 WHERE deleted_at IS NOT NULL
   AND deleted_at < (now - 30 days)
   AND (
         (SELECT value FROM wiki_settings WHERE key='backend') != 'cloud'
         OR cloud_acked_version >= msg_version
       );
```

If `backend=cloud` and uploader is stuck, tombstones for that chat stay until ACK arrives. No silent loss of deletes on cloud.

### Cloud (Oracle box) schema

`/var/lib/tg-seoyu/wiki.db`. Single tenant.

```sql
-- Mac-mirrored
CREATE TABLE messages (
    chat_id      INTEGER NOT NULL,
    message_id   INTEGER NOT NULL,
    msg_version  INTEGER NOT NULL,
    timestamp    INTEGER NOT NULL,
    sender_id    INTEGER NOT NULL,           -- 0 = unknown; satisfies v9 wiki_evidence FK shape
    text         TEXT    NOT NULL,
    link         TEXT,
    deleted_at   INTEGER,
    PRIMARY KEY (chat_id, message_id)
);

CREATE TABLE chats (
    chat_id     INTEGER PRIMARY KEY,
    title       TEXT NOT NULL,
    chat_type   TEXT NOT NULL,
    username    TEXT,
    access_hash INTEGER,
    updated_at  INTEGER NOT NULL
);

-- Verbatim from v9 design (see docs/specs/2026-04-24-reindex-and-wiki-v2-design.md):
--   wiki_pages, wiki_page_aliases, wiki_evidence, wiki_classify_queue,
--   wiki_rewrite_queue, trending_cache, trending_watermark,
--   ask_history, wiki_settings, wiki_last_open,
--   wiki_pages_index, wiki_pages_index_fts, wiki_evidence_fts

-- Durable pull source
CREATE TABLE cloud_change_log (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    type       TEXT NOT NULL,
    payload    BLOB NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX ix_change_log_seq ON cloud_change_log(seq);

-- For gap detection / snapshot rebackfill
CREATE TABLE cloud_change_log_meta (
    min_seq INTEGER NOT NULL,                 -- updated by retention sweep
    sweep_at INTEGER NOT NULL
);
```

## Sync protocol

### Push: Mac → cloud

`POST /v1/wiki/jobs` (NDJSON, one op per line):

```
{"protocol_version":1,"client_op_id":"01H…","op":"msg_upsert","chat_id":-1001,"message_id":42,"msg_version":3,"timestamp":1714210000,"sender_id":12345,"text":"...","link":null}
{"protocol_version":1,"client_op_id":"01H…","op":"chat_meta","chat_id":-1001,"title":"…","chat_type":"channel","username":"foo","access_hash":7777}
{"protocol_version":1,"client_op_id":"01H…","op":"msg_delete","chat_id":-1001,"message_id":99,"msg_version":2}
{"protocol_version":1,"client_op_id":"01H…","op":"chat_purge","chat_id":-1001}
```

Server response (NDJSON, one per request line):

```
{"client_op_id":"01H…","status":"applied","seq":17}
{"client_op_id":"01H…","status":"stale","current_version":4}
{"client_op_id":"01H…","status":"error","error":"…"}
```

`applied` and `stale` both → client deletes outbox row + writes `cloud_acked_version` on the message row. `error` → keep row, increment `attempts`.

`chat_meta` and `chat_purge` carry no `msg_version` (column nullable). Server treats `chat_meta` as last-write-wins on `updated_at`; `chat_purge` is idempotent (re-applying does nothing).

### Two-txn outbox semantics (Postbox + sidecar separate)

Observer at `SeoyuIngestObserver.swift:34` calls UniFFI synchronously inside Postbox's mutation, but Postbox + sidecar each commit independently. Cases:

| Case | What happens | Recovery |
|---|---|---|
| Both commit | Normal; outbox row exists, uploader sends | — |
| Postbox commits, sidecar fails | Postbox has msg, sidecar doesn't, no outbox row | Per-chat reconciliation diff |
| Sidecar commits, Postbox aborts | Sidecar has row + outbox for msg Postbox doesn't have | Per-chat reconciliation finds orphan, writes tombstone, propagates `msg_delete` |

**Reconciliation (replaces v3's broken global-timestamp scan)**:

Per-chat watermarks in `postbox_recon_watermark`. Triggered:

- on `SeoyuBridge.bootstrap()`
- on Postbox `didCommit` (debounced 30s per chat)
- on user-visible chat open
- nightly full diff for chats not touched in 7d

Per-chat diff: `SELECT message_id FROM postbox_msgs WHERE chat_id=? ORDER BY message_id` vs same on sidecar. Symmetric difference yields:

- Postbox-only ids → re-call `index_messages` for those rows.
- Sidecar-only ids → write tombstones, enqueue `msg_delete` to outbox.

Cost: O(N) per chat per diff; with index, ~ms for 10k-msg chat. Spike step 7 measures full-corpus diff cost.

Outbox enqueue happens **inside the sidecar txn** at `sidecar/src/store/message.rs:121` — same txn as the FTS write. Upload happens on a **separate tokio task**. Network never blocks the observer call.

### Pull: cloud → Mac

`GET /v1/wiki/changes?after=<last_applied_seq>` long-poll, NDJSON body. One JSON object per line, terminated by `\n`. Connection held up to 30s; closes when 50 events sent or timeout reached.

```
{"protocol_version":1,"seq":17,"type":"page_upsert","payload":{...}}
{"protocol_version":1,"seq":18,"type":"evidence_upsert","payload":{...}}
{"protocol_version":1,"seq":19,"type":"trending_watermark_update","payload":{"window":"24h","last_evidence_id":4012,"last_computed_at":1714210400}}
```

**Cursor advance in same SQLite txn as artifact apply**:

```sql
BEGIN;
  -- apply event payload to local wiki_* tables
  UPDATE wiki_settings SET value = '<seq>' WHERE key = 'last_pull_seq';
COMMIT;
```

### Gap detection + snapshot rebackfill

Every pull response also includes a header line `{"meta":"window","min_seq":<cloud_min_seq>,"max_seq":<cloud_max_seq>}`. Client checks:

```
if last_pull_seq < min_seq:
    # offline too long; cloud has swept events we never saw
    request snapshot rebackfill
else:
    apply normally
```

Snapshot rebackfill: `POST /v1/wiki/snapshot` returns a tar-streamed dump of all wiki tables at `seq=max_seq`; client truncates local wiki cache, applies snapshot in single txn, sets `last_pull_seq = max_seq`.

Cloud `cloud_change_log_meta.min_seq` updated by retention sweep; client's `cloud_min_seq_seen` tracks the last value the server reported, so gap detection works across reconnects.

### Event-type table (full set)

| Event | Payload | Notes |
|---|---|---|
| `page_upsert` | row from `wiki_pages` | full row, last-write-wins by `page.updated_at` |
| `page_delete` | `{page_id}` | cascade locally to evidence + aliases + trending + ask_history refs |
| `page_pin_change` | `{page_id, pinned}` | for pin/unpin without full upsert |
| `page_state_change` | `{page_id, state}` | for state machine (draft/active/hidden/etc.) |
| `page_alias_upsert` | row from `wiki_page_aliases` | |
| `page_alias_delete` | `{page_id, alias_norm}` | |
| `evidence_upsert` | row from `wiki_evidence` (incl. `id`) | |
| `evidence_remove` | `{evidence_id}` | keyed by `evidence_id`, not `(page_id, msg_id)` |
| `trending_recompute` | `{window, computed_at, rows: [trending_cache rows]}` | per `window` (1h/24h/7d), full replace; client truncates `WHERE window=?` first |
| `trending_watermark_update` | row from `trending_watermark` | per-window |
| `ask_history_append` | row from `ask_history` | |
| `ask_history_delete` | `{ask_id}` | for retention purges + opt-out cascade |
| `wiki_settings_change` | `{key, value}` | for cloud-driven settings (e.g. model_*) |
| `pages_fts_refresh` | `{page_ids: [...]}` | hint for client to drop FTS cache rows; cloud-side FTS owned by cloud, but Mac mirrors `wiki_pages_index` so needs invalidation |
| `evidence_fts_refresh` | `{evidence_ids: [...]}` | same |
| `chat_purge_complete` | `{chat_id}` | confirms server-side cascade done; client can mark opt-out as fully effective |

All payloads include `protocol_version: 1` envelope; per-event field shapes carry only what differs from the named v9 row.

### Search

Local FTS only. No `/v1/search` endpoint.

### Ask / digest / trending

Cloud-only when `backend = cloud`:

- `POST /v1/wiki/ask` (NDJSON, citations)
- `GET /v1/wiki/digest?date=YYYY-MM-DD`
- `GET /v1/wiki/trending`

When `backend = local`, run via local worker on-demand.

## Backfill

1. User flips `backend = local → cloud`.
2. Local enumerates `messages WHERE deleted_at IS NULL`, filters `cloud_chat_optout`, batches 1k rows in `(timestamp, chat_id, message_id)` cursor order. Each enqueue gets fresh `client_op_id`.
3. Background uploader drains at throttled rate (default 10 batches/min).
4. UI: two progress bars — `(uploaded / total)` from local outbox; `(classified / total)` from cloud `GET /v1/wiki/status`.
5. Cloud worker classifies as it receives. Pulls flow back via `cloud_change_log`.
6. Cutover complete when both queues drain.

Reverse cutover (`cloud → local`): cloud exposes `GET /v1/wiki/pending` returning un-classified `(chat_id, message_id, msg_version)` triples; local re-enqueues into local `wiki_classify_queue`.

## Per-chat opt-out (real privacy control)

Adding a chat to `cloud_chat_optout` triggers the full purge sequence:

1. Local writes `cloud_chat_optout` update.
2. Local **cancels pending outbox rows** for that chat:
   `DELETE FROM cloud_outbox WHERE chat_id = ?`
   (Done before `chat_purge` enqueue so no in-flight upload races.)
3. Local enqueues `chat_purge` op for that `chat_id`.
4. Server applies in a single txn:
   - `DELETE FROM messages WHERE chat_id = ?`
   - `DELETE FROM wiki_evidence WHERE chat_id = ?`
   - Cascade to `wiki_pages` (drop pages with no remaining evidence)
   - `DELETE FROM ask_history WHERE chat_id = ?` (or `query LIKE '%' || chat_title || '%'` if linkable)
   - `DELETE FROM trending_cache WHERE page_id IN (deleted page_ids)`
   - **Scrub `cloud_change_log` payloads referencing `chat_id`**: rewrite payload to `{redacted: chat_id}` (keep `seq`/`type` for cursor consistency, drop text)
5. Server emits cascade events: `evidence_remove`, `page_delete`, `ask_history_delete`, `trending_recompute`, then `chat_purge_complete`.
6. Local applies cascade, evicts wiki cache.
7. Local-side: `DELETE FROM messages WHERE chat_id = ?` (purge from Mac too, since the user opted out post-hoc) — confirmed via Settings UI w/ explicit warning.

This makes opt-out actually purge:

- pending uploads cancelled (1, 2)
- existing cloud data deleted (4)
- change_log payloads scrubbed (4)
- ask_history purged (4)
- local copies optionally purged (7)
- server logs (Codex, axum, systemd journal) — documented gap; mitigation = log-rotation policy filters chat_id, or accept residual w/ user disclosure

## Privacy / security

**README "nothing leaves the machine" claim must be updated before `cloud` mode ships.** Proposed: *"Local-by-default Korean search. Optional cloud wiki worker on user-owned Oracle Free Tier instance over Tailscale, opt-in via Settings."*

| Layer | Control |
|---|---|
| Transport | Tailscale wireguard tunnel; TLS terminates inside tailnet w/ self-signed cert pinned client-side. OCI security-list keeps SSH(22)+ICMP only. |
| Tailscale auth | Tagged server (`tag:tg-seoyu-cloud`); ACL grant Mac → cloud `tcp:4443`; node-key expiry monitored; reauth flow doc'd in `oracle-cloud/README.md`. |
| Endpoint storage | Tailscale MagicDNS hostname, not raw IP |
| At-rest (cloud, v1) | Filesystem perms (0700) + OS-level disk encryption only. No sqlcipher v1. |
| At-rest (cloud, v2) | sqlcipher w/ OpenSSL provider; deferred |
| At-rest (Mac) | Plain SQLite today; flagged as separate gap |
| Per-chat opt-out | Cancel-then-purge cascade (above) |
| Server log scrubbing | Rotate Codex stdout logs daily; redact chat_id from systemd journal via filter; document residual |
| Kill switch | `wiki_settings.pause = '1'` halts uploader + worker |

Threat model: Oracle box compromise = corpus exposure for non-opt-out chats. Acceptable for single-user, user-owned-VM setup; documented loudly.

## Codex subprocess hardening (real blocker)

Required before cloud worker runs unattended:

- Replace `/tmp/tg-wiki-codex-{pid}.txt` (`llm.rs:83`) w/ `tempfile::NamedTempFile` per call
- Configurable timeout per phase (classify: 30s, rewrite: 120s, ask: 60s)
- Concurrency limit via `tokio::Semaphore` (start 4)
- Circuit breaker: 3 consecutive failures → 5min backoff; alert via Prometheus
- Codex auth health probe at startup + every 1h; alert on 401
- Token-budget meter logged per call (not enforced v1)
- Crash-resume: stale `claimed_at` (>10min) reset to pending (already in `wiki_queue.rs:173-180`)

Same `sidecar/src/wiki/llm.rs` shared local + cloud.

## Schema + protocol coordination

`protocol_version` field in every push and pull payload (now consistent in samples above).

- Server returns HTTP 426 if `protocol_version > server's max`.
- Server accepts N-1 best-effort.
- Migration order: cloud first (gain support), Mac second (start sending). Reverse for removals.
- Downgrade not supported v1.
- Cloud schema reset path: stop service, `rm /var/lib/tg-seoyu/wiki.db`, restart, Mac re-uploads via backfill.

## Cutover plan

| # | Task | Files | Notes |
|---|---|---|---|
| 0 | **Land Plan 0** (Swift delete observer) | `submodules/telegram-ios/...`, `Telegram-Mac/Seoyu/SeoyuIngestObserver.swift` | Hard prereq |
| 1 | Schema v9 migration: `msg_version`, `deleted_at`, `cloud_acked_version`, `sender_id`, `wiki_settings(key,value)`, `cloud_outbox`, `postbox_recon_watermark` | `sidecar/src/store/schema.rs` | All in one migration |
| 2 | Bump `msg_version` on every observable mutation (text, link, timestamp, delete) | `sidecar/src/store/message.rs:206`, `:255` | Table-driven test |
| 3 | Add `deleted_at IS NULL` filter to all read paths | `sidecar/src/store/message.rs`, `sidecar/src/search/engine.rs`, `sidecar/src/wiki/worker.rs:303` | Audit grep before merge |
| 3a | Tombstone retention sweep gated on `cloud_acked_version >= msg_version` | `sidecar/src/store/message.rs` | Bridge bootstrap timer |
| 3b | Per-chat reconciliation diff (`postbox_recon_watermark`) | `Telegram-Mac/Seoyu/SeoyuBridge.swift`, `sidecar/src/store/message.rs` | Triggered on bootstrap, didCommit (debounced), chat-open, nightly |
| **SPIKE** | **Proof-of-risk** | `oracle-cloud/`, `scripts/build-cloud.sh` | Hard gate |
| 4 | UniFFI types: `IndexedMessage` adds `sender_id: i64` | `sidecar/src/uniffi_api.rs:64`; `Telegram-Mac/Seoyu/SeoyuIngestObserver.swift:24` reads `StoreMessage.author?.id.toInt64() ?? 0` | Field-additive |
| 5 | `WikiBackend` trait, gate worker on `backend` flag | `sidecar/src/wiki/worker.rs:147` | |
| 6 | Outbox enqueue inside sidecar txn + uploader tokio task | `sidecar/src/store/message.rs:193,235`, new `sidecar/src/cloud/outbox.rs` | Network out-of-txn |
| 7 | Codex subprocess hardening | `sidecar/src/wiki/llm.rs` | Required |
| 8 | Cloud server bin: axum + ingest + change_log writer in same txn as artifact apply + snapshot endpoint | `sidecar/src/bin/cloud_server.rs` | New binary target |
| 9 | aarch64 cross-compile + deploy + systemd unit | `scripts/build-cloud.sh`, `oracle-cloud/monitoring/install.sh` extension, new `oracle-cloud/systemd/tg-seoyu-worker.service` | Codex auth env, restart=always, tempfile dir, log-rotation |
| 10 | Sync client: push + long-poll pull w/ in-txn cursor + gap-detection + snapshot rebackfill | `sidecar/src/cloud/{client.rs, push.rs, pull.rs, snapshot.rs}` | |
| 11 | UniFFI: `set_backend`, `get_sync_status`, `pause_cloud`, `purge_chat` | `sidecar/src/uniffi_api.rs:220` | Before Settings UI |
| 12 | Settings UI: backend toggle, chat opt-out + purge confirm, sync status, pause | `Telegram-Mac/Seoyu/SeoyuCloudSettings.swift` (new) | Consumes step 11 |
| 13 | Protocol-version negotiation tests + event-replay tests | `sidecar/tests/cloud_protocol.rs`, `sidecar/tests/cloud_replay.rs` (new) | N-1 compat, HTTP 426 path, every event type applied to clean DB |
| 14 | README + handoff + CLAUDE.md updates | `README.md`, `docs/handoff.md`, `CLAUDE.md` | Required before merging cloud mode |

## Spike (mandatory before steps 4+)

Each is a few hours; total ~3 sessions.

1. **aarch64 Rust build.** `cross build --target aarch64-unknown-linux-gnu --release` against `sidecar/`. Resolve `security-framework` (cfg-gate to `target_os = "macos"`) and any other macOS-only deps. Acceptance: builds cleanly.
2. **SQLite + FTS5 + JSON1 + trigram on Linux aarch64.** Repo uses `sqlite = "0.37"` crate. Confirm bundled SQLite carries trigram tokenizer; document patch path if not. Acceptance: `cargo test --lib` passes on Oracle box.
3. **Codex on Oracle, exec smoke.** Single classify call e2e via `codex exec` from `systemd --user` as `ubuntu`. Test non-interactive auth, restart-on-crash, env propagation, tempfile dir, Prometheus scrape. Acceptance: valid JSON, p50 < 30s, restart leaves no orphan tempfile.
4. **Tailscale RTT measurement.** Mac → Oracle direct + DERP fallback. Acceptance: numbers in this doc; p95 captured for both paths.
5. **Push/pull crash harness.** Forced crash at every ACK / cursor boundary. Acceptance: no duplicate apply, no missed event, no infinite retry.
6. **Schema migration replay.** v9 migration on copy of user's 51k-msg DB. Acceptance: <60s, no FTS hits lost, `msg_version` initialized.
7. **Backfill ordering races + per-chat reconciliation cost.** Insert + edit + delete interleaved during backfill on same `(chat_id, message_id)`. Also measure full-corpus per-chat diff cost across all chats. Acceptance: cloud final state = local final state; full diff < 5s for 51k msgs.
8. **Opt-out purge end-to-end** (privacy audit). Toggle chat off; verify outbox cancel, server cascade, change_log scrub, ask_history purge, local purge. Acceptance: nothing for purged chat survives anywhere except documented log residual.
9. **TLS + Tailscale ACL.** Self-signed cert pinned client-side; ACL `tag:tg-seoyu-cloud` reachable only from Mac tailnet. Acceptance: external test from non-tailnet host fails.
10. **Change-log gap + snapshot rebackfill.** Force `last_pull_seq < min_seq` (truncate change_log on cloud, advance min_seq); verify Mac detects gap, requests snapshot, applies cleanly. Acceptance: post-snapshot wiki cache equals server state.
11. **v9 event-type replay.** For every event type in the table above, apply to a clean Mac wiki cache + verify final state matches server. Catches missing event handlers early. Acceptance: all 16 event types replayable.

Fail any of (1)–(3) and the architecture changes fundamentally.

## Risks

1. Plan 0 unmerged. Hard prereq.
2. aarch64 build complexity. Spike 1.
3. Two-txn outbox needs reconciliation discipline. Spike 5 + 7.
4. Oracle reclamation. Paid tier (~$5/mo) is documented fallback.
5. Tailscale DERP fallback latency. Fine for backfill; matters for `Ask`. Spike 4.
6. `msg_version` correctness. Spike 7 race tests.
7. Schema/protocol coordination. Spike 6 + step 13.
8. Codex auth expiry. Step 7 health probe + alert.
9. Multi-device. Two Macs both bump `msg_version` → conflicting histories. Defer w/ explicit non-goal.
10. Outbox unbounded growth offline. Cap 1M rows; oldest evicted (msg still in `messages` table, re-uploadable).
11. Per-chat opt-out purge is async — settings UI must surface progress + warn that residual logs may exist for log-rotation window.
12. Reconciliation cost on huge chats (>100k msgs each). Spike 7 measures.
13. Snapshot rebackfill payload size — 220k msgs × ~200 bytes JSON ≈ 44 MB; acceptable on Tailscale.

## Open questions (require user decision before code)

- [x] Default `backend = cloud` per user direction "all to remote" (overrides codex preference for `local` default).
- [ ] Oracle paid-tier promotion budget: pre-approved, or wait for first reclaim alert?
- [ ] Per-chat opt-out UX: list view w/ checkboxes, or inline per-chat menu?
- [ ] Prometheus scrape of `tg-seoyu-worker` into existing Oracle stack? Recommend yes.
- [ ] `protocol_version` mismatch: hard-fail w/ alert, or warn-and-degrade?
- [ ] Tombstone retention: 30d default; configurable or fixed?
- [ ] Server-log residual on opt-out: acceptable w/ disclosure, or block ship until log scrubbing fully verified?

## Non-goals

- Multi-user / multi-tenant
- Cloud-primary search
- E2EE inside Oracle box (deferred to cloud-worker v2)
- DB encryption on Mac (separate work)
- Web UI for wiki
- Replacing local SQLite

---

**Approval gate**. Before steps 0–3 ship, user signs off on:

1. Hybrid (local search + cloud wiki) confirmed.
2. Default `backend = cloud` (user overrode codex's `local` recommendation).
3. Spike list approved; spikes 1–3 must pass before step 4.
4. Oracle paid-tier promotion policy decided.
5. README rewrite text agreed.
6. Plan 0 lands first.
