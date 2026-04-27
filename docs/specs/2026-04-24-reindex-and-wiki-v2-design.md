# Reindex + Wiki v2 — Design

> Date: 2026-04-24 (rev 2026-04-25 after codex critique pass)
> Status: proposed, phase 1-4 (v8 message index) already landed on disk
> Scope: message index rewrite (schema v8) **and** wiki redesign (schema v9)
> Supersedes: `docs/handoff.md` phase list, `docs/specs/2026-04-23-wiki-panel-design.md`,
> `docs/legacy/2026-04-06-wiki-feature-design.md`

---

## 0. Goal

Rebuild the sidecar's indexing end-to-end so that:

1. **Message search** returns cleaner results with bm25 + recency (instead of
   3-table UNION + `priority`), supports upsert/delete, returns accurate
   `(inserted, updated)` counts, and maintains Korean (jamo / nospace)
   parity.
2. **Wiki** stops being a dead list of lifetime counts and becomes an
   ambient intelligence layer over all chats with four surfaces sharing one
   pipeline:
   - **Radar** — what is hot cross-chat, now, *with a concrete reason why*.
   - **Digest** — what changed in my chats since last open.
   - **Library** — durable pages (topic / event / entity), including resolved history.
   - **Ask** — query → synthesized answer grounded in real messages.

One pipeline, four surfaces. Backfill runs post-migration; user sees
indexing progress in the panel and results stream in as classify drains.

## 0.1 Author decisions (from brainstorming pass)

| Question | Decision |
|---|---|
| 1 msg → how many pages | Up to N pages per msg. Subagent classify may split a msg across multiple pages (e.g. "삼성전자 실적" and "반도체 섹터" both touched by one post). Evidence uniqueness is `(page_id, msg_id, chat_id)`; same msg can co-exist under multiple pages. |
| Privacy / chat allowlist | None. All chats in scope by default. No per-chat exclusion at launch. Settings has one global "pause wiki / stop sending to codex" kill switch. |
| Crash mid-backfill | Resume exact queue state. Preserve v1's `status + claimed_at TTL` pattern (see `sidecar/src/store/schema.rs:412-426`). On startup, any row with `status='processing' AND claimed_at < now - 5min` reverts to `pending`. |
| Resolved events | Searchable and askable (stay in `pages_fts`), but hidden from Radar and Digest. Library has a "show resolved" toggle, default off. |
| Ask placement | Inline. Expands below the search bar, pushes dashboard content down, streams in-place. ESC / ⌘W closes, `ask_history` row persists. |
| Trending card — "why is this hot" | Each Radar card shows **both** a data-driven `reason` (structured, SQL-derived) and a prose `hook` (LLM, one-liner). Users never see a generic "N msgs" row with no explanation. |

---

## 1. Non-goals

- No vector DB, no embeddings. BM25 + codex rerank is the ranking stack.
- No extra process, no external search service. sqlcipher 4.6.1 / SQLite
  3.46.1 carries everything.
- No salvage of v1 wiki output. Titles like "시장 심리 1359 msgs" do not
  migrate; the new prompt + reason fields are stricter.
- No cross-device sync.
- No TDLib / grammers in the sidecar.
- No lindera / unicode-segmentation. Hangul normalization stays
  codepoint-based in `sidecar/src/search/hangul.rs`.

---

## 2. User-facing outcome

Before (current `main`, screenshot 2026-04-24):

```
Trending
  시장 심리        1359 msgs
  비트코인 가격 업데이트   3355 msgs
  불장 심리        1563 msgs
  …
```

Generic titles. Lifetime counts. No reason.

After:

```
Radar                              1h | 24h | 7d

 ①  비트코인 ETF 재유입, 113k 돌파             ▂▃▃▅▇█
     [event · active]
     why: 42 evidence · 4 chats · 3× vs prior 24h · last 8m
     hook: 스팟 ETF 순유입 4일 연속, 숏스퀴즈 경계
     ▸ #crypto-kr   @han  "ETF 재유입 확인, 숏포지션 청산…"    8m
     ▸ #macro-kr    @lee  "113k 저항 돌파 시 120k 단기 타겟"  31m

 ②  …
```

Every card carries:

- data `reason` (SQL, always true): evidence count, chat spread, velocity, recency
- prose `hook` (LLM): one line why you should care today
- kind chip, state chip
- sparkline (hourly bins over window)
- top-2 evidence rows w/ chat + sender + excerpt + time
- click evidence → jump to chat

Search gains an **Ask** button (⌘↵) that expands inline under the search
bar, streams a markdown answer with validated `[n]` citations.

---

## 3. Architecture at a glance

```
Postbox observer ──► sidecar.ingest(msg)
                          │
                          ├── messages upsert + messages_fts   (v8, shipped)
                          └── classify_queue.enqueue(msg_id, chat_id)

worker loop (weighted fair-share scheduler)
  reserve 40% classify, 20% rewrite, 15% trending, 25% ask
  (over a 60s moving window; unused reserve spills)
    classify    gpt-5.5-nano, batch=20
    rewrite     gpt-5.5,      per-page, debounced
    trending    gpt-5.5,      one call per dirty window, ≥5min between
    ask         gpt-5.5-fast, streamed, user-waiting

reads (never touch codex)
  wiki_search  →  FTS5 fanout over pages_fts / evidence_fts / messages_fts
  wiki_radar   →  trending_cache read
  wiki_digest  →  evidence regroup since wiki_settings.last_open_at[chat_id]
  wiki_ask     →  FTS retrieve + codex-fast stream (only Ask touches LLM on user read)
```

---

## 4. Schema v8 — message index rewrite (shipped)

Already implemented on disk as of 2026-04-24 (46-edit codex run). Summary
kept here for reference; see `sidecar/src/store/schema.rs`,
`sidecar/src/store/message.rs`, `sidecar/src/search/engine.rs`,
`sidecar/src/uniffi_api.rs`.

- Single external-content FTS5: `messages_fts(text_plain, text_stripped, text_jamo)`.
- Upsert w/ `RETURNING rowid`, FTS reindex on text change.
- `delete_messages(refs)` UniFFI + IPC surface.
- `IndexOutcome { inserted, updated }`.
- BM25 + linear recency in search, cursor `(rank, ts, chat_id, id)`.
- LIKE fallback covers all three text columns.
- `./scripts/build-dev.sh --run` snapshots DB before launch.
- Outstanding: **Swift Postbox deletion observer** — Rust side ready, Swift
  hook in `Telegram-Mac/Seoyu/SeoyuBridge.swift` TBD (phase 2b).

---

## 5. Schema v9 — wiki v2

### 5.1 Drop (deferred until v2 backfill complete)

v1 wiki tables are **not** dropped at migration time. They are renamed
with `_v1` suffix and the drop happens in a later idempotent step once
`wiki_settings.v2_backfill_complete='1'` is set by the backfill runner.

```
ALTER TABLE wiki_topics          RENAME TO wiki_topics_v1;
ALTER TABLE wiki_topic_aliases   RENAME TO wiki_topic_aliases_v1;
ALTER TABLE wiki_topic_messages  RENAME TO wiki_topic_messages_v1;
ALTER TABLE wiki_pages           RENAME TO wiki_pages_v1;
ALTER TABLE wiki_page_sources    RENAME TO wiki_page_sources_v1;
ALTER TABLE wiki_classify_queue  RENAME TO wiki_classify_queue_v1;
ALTER TABLE wiki_categories      RENAME TO wiki_categories_v1;
```

A later migration `drop_v1_wiki` runs when `v2_backfill_complete='1'`
and is idempotent (drops only if present).

### 5.2 Core tables

```sql
CREATE TABLE wiki_pages (
    id                            INTEGER PRIMARY KEY,
    kind                          TEXT NOT NULL
                                      CHECK (kind IN ('topic','event','entity')),
    title                         TEXT NOT NULL,
    title_norm                    TEXT NOT NULL,                 -- lower + nfkc + strip
    summary_md                    TEXT NOT NULL DEFAULT '',
    summary_rev                   INTEGER NOT NULL DEFAULT 0,
    state                         TEXT NOT NULL DEFAULT 'active'
                                      CHECK (state IN ('active','resolved','frozen','hidden')),
    pinned                        INTEGER NOT NULL DEFAULT 0,
    facts                         TEXT,                          -- JSON, kind-specific
    facts_version                 INTEGER NOT NULL DEFAULT 1,
    evidence_count                INTEGER NOT NULL DEFAULT 0,
    last_rewrite_evidence_count   INTEGER NOT NULL DEFAULT 0,
    last_evidence_at              INTEGER,
    last_rewrite_at               INTEGER,
    created_at                    INTEGER NOT NULL,
    updated_at                    INTEGER NOT NULL
);

CREATE UNIQUE INDEX ux_pages_title_norm ON wiki_pages(title_norm);
CREATE INDEX ix_pages_active_evidence
    ON wiki_pages(state, last_evidence_at DESC)
    WHERE state = 'active';
CREATE INDEX ix_pages_kind_state ON wiki_pages(kind, state);

-- aliases as an indexed table (not JSON) for scale at 100k+ pages.
CREATE TABLE wiki_page_aliases (
    page_id     INTEGER NOT NULL REFERENCES wiki_pages(id) ON DELETE CASCADE,
    alias_norm  TEXT NOT NULL,                 -- nfkc + lower + strip
    alias_raw   TEXT NOT NULL,
    PRIMARY KEY (page_id, alias_norm)
);
CREATE INDEX ix_aliases_norm ON wiki_page_aliases(alias_norm);

CREATE TABLE wiki_evidence (
    id          INTEGER PRIMARY KEY,
    page_id     INTEGER NOT NULL REFERENCES wiki_pages(id) ON DELETE CASCADE,
    msg_id      INTEGER NOT NULL,
    chat_id     INTEGER NOT NULL,
    sender_id   INTEGER NOT NULL,
    ts          INTEGER NOT NULL,
    excerpt     TEXT NOT NULL,
    source_hash BLOB NOT NULL,           -- BLAKE3(page_id || msg_id || chat_id || normalized_excerpt), 16 bytes
    salience    REAL NOT NULL DEFAULT 0.5,
    cited       INTEGER NOT NULL DEFAULT 0,   -- incremented when cited by Ask; retention protects cited rows
    created_at  INTEGER NOT NULL,
    UNIQUE (page_id, msg_id, chat_id)        -- one msg may have evidence on N pages, one row per (page,msg)
);
-- source_hash is kept for cross-session idempotency checks but is NOT
-- a unique index: only (page_id, msg_id, chat_id) is enforced. source_hash
-- is BLAKE3 over the concatenation of the decimal text of page_id, msg_id,
-- chat_id, and the NFC-normalized excerpt bytes (no separators beyond length
-- prefixes), truncated to 16 bytes. Used for retention-sweep dedup and
-- cross-page equality checks.
CREATE INDEX ix_evidence_source_hash ON wiki_evidence(source_hash);

CREATE INDEX ix_evidence_page_ts   ON wiki_evidence(page_id, ts DESC);
CREATE INDEX ix_evidence_chat_ts   ON wiki_evidence(chat_id, ts DESC);
CREATE INDEX ix_evidence_ts        ON wiki_evidence(ts DESC);
CREATE INDEX ix_evidence_msg       ON wiki_evidence(msg_id, chat_id);   -- for edit/delete propagation

CREATE TABLE wiki_classify_queue (
    msg_id          INTEGER NOT NULL,
    chat_id         INTEGER NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending','processing','failed','done')),
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    hint            TEXT,                     -- e.g. 'successor_needed', 'resolved_context'
    hint_page_id    INTEGER REFERENCES wiki_pages(id) ON DELETE SET NULL,
    text_hash       BLOB NOT NULL,            -- BLAKE3(text_plain) at enqueue time; lets ingest detect text edits
    enqueued_at     INTEGER NOT NULL,
    claimed_at      INTEGER,                  -- processing-lease TTL (5 min)
    next_attempt_at INTEGER,                  -- backoff scheduling
    PRIMARY KEY (msg_id, chat_id)
);
CREATE INDEX ix_classify_ready
    ON wiki_classify_queue(status, next_attempt_at)
    WHERE status = 'pending';

CREATE TABLE wiki_rewrite_queue (
    page_id         INTEGER PRIMARY KEY REFERENCES wiki_pages(id) ON DELETE CASCADE,
    status          TEXT NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending','processing','failed','done')),
    attempts        INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    enqueued_at     INTEGER NOT NULL,
    claimed_at      INTEGER,
    next_attempt_at INTEGER
);

CREATE TABLE trending_cache (
    window            TEXT NOT NULL CHECK (window IN ('1h','24h','7d')),
    page_id           INTEGER NOT NULL REFERENCES wiki_pages(id) ON DELETE CASCADE,
    rank              INTEGER NOT NULL,
    hook              TEXT NOT NULL,                 -- LLM prose, ≤90 chars
    reason_code       TEXT NOT NULL,                 -- enum; see §6.4
    reason_metrics    TEXT NOT NULL,                 -- JSON { evidence, chats, senders, velocity, last_seen_sec_ago }
    sparkline         TEXT NOT NULL,                 -- JSON u32[24] bins over window
    computed_at       INTEGER NOT NULL,
    PRIMARY KEY (window, page_id),
    UNIQUE (window, rank)
);
CREATE INDEX ix_trending_window_rank ON trending_cache(window, rank);

CREATE TABLE trending_watermark (
    window              TEXT PRIMARY KEY CHECK (window IN ('1h','24h','7d')),
    last_evidence_id    INTEGER NOT NULL DEFAULT 0,   -- max wiki_evidence.id seen when cache written
    last_computed_at    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE ask_history (
    id            INTEGER PRIMARY KEY,
    query         TEXT NOT NULL,
    answer_md     TEXT NOT NULL,
    cited_sources TEXT NOT NULL,                      -- JSON [{evidence_id, msg_id, chat_id}]
    model         TEXT NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('streaming','done','cancelled','failed')),
    created_at    INTEGER NOT NULL,
    finished_at   INTEGER
);

CREATE TABLE wiki_settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);

CREATE TABLE wiki_last_open (
    chat_id        INTEGER PRIMARY KEY,
    last_open_at   INTEGER NOT NULL
);
-- digest cursor is per-chat, not global.
```

Seeded settings (on migration):

```
max_codex_calls_per_hour_total = 500
model_classify  = gpt-5.5-nano              # TBD exact codex id
model_rewrite   = gpt-5.5
model_trending  = gpt-5.5
model_ask       = gpt-5.5-fast
classify_batch_size = 20
rewrite_per_hour_cap = 30
trend_refresh_min_interval_sec = 300
trend_window_min_refresh_sec = 3600   # 7d window only needs hourly refresh
min_classify_chars = 12               # characters, not bytes, NFC-normalized
max_classify_attempts = 3
max_rewrite_attempts = 3
max_ask_attempts = 2
retention_evidence_per_page = 200
fuzzy_title_dedup = 0                 # off by default; alias-based dedup only
pause_codex = 0
pause_on_low_battery = 1
low_battery_threshold_percent = 20
v2_backfill_complete = 0
v2_backfill_allow_failed_tolerance = 100   # allow completion if failed<=N; surfaced in settings
schema_v9_marker = 1
```

### 5.3 FTS

Wiki FTS tables use external-content FTS5 over concrete backing rows
so DELETE/UPDATE work through normal triggers. Jamo columns live on the
backing rows (not UNINDEXED shadows on a contentless table).

```sql
-- Flat backing table: one row per (page_id) for pages FTS.
-- Rebuilt from wiki_pages + wiki_page_aliases on rewrite and on
-- alias insert/delete. Trigger-less to keep semantics obvious.
CREATE TABLE wiki_pages_index (
    page_id       INTEGER PRIMARY KEY REFERENCES wiki_pages(id) ON DELETE CASCADE,
    title         TEXT NOT NULL,
    aliases       TEXT NOT NULL DEFAULT '',   -- space-joined alias_raw list
    summary_md    TEXT NOT NULL DEFAULT '',
    title_jamo    TEXT NOT NULL DEFAULT '',
    aliases_jamo  TEXT NOT NULL DEFAULT '',
    summary_jamo  TEXT NOT NULL DEFAULT ''
);

CREATE VIRTUAL TABLE pages_fts USING fts5(
    title, aliases, summary_md,
    title_jamo, aliases_jamo, summary_jamo,
    content='wiki_pages_index',
    content_rowid='page_id',
    tokenize='trigram case_sensitive 0'
);

-- Evidence FTS is external-content over wiki_evidence directly.
-- Jamo lives as a column on wiki_evidence.
ALTER TABLE wiki_evidence ADD COLUMN excerpt_jamo TEXT NOT NULL DEFAULT '';

CREATE VIRTUAL TABLE evidence_fts USING fts5(
    excerpt, excerpt_jamo,
    content='wiki_evidence',
    content_rowid='id',
    tokenize='trigram case_sensitive 0'
);
```

External-content FTS5 semantics (confirmed for sqlcipher 4.6.1 / SQLite
3.46.1):

- `INSERT INTO wiki_evidence(...) → INSERT INTO evidence_fts(rowid, ...)
  VALUES (new.id, ...)` — manual, not automatic.
- Deletes from backing table **do not** auto-remove FTS rows on
  external-content tables; we explicitly `DELETE FROM evidence_fts
  WHERE rowid = :id` in every evidence removal code path (retention,
  page cascade, v1 drop).
- Page FTS rebuild uses `DELETE FROM pages_fts WHERE rowid = :id; INSERT INTO pages_fts(rowid, ...) SELECT ... FROM wiki_pages_index WHERE page_id = :id;`
  (wrapped in helper `fts::refresh_page(page_id)`). Wiki_pages_index is
  rebuilt from the live row inside the same txn.

Insert/delete helpers live in `sidecar/src/store/wiki/fts.rs`. Every
write path that touches `wiki_pages`, `wiki_page_aliases`, or
`wiki_evidence` MUST call the matching helper in the same transaction.

### 5.4 Per-kind `facts` JSON

Validated against a Rust serde struct at rewrite time. Unknown keys are
preserved but the canonical shape is:

```
topic:  { "facts_version": 1 }
event:  { "facts_version": 1,
          "started_at": ts,
          "resolved_at": ts|null,
          "severity": "info"|"warn"|"high"|null,
          "resolution_note": string|null }
entity: { "facts_version": 1,
          "canonical_name": string,
          "relations": [{ "name": string, "type": string }],
          "last_seen": ts }
```

Aliases live only in `wiki_page_aliases`. `facts` never duplicates them.

---

## 6. Pipeline

### 6.1 Ingest

```
ingest(msg):
    messages upsert + messages_fts   (v8)
    if chars_nfc(text) >= min_classify_chars and not service_msg
       and wiki_settings.pause_codex = 0:
        text_hash_new = blake3_16(text_plain_nfc)
        existing = SELECT status, text_hash FROM wiki_classify_queue
                   WHERE msg_id=? AND chat_id=?;
        match existing:
            None ->
                INSERT INTO wiki_classify_queue
                    (msg_id, chat_id, status, attempts, text_hash,
                     enqueued_at, next_attempt_at)
                VALUES (?, ?, 'pending', 0, text_hash_new, now, now);
            Some(row) when row.text_hash == text_hash_new ->
                -- same text; no-op regardless of row.status
                noop;
            Some(row) when row.status IN ('done','processing','failed') ->
                -- text actually changed -> reclassify from scratch.
                -- (Prior evidence under (page_id, msg_id, chat_id) is
                -- invalidated lazily: classify apply replaces excerpt by
                -- source_hash miss; retention sweep drops orphans.)
                UPDATE wiki_classify_queue
                   SET status='pending', attempts=0, last_error=NULL,
                       hint=NULL, hint_page_id=NULL,
                       text_hash=text_hash_new, claimed_at=NULL,
                       next_attempt_at=now, enqueued_at=now
                 WHERE msg_id=? AND chat_id=?;
            Some(row) when row.status = 'pending' ->
                -- edited before it was picked; bump hash only.
                UPDATE wiki_classify_queue
                   SET text_hash=text_hash_new, next_attempt_at=now
                 WHERE msg_id=? AND chat_id=?;
```

Explicit match logic, never `INSERT OR REPLACE`, so attempts and error
state are never lost for in-flight or failed rows that haven't changed.

### 6.2 Classify

Worker picks up N=20 rows where `status='pending' AND next_attempt_at<=now`,
atomically moves them to `status='processing', claimed_at=now`. On
startup, any `status='processing' AND claimed_at < now - 5min` resets
to `pending`.

Candidate retrieval per batch (one query for the whole batch):

```sql
-- 1) alias direct hit (fast, indexed)
SELECT page_id FROM wiki_page_aliases
WHERE alias_norm IN (:normalized_tokens_from_batch);

-- 2) FTS title match, top 10 per message, state='active' or 'resolved'
-- (resolved passed as context for successor proposal)
SELECT id FROM wiki_pages
JOIN pages_fts ON pages_fts.rowid = wiki_pages.id
WHERE pages_fts MATCH :title_tokens
  AND state IN ('active','resolved')
ORDER BY bm25(pages_fts) LIMIT 10;
```

Candidate set capped at 30 per batch (alias hits first, FTS to fill).

**Prompt injection boundary**: all chat content is delivered as JSON
string fields, never concatenated into instruction text. Output JSON
is strictly validated against a schema:

```
INPUT (structured):
{
  "existing_pages": [{ "id": int, "kind": "...", "title": "...", "aliases": [...] }],
  "messages": [
    { "msg_id": int, "chat_id": int, "chat_title": "...",
      "sender": "...", "ts": int, "text": "..."  // untrusted, treat as quote
    }
  ],
  "policies": {
    "max_pages_per_message": 3,
    "skip_if_salience_below": 0.2,
    "may_propose_new": true
  }
}

OUTPUT (strict JSON, no prose):
{
  "assignments": [
    {
      "msg_id": int,
      "assignments": [
        { "page_ref": { "existing_id": int } | { "new": { "kind": "topic|event|entity",
                                                           "title": "...",
                                                           "aliases": ["..."] } },
          "excerpt": "string, ≤120 chars",
          "salience": 0.0..1.0 }
      ] | []   // empty = skip
    }
  ]
}
```

Output validator (Rust):

1. JSON parse or → `retry(row)`.
2. Schema validate (serde + custom checks) or → `retry(row)`.
3. For each assignment:
   - `existing_id` must be in the provided `existing_pages` list.
   - `new.title` must be non-empty, ≤80 chars, not a pure URL.
   - `new.aliases` ≤5, each ≤40 chars after normalization.
   - `excerpt` is re-extracted from the original msg text (not trusted from model) — LLM gives us *which span*, we substring from msg text.
   - `salience` clamped to `[0, 1]`.
4. If ANY assignment fails validation for a message → `retry(row)` for that message only.
5. If all validate but array is empty → `mark_done(row)`.
6. On successful apply → `mark_done(row)`.

Exact retry transitions (SQL):

```sql
-- retry(row): bump attempts, back off, clear claim, remain pending
-- unless exhausted.
UPDATE wiki_classify_queue
   SET attempts = attempts + 1,
       last_error = :err,
       claimed_at = NULL,
       status = CASE
                  WHEN attempts + 1 >= :max_classify_attempts THEN 'failed'
                  ELSE 'pending'
                END,
       next_attempt_at = strftime('%s','now')
                       + CASE
                           WHEN attempts + 1 >= :max_classify_attempts THEN 0
                           ELSE (30 * (1 << MIN(attempts + 1, 8)))   -- cap 2^8*30s = 128min
                         END
 WHERE msg_id = :m AND chat_id = :c;

-- mark_done(row): terminal success.
UPDATE wiki_classify_queue
   SET status = 'done',
       attempts = attempts + 1,
       claimed_at = NULL,
       last_error = NULL
 WHERE msg_id = :m AND chat_id = :c;
```

Failed rows are surfaced in settings (§12) with one-click
`wiki_reset_failed_classify` which flips them back to `pending`,
`attempts=0`, `next_attempt_at=now`.

Apply (one txn per message, not per batch, so a single bad msg cannot
poison the rest):

```sql
BEGIN IMMEDIATE;
needs_successor := false;
successor_context := [];

FOR each assignment:
    page_id = dedup_or_insert(kind, title, aliases);   -- returns id
    state = (SELECT state FROM wiki_pages WHERE id = page_id);

    IF state = 'frozen' OR state = 'hidden':
        CONTINUE;                                   -- drop assignment silently

    IF state = 'resolved':
        -- Route: evidence does NOT append; instead mark queue row so next
        -- classify pass gets this resolved page as explicit "successor_needed"
        -- context and must either propose a NEW event page or skip.
        needs_successor := true;
        successor_context.push(page_id);
        CONTINUE;

    new_id = INSERT INTO wiki_evidence(
        page_id, msg_id, chat_id, sender_id, ts,
        excerpt, excerpt_jamo, source_hash, salience, created_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT (page_id, msg_id, chat_id) DO NOTHING
        RETURNING id;

    IF new_id IS NOT NULL:
        UPDATE wiki_pages
           SET evidence_count = evidence_count + 1,
               last_evidence_at = ts,
               updated_at = now
         WHERE id = page_id;
        -- external-content FTS: manual insert inside the SAME branch.
        INSERT INTO evidence_fts(rowid, excerpt, excerpt_jamo)
            VALUES (new_id, :excerpt, :excerpt_jamo);
        maybe_enqueue_rewrite(page_id);
    -- else: duplicate (page_id, msg_id, chat_id) — no-op, no count bump,
    -- no FTS insert. Idempotent re-apply is safe.

IF needs_successor AND no other assignment succeeded:
    -- Re-queue the message specifically with successor context for the
    -- next classify call.
    UPDATE wiki_classify_queue
       SET status = 'pending',
           hint = 'successor_needed',
           hint_page_id = :first_resolved_page_id,   -- single hint for prompt clarity
           attempts = attempts + 1,
           claimed_at = NULL,
           next_attempt_at = strftime('%s','now') + 30
     WHERE msg_id = :m AND chat_id = :c;
ELSE:
    mark_done(row);

COMMIT;
```

Dedup (`dedup_or_insert`):

1. Exact match by `title_norm` → reuse, merge any new aliases.
2. Any `alias_norm` in candidate aliases hits `wiki_page_aliases` → reuse,
   merge aliases.
3. Optional fuzzy (Jaccard over trigram shingles, threshold 0.82) —
   gated by `wiki_settings.fuzzy_title_dedup=1`. Default off.
4. Else insert new page + aliases.

### 6.3 Rewrite

Trigger enqueue (inside classify txn, idempotent via PK):

```
maybe_enqueue_rewrite(page_id):
    if page.evidence_count - page.last_rewrite_evidence_count >= 20
       OR (now - page.last_rewrite_at >= 86400
           AND page.evidence_count > page.last_rewrite_evidence_count):
        INSERT OR REPLACE INTO wiki_rewrite_queue(page_id, status, attempts,
                                                  enqueued_at, next_attempt_at)
            VALUES (page_id, 'pending', 0, now, now);
```

Worker claims `pending` rewrites with TTL lease, selects evidence:

- Up to 50 rows, composed from:
  - all evidence newer than `last_rewrite_at` (the delta), up to 30 rows
  - plus top-K by `salience` from the remainder, up to 20 rows
  - plus any rows with `cited > 0` (always preserved if fit)
- Feeds prior `summary_md`, prior `facts`, and the 50 rows to codex.

Prompt (per kind, JSON input/output, same injection boundary as classify):

```
OUTPUT schema (model cannot emit 'frozen' or 'hidden'):
{
  "summary_md":        "string, ≤400 words for topic, ≤600 for event w/ timeline",
  "facts":             { ...kind schema, facts_version:1 },
  "new_aliases":       ["..."],
  "state":             "active"|"resolved",
  "resolution_note":   string|null   // required iff state='resolved' AND kind='event'
}
```

Validator:
- `state` transitions allowed: `active → active`, `active → resolved` (event kind only). Any other input rejects.
- Model-emitted `frozen` or `hidden` is rejected and the row retries. Those states are admin-only (future `wiki_freeze(id)` / `wiki_hide(id)` API).
- `summary_md` word limit enforced.
- `facts` shape matches `kind`, `facts_version=1`.
- `new_aliases` filtered through alias validator; ≤5.

Apply (single txn):

```sql
BEGIN IMMEDIATE;
UPDATE wiki_pages
   SET summary_md = :summary_md,
       summary_rev = summary_rev + 1,
       facts = :facts_json,
       state = :state,
       last_rewrite_at = now,
       last_rewrite_evidence_count = evidence_count,
       updated_at = now
 WHERE id = :id;
INSERT OR IGNORE INTO wiki_page_aliases(page_id, alias_norm, alias_raw)
    VALUES (:id, :norm, :raw), ... ;                 -- new aliases

-- Rebuild wiki_pages_index row, then refresh pages_fts.
INSERT INTO wiki_pages_index(page_id, title, aliases, summary_md,
                             title_jamo, aliases_jamo, summary_jamo)
    VALUES (:id, :title, :aliases_joined, :summary_md,
            :title_jamo, :aliases_jamo, :summary_jamo)
    ON CONFLICT(page_id) DO UPDATE SET
        title = excluded.title,
        aliases = excluded.aliases,
        summary_md = excluded.summary_md,
        title_jamo = excluded.title_jamo,
        aliases_jamo = excluded.aliases_jamo,
        summary_jamo = excluded.summary_jamo;
DELETE FROM pages_fts WHERE rowid = :id;
INSERT INTO pages_fts(rowid, title, aliases, summary_md,
                      title_jamo, aliases_jamo, summary_jamo)
    SELECT page_id, title, aliases, summary_md,
           title_jamo, aliases_jamo, summary_jamo
      FROM wiki_pages_index WHERE page_id = :id;

-- Retention sweep (must also remove evidence_fts rows).
-- Keep:
--   1. rows with cited > 0
--   2. rows from last 24h
--   3. top-2 per chat_id by (ts DESC, salience DESC)
-- Then among the remainder, drop lowest-salience rows until total ≤ retention_evidence_per_page.
WITH keep AS (
    SELECT id FROM wiki_evidence
     WHERE page_id = :id
       AND ( cited > 0
             OR ts >= (strftime('%s','now') - 86400)
             OR id IN (
                 SELECT id FROM (
                     SELECT id,
                         row_number() OVER (
                             PARTITION BY chat_id
                             ORDER BY ts DESC, salience DESC
                         ) AS rn
                       FROM wiki_evidence WHERE page_id = :id
                 ) WHERE rn <= 2
             )
           )
), candidates AS (
    SELECT id FROM wiki_evidence
     WHERE page_id = :id AND id NOT IN (SELECT id FROM keep)
     ORDER BY salience ASC, ts ASC
     LIMIT MAX(0, (SELECT COUNT(*) FROM wiki_evidence WHERE page_id=:id)
                - :retention_evidence_per_page)
)
DELETE FROM evidence_fts WHERE rowid IN (SELECT id FROM candidates);
DELETE FROM wiki_evidence WHERE id IN (SELECT id FROM candidates);

UPDATE wiki_pages
   SET evidence_count = (SELECT COUNT(*) FROM wiki_evidence WHERE page_id = :id)
 WHERE id = :id;

UPDATE wiki_rewrite_queue SET status='done' WHERE page_id=:id;
COMMIT;
```

If the update fails (e.g. facts schema invalid after parse), row stays
`processing`, TTL expires it, attempt count grows, `next_attempt_at`
backs off; `max_rewrite_attempts=3` before `status='failed'` and a
user-visible flag.

### 6.4 Trending

**Dirty-window detection**: a trending window W is dirty iff
`(SELECT MAX(id) FROM wiki_evidence) > trending_watermark.last_evidence_id`
for that window. Using the monotonic `wiki_evidence.id` watermark
(rather than `ts`) catches late-inserted evidence with old timestamps
(e.g. during backfill or historical re-classify). On refresh, we write
`last_evidence_id = MAX(id)` and `last_computed_at = now` to
`trending_watermark`. Clean windows skip their refresh entirely.

**Minimum refresh gap**: 5 min for 1h+24h, 1h for 7d. Codex offline → last
cache is served with a `stale=true` UI hint.

SQL shortlist (top 30 per dirty window, atomic):

```sql
WITH window_evidence AS (
    SELECT page_id, chat_id, sender_id, ts
    FROM wiki_evidence
    WHERE ts >= :window_start AND ts < :now     -- half-open
),
agg AS (
    SELECT page_id,
           COUNT(*)                    AS ec,
           COUNT(DISTINCT chat_id)     AS chats,
           COUNT(DISTINCT sender_id)   AS senders,
           MAX(ts)                     AS last_ts
    FROM window_evidence
    GROUP BY page_id
),
prior AS (
    SELECT page_id, COUNT(*) AS ec2
    FROM wiki_evidence
    WHERE ts >= :prior_start AND ts < :window_start    -- half-open, no overlap
    GROUP BY page_id
),
scored AS (
    SELECT p.id, p.kind, p.created_at, a.ec, a.chats, a.senders, a.last_ts,
           COALESCE(pr.ec2, 0) AS prior_ec,
           (LN(1 + a.ec)
            + 0.5 * LN(1 + a.chats)
            + 0.3 * LN(1 + a.senders)
            + CASE WHEN COALESCE(pr.ec2,0) >= 3
                   THEN LEAST(3.0, (1.0 * a.ec) / pr.ec2) - 1
                   ELSE 0 END                   -- cap velocity at 3×, require ≥3 prior
            - 0.1 * (strftime('%s','now') - a.last_ts) / 3600.0
           ) AS score
    FROM wiki_pages p
    JOIN agg a ON a.page_id = p.id
    LEFT JOIN prior pr ON pr.page_id = p.id
    WHERE p.state = 'active' AND p.pinned = 0
)
SELECT * FROM scored
ORDER BY score DESC, last_ts DESC
LIMIT 30;
```

**Reason code** (derived in code, not LLM):

| code | when |
|---|---|
| `surge` | velocity ≥ 2× prior window |
| `spread` | chats ≥ 4 |
| `fresh_event` | `kind='event' AND (now − created_at) ≤ 2h` |
| `sustained` | ec ≥ window_median × 2 for ≥ 3 consecutive refreshes |
| `cross_chat` | chats ≥ 3 AND senders ≥ 5 |
| `pinned_active` | pinned row w/ ≥ 1 new evidence in window |
| `default` | fell through, still top-ranked by score |

Each Radar card always shows the reason code + concrete metrics in
`reason_metrics`. The UI renders this as e.g.
"42 evidence · 4 chats · 3× vs prior 24h · last 8m" — no LLM required.

Codex rerank (one call per dirty window, at most every 5min/1h per gap
rule):

```
INPUT (JSON):
{
  "window": "24h",
  "candidates": [
    { "page_id": ..., "title": ..., "kind": ..., "reason_code": "surge",
      "metrics": {...}, "samples": ["excerpt 1", "..."] },
    ... 30 ...
  ]
}
OUTPUT (strict):
{
  "ranked": [
    { "page_id": int, "rank": 1..10, "hook": "≤90 chars, Korean or mixed" }
  ]
}
```

Validator:
- `ranked` size ≤10.
- Each `page_id` must be in input candidates.
- `hook` ≤90 chars, no citations, no trailing ellipsis.
- On any validation miss → serve SQL shortlist top 10 with
  `hook = ''` fallback and retry at next refresh.

Apply (atomic replace of window + watermark bump):

```sql
BEGIN IMMEDIATE;
DELETE FROM trending_cache WHERE window = :W;
INSERT INTO trending_cache(window, page_id, rank, hook, reason_code,
                           reason_metrics, sparkline, computed_at)
    VALUES ...;
INSERT INTO trending_watermark(window, last_evidence_id, last_computed_at)
    VALUES (:W, :max_evidence_id_at_shortlist_time, strftime('%s','now'))
    ON CONFLICT(window) DO UPDATE SET
        last_evidence_id = excluded.last_evidence_id,
        last_computed_at = excluded.last_computed_at;
COMMIT;
```

`:max_evidence_id_at_shortlist_time` is captured at the top of the
refresh before the shortlist query runs, so no evidence inserted after
the shortlist will be silently skipped on the next dirty-check.

Pinned pages with ≥1 evidence in window are surfaced above the ranked
list in a separate UI slot (not competing with codex top-10).

### 6.5 Digest

Pure SQL, no LLM.

```sql
-- per chat, group by page since last_open_at[chat_id]
SELECT e.chat_id, e.page_id, p.kind, p.state,
       COUNT(*) AS n, MAX(e.ts) AS last_ts
FROM wiki_evidence e
JOIN wiki_pages p ON p.id = e.page_id
WHERE e.ts > COALESCE(
        (SELECT last_open_at FROM wiki_last_open WHERE chat_id = e.chat_id),
        0)
  AND p.state != 'hidden'
  AND p.state != 'resolved'                 -- resolved hidden from digest
GROUP BY e.chat_id, e.page_id
HAVING n >= 3
ORDER BY e.chat_id, n DESC, last_ts DESC
LIMIT 200;
```

`wiki_mark_read(chat_id)` upserts `wiki_last_open`. Panel open does
NOT auto-advance the cursor — the user mashing the panel open shouldn't
blow away the "since last read" marker unexpectedly; we mark read only
on explicit digest row click or the chat itself being opened.

Optional per-chat "Digest summary" button → codex fast model over that
chat's digest page summaries, capped at 1 call per chat per day.

### 6.6 Ask (streamed, inline)

**Scope**: global across all chats. Per-chat scope is a future toggle.

**Retrieval**: FTS top-5 pages (bm25 on `pages_fts`) + top-20 evidence
(bm25 on `evidence_fts`, time-decayed), dedup'd. Feed into codex fast.

**Prompt boundary**: same JSON-structured input. Query is a trusted
field; evidence rows are quoted untrusted fields with explicit
`source_id` integer keys (1..20).

**Output schema** (strict):

```json
{
  "answer_segments": [
    { "type": "text", "md": "...", "cites": [1, 3] },
    { "type": "text", "md": "...", "cites": [] },
    ...
  ],
  "thin_evidence": false
}
```

LLM does NOT emit inline `[n]` syntax. The host assembles `[n]` markers
from validated `cites` after each segment is received. This makes
citation hallucination structurally impossible — unknown IDs are
stripped before any character is shown.

**Thin-evidence policy** (pre-LLM):
- `< 3 distinct evidence rows` → skip LLM, surface "not enough in your chats yet"
  + raw FTS results below.
- `< 2 distinct chats AND < 3 distinct senders` → `thin_evidence=true` hint
  to model; model may still answer but must flag uncertainty.

**Streaming framing (NDJSON)**: codex is prompted to emit one JSON
object per line on stdout. Host consumes stdout line-by-line; each line
is parsed independently. Partial/trailing lines are buffered until a
newline arrives. Two accepted line shapes:

```
{"type":"segment","seg":0,"md":"...","cites":[1,3]}
{"type":"done","thin_evidence":false}
```

Rules:
- `seg` must monotonically increase from 0.
- `cites` integers must be in `[1..evidence_count_sent]`.
- Host strips unknown `cites` elements; if the entire `cites` array is
  unknown, still emits `on_delta` but no `on_source` for that segment.
- Any line that fails JSON parse → stream cancelled, `ask_history.status='failed'`, `on_error("malformed stream")`.
- `"type":"done"` terminates; without it after stream close the answer
  is marked `failed` with `"incomplete stream"`.

**Cancellation**: `wiki_ask(q)` returns `AskHandle { id }`. `wiki_cancel_ask(id)`
sets `ask_history.status='cancelled'` and sends Ctrl+C to the codex
subprocess via the bridge. Partial answers shown so far are persisted
with the cancelled status.

**Persistence**: completed answers written to `ask_history` with
`cited_sources` (actual evidence rows, not `[n]` labels).

---

## 7. UniFFI surface

```rust
impl Seoyu {
    // v8 (message index) — shipped
    fn index_messages(&self, msgs: Vec<IngestMessage>) -> Result<IndexOutcome>;
    fn delete_messages(&self, refs: Vec<MessageRef>) -> Result<u64>;
    fn search_messages(&self, q: String, cursor: Option<SearchCursor>,
                       limit: u32) -> Result<SearchPage>;

    // v9 (wiki)
    fn wiki_dashboard(&self, window: TrendWindow) -> Result<DashboardSnapshot>;
    fn wiki_page(&self, id: i64) -> Result<WikiPageFull>;
    fn wiki_search(&self, q: String, limit: u32) -> Result<Vec<WikiSearchResult>>;
    fn wiki_ask(&self, q: String, handler: Arc<dyn AskStreamHandler>) -> Result<i64>;
    fn wiki_cancel_ask(&self, id: i64) -> Result<()>;
    fn wiki_pin(&self, id: i64, pinned: bool) -> Result<()>;
    fn wiki_hide(&self, id: i64) -> Result<()>;
    fn wiki_mark_read(&self, chat_id: i64) -> Result<()>;
    fn wiki_settings_get(&self) -> Result<WikiSettings>;
    fn wiki_settings_set(&self, s: WikiSettings) -> Result<()>;
    fn wiki_reset_failed_classify(&self) -> Result<u32>;
    fn wiki_reset_failed_rewrite(&self)  -> Result<u32>;
    fn wiki_reclassify_progress(&self) -> Result<ReclassifyProgress>;
    fn wiki_pause(&self, pause: bool) -> Result<()>;
}

pub struct DashboardSnapshot {
    pub radar: Vec<RadarCard>,                    // sorted rank
    pub pinned: Vec<PageSummary>,
    pub digest: Vec<DigestEntry>,
    pub library_recent: Vec<PageSummary>,
    pub ask_recent: Vec<AskSummary>,
    pub stale: bool,                              // true if trending_cache older than gap
    pub computed_at: i64,
    pub window: TrendWindow,
    pub backfill: Option<BackfillProgress>,
}

pub struct RadarCard {
    pub page_id: i64,
    pub title: String,
    pub kind: PageKind,
    pub state: PageState,
    pub hook: String,                             // may be empty if fallback
    pub reason_code: ReasonCode,                  // enum, UI renders chip
    pub reason_metrics: ReasonMetrics,            // evidence, chats, senders, velocity, last_ago_sec
    pub sparkline: Vec<u32>,
    pub chat_chips: Vec<ChatChip>,
    pub top_evidence: Vec<EvidenceSummary>,       // ≤2
}

pub trait AskStreamHandler: Send + Sync {
    /// Host dispatches to main thread before calling these hooks.
    /// Drops of the handler cancel the stream automatically.
    fn on_delta(&self, segment_index: u32, text: String);
    fn on_source(&self, segment_index: u32, tag: u32, source: EvidenceSummary);
    fn on_finished(&self, ask_id: i64);
    fn on_cancelled(&self, ask_id: i64);
    fn on_error(&self, ask_id: i64, message: String);
}
```

**Threading contract** (§7 hard rule): the sidecar wraps any
`AskStreamHandler` invocation through `dispatch_main` — Swift
implementations may touch AppKit directly. Callbacks for a single
handle are serialized. Dropping the `Arc<dyn AskStreamHandler>` on the
Swift side triggers `wiki_cancel_ask` implicitly.

---

## 8. Swift UI

`Telegram-Mac/Seoyu/Wiki/` rebuild. New controllers:

```
WikiDashboardController
  ├─ WikiSearchBarView
  │     └─ AskInlineContainerView  (expands when user presses ⌘↵)
  │           ├─ AskStreamingView (tokens+sources)
  │           └─ AskFallbackView  (thin-evidence / offline)
  ├─ WikiRadarSectionView (window tabs 1h | 24h | 7d)
  │     ├─ WikiStaleBannerView (when snapshot.stale)
  │     ├─ WikiPinnedRowView *
  │     └─ WikiRadarCardView *
  │           ├─ title + reason chip
  │           ├─ metrics line
  │           ├─ hook line (may be empty)
  │           ├─ sparkline
  │           └─ evidence rows *
  ├─ WikiDigestSectionView
  │     └─ WikiDigestRowView *      (per chat, grouped pages)
  ├─ WikiLibrarySectionView
  │     ├─ filters (kind, state incl. "show resolved" toggle)
  │     └─ WikiPageRowView *
  └─ WikiAskHistorySectionView

WikiArticleController     — page detail (summary + evidence timeline)
WikiSettingsController    — budgets, pause, reset-failed, reclassify progress
```

UX rules:
- Dashboard density: 8pt vertical padding between sections, sparkline 28×80pt.
- Ask inline: pushes radar down; does not cover it. ESC collapses.
- Keyboard: `/` focuses search, `Esc` collapses Ask, `j/k` moves between Radar cards, `Enter` opens article, `Tab` moves between citations in an Ask answer, `Enter` on citation jumps to msg.
- Sparkline binning: 24 bins always, but bin width = window / 24 (1h → 2.5min bins; 7d → 7h bins). Label the axis accordingly.
- Resolved pages: faded style in Library; hidden from Radar/Digest regardless of filter state.

Removed: `WikiCategoryChipsView.swift`, `WikiSourceCellView.swift` (subsumed),
current `WikiListViewController.swift`.

---

## 9. Worker + scheduling

Single tokio task owns the codex lane. Scheduler is **weighted fair
share over a 60s moving window**:

```
target share (60s):
  classify  40%
  rewrite   20%
  trending  15%
  ask       25%

rule: at dispatch time, pick the class with (target - actual_share) max.
      unused share spills to whoever wants it.
      ask jumps the queue only when no other class is over its target and
      classify has capacity; otherwise ask still runs but concurrent with
      classify on a second parallel codex process.
```

Backpressure:
- codex 429 / timeout → exponential backoff 5s…5min per *model*.
- queue keeps filling; ingest never blocks on LLM.
- `pause_codex=1` setting fully halts worker; ingest and SQL surfaces
  keep working (stale trending, no new evidence).

Battery: `IOPSGetPowerSourcesInfo` via a tiny Swift helper bridged to
Rust. If `pause_on_low_battery=1` (default true) and power source
is battery AND `percent < 20`, worker pauses. Cheap API, justifies the
line of IOKit.

---

## 10. Migration + backfill

**Migration sequence** (single `migrate_v9_wiki` function, idempotent):

1. Rename all v1 wiki tables → `_v1` suffix (if not already renamed).
2. Create v9 tables + FTS + settings defaults.
3. Seed `schema_v9_marker=1`, `v2_backfill_complete=0`.
4. Enqueue backfill — **recency first**:

```sql
INSERT OR IGNORE INTO wiki_classify_queue(msg_id, chat_id, status,
                                          attempts, enqueued_at,
                                          next_attempt_at)
SELECT m.id, m.chat_id, 'pending', 0, strftime('%s','now'),
       -- newer messages processed first (negated timestamp)
       strftime('%s','now') - m.timestamp
FROM messages m
WHERE m.text_plain IS NOT NULL
  AND length(m.text_plain) >= 12;
```

The `next_attempt_at` gradient means "now − ts" — newer msgs have
smaller values (i.e. run sooner). `ix_classify_ready` keeps this cheap.

**Backfill runner** (worker loop entry):

- Reads `pause_codex`, battery, budgets.
- Drains queue at `max_codex_calls_per_hour_total`.
- Dashboard shows `BackfillProgress { queued, processing, done, failed, eta_hours }`.
- Completion condition (checked every 60s when backfill strip visible):
  ```sql
  SELECT
      SUM(CASE WHEN status='pending'    THEN 1 ELSE 0 END) AS pending,
      SUM(CASE WHEN status='processing' THEN 1 ELSE 0 END) AS processing,
      SUM(CASE WHEN status='failed'     THEN 1 ELSE 0 END) AS failed
  FROM wiki_classify_queue;
  ```
  Complete when `pending=0 AND processing=0 AND
  failed <= v2_backfill_allow_failed_tolerance`.
  If `failed > tolerance`: surface prompt in settings "N classify failures
  — review / retry / ignore". User choosing "ignore" raises the
  tolerance setting and re-evaluates.
- On completion:
  - Set `wiki_settings.v2_backfill_complete='1'`.
  - Enqueue one-shot `drop_v1_wiki` migration.
  - Hide backfill strip in UI.

**Crash recovery**:
- Any `status='processing'` row older than 5min → `pending`, `attempts++`.
- `status='failed'` rows ignored by worker; surfaced in settings.
- Migration itself is idempotent; rerunning is safe.
- If user force-quits during backfill, nothing is lost — queue state
  is durable; worker resumes where it left off on next launch.

**Build script safety**: `scripts/build-dev.sh` already snapshots DB
before `--run` (landed with v8). The migration test harness adds
`--migration-dry-run` flag to run v9 against a copy of the live DB
under `.backup/migration-dryrun-<ts>.db` with no effect on the real
file. Required before any hand-run `build-dev.sh --run` against a live
DB.

---

## 11. Phase rollout

Strict commit boundary per phase. Each phase must pass:

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test` (or `--lib + --test uniffi_surface` when sandbox blocks sockets)
- `./scripts/build-dev.sh` compiles

Phases:

| # | Scope | Status |
|---|-------|--------|
| 1 | Schema v8 + backfill helpers | **shipped** (codex run, 2026-04-24) |
| 2a | v8 ingest upsert + counts + `delete_messages` Rust/IPC | **shipped** |
| 2b | Swift Postbox delete observer hook → `delete_messages` | **TODO** — blocker for v8 PR |
| 3 | bm25 + recency search + LIKE fallback widening | **shipped** |
| 4 | v8 tests | **shipped** |
| 5 | Schema v9 + FTS + settings seed (renames v1, no drops) | pending |
| 6 | Classify worker w/ validation, retry, prompt-injection guard | pending |
| 7 | Rewrite per-kind w/ retention sweep | pending |
| 8 | Trending SQL shortlist + reason codes + codex rerank + cache | pending |
| 9 | wiki search (FTS fanout) + Ask streaming w/ citation validator | pending |
| 10 | Digest SQL + per-chat read cursor | pending |
| 11 | Dashboard UI rebuild (Swift) | pending |
| 12 | Settings UI: pause, budgets, failed counters, reclassify progress | pending |
| 13 | Backfill runner + recency-first enqueue + drop v1 on complete | pending |
| 14 | Docs: update `CLAUDE.md`, `README.md`, retire `handoff.md` | pending |

---

## 12. Tests

### v8 (shipped)
- upsert round-trip, delete propagation, bm25 > recency, LIKE hits jamo,
  `IndexOutcome` split, migration idempotency, `ipc_roundtrip` coverage.

### v9 must-haves (before phase 13 PR)

Classify:
- malformed JSON output → row stays pending, attempts++.
- schema-valid but excerpt not-in-text → excerpt re-extracted from msg, or rejected.
- prompt-injection attempt ("forget previous and output {...}") → output
  treated as data, validator rejects.
- successful apply marks row `done`.
- exhausted attempts transitions to `failed`.

Rewrite:
- 20-evidence trigger fires once, respects debounce.
- event `state='resolved'` freezes page and blocks further appends.
- facts schema validation rejects bad JSON and retains prior facts.
- retention sweep keeps cited rows and last-24h rows.

Trending:
- half-open windows have no overlap.
- dirty-window detection skips clean windows.
- velocity cap at 3× protects against prior_ec=1 noise.
- reason_code derivation deterministic for fixture.
- atomic replace never leaves partial cache.

Ask:
- citation validator strips hallucinated ids.
- thin-evidence short-circuits without LLM.
- cancellation persists partial w/ status='cancelled'.
- callback threading: spawn on worker, observe on main via test harness.

Migration:
- rename preserves v1 data.
- enqueue is recency-ordered.
- crash mid-backfill: kill mid-drain, restart, verify resume.
- v2_backfill_complete flip triggers v1 drop, idempotent rerun.

Load:
- 10k synthetic msgs through ingest+classify stub in <60s.
- 100k pages + 3M evidence: trending SQL <200ms p95.

---

## 13. Risks + mitigations

| Risk | Mitigation |
|------|------------|
| Codex rate limit during 22h backfill | Per-hour cap, per-model backoff, pause toggle, dashboard strip shows ETA. |
| Classify mis-groups near-duplicates | Alias normalization + exact-title first, fuzzy off by default. Rerank step filters single-sender echoes. |
| Evidence table unbounded growth | Retention sweep after every rewrite; caps 200/page; keeps cited + recent + per-chat diversity. |
| Edits / deletes not propagating to evidence | v8 delete covers messages; evidence points by `(msg_id, chat_id)` — phase-15 follow-up reclassifies on text edit. Documented, not shipped. |
| Wiki writes block search | WAL; small txns (≤500); pause toggle. |
| Codex offline for a day | SQL surfaces keep working; Radar shows `stale=true`; Ask returns "codex offline" fallback with raw FTS results. |
| LLM hallucinated citations | Structural — host assembles `[n]` markers from validated `cites` integers after each segment. Hallucinated ids never reach the UI. |
| Off-main-thread AppKit | UniFFI handler invocations wrapped through `dispatch_main` on sidecar side; serialized per handle; drop = cancel. |
| Ask starves classify | Weighted fair share instead of strict priority; per-class reserves. |
| v1 drop before v2 ready | v1 tables renamed, not dropped; `drop_v1_wiki` migration runs only on `v2_backfill_complete=1`. |
| JSON1 missing in sqlcipher amalgamation | Verify `json_extract` at v9 migration smoke test; fail fast with clear error. (Default on in 3.46.1.) |

---

## 14. Blockers vs deferrals

**Ship-blockers before v2 launch (phases 5-13)**:
- Swift Postbox delete observer (phase 2b, from v8 leftover)
- Exact codex model IDs pinned in settings (phase 5 seed)
- LLM output validators + prompt-injection boundary (phase 6)
- Citation validator for Ask (phase 9)
- Crash-safe backfill resume tested (phase 13)
- Global pause toggle wired (phase 12)

**Deferrals (post-launch follow-ups)**:
- Message edit → evidence re-classify (noted in §13)
- Per-chat Ask scope
- Cross-device encrypted export
- Per-chat trending tab
- Semantic/embedding retrieval for Ask
- Multi-user / shared wikis
- Per-chat privacy exclusions (can add later via settings without schema migration — `wiki_settings.excluded_chat_ids` JSON array)

---

## 15. Success criteria

- User opens wiki panel, sees Radar with concrete reasons — `42 evidence · 4 chats · 3×` + LLM hook — never a bare title with only a msg count.
- Resolved events are searchable via Library filter but don't crowd Radar/Digest.
- User types a query, presses ⌘↵, gets a streamed answer with citations that actually point to real messages in chat. No hallucinated `[n]`.
- Full message delete removes evidence rows and search hits in one Postbox tick (after phase 2b lands).
- Crash during 22h backfill resumes from last `status='pending'` row; no evidence double-counted.
- `cargo test` green across every phase commit.
- Radar stays useful when codex is offline for 12h — stale banner, no crash, still renders last-cached top-10.
