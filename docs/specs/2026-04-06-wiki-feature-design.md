# Telegram Wiki Feature — Design Spec

> **v3 — Updated after second Codex review. Additional fixes: rollup by message timestamp, channel membership table, citation-page mapping, queue crash recovery, defined total_active_channels, topic metadata reconciliation policy.**

## Problem

The user collects messages from 100K+ crypto/finance Telegram channels but has no way to surface trends, organize knowledge, or get summaries. Raw search isn't enough — they need auto-generated intelligence from the data.

## Solution

A **Wiki tab** inside the existing app that:
- Classifies every message into topics via a **decoupled background worker** (not inline with sync)
- Shows a **trending dashboard** of hot topics across all channels
- Generates **bilingual wiki articles** (Korean + English) per topic on demand, with **source citations**
- Categories use a **controlled taxonomy** (LLM maps to canonical set)
- Topics use **canonical IDs with aliases** (prevents fragmentation)
- Source messages viewable as collapsible section under each article

## Architecture: Decoupled Stream Processing

```
Message Collection (existing sync)
    │
    └─ After batch save → INSERT into wiki_classify_queue
       (just message IDs, no LLM call during sync)

Background Classification Worker (separate thread)
    │
    ├─ Poll wiki_classify_queue for pending messages
    │
    ├─ Each message → OpenAI API (gpt-4o-mini)
    │   Classify: topic, category, relevance
    │   Skip: greetings, spam, off-topic
    │   Concurrency: max 5 parallel API calls
    │   Rate limit: 200ms between calls
    │   Retry: 3 attempts with exponential backoff
    │
    ├─ Match to canonical topic (alias lookup) or create new
    │
    ├─ Link message → topic in DB
    │
    ├─ Update topic_stats_daily rollup
    │
    └─ Recalculate trending score for affected topic

When user views topic:
    │
    └─ If new messages since last summary
       → Regenerate bilingual wiki article via LLM
       → Include source citations [1] [2] [3]
       → Cache in wiki_pages table with input_hash
```

### Why Decoupled (not Inline)

- Sync is never blocked by LLM latency or API failures
- Worker has its own retry/backpressure logic
- Worker can be paused/resumed independently
- Failed classifications don't corrupt sync state
- Can process historical backlog without re-syncing

## Data Model

### wiki_topics

| Column | Type | Description |
|--------|------|-------------|
| topic_id | INTEGER PK AUTO | Canonical topic ID |
| title | TEXT NOT NULL UNIQUE | Canonical English title |
| title_ko | TEXT | Canonical Korean title |
| category_id | INTEGER FK | References wiki_categories |
| trending_score | REAL DEFAULT 0.0 | Computed score |
| message_count | INTEGER DEFAULT 0 | Total linked messages |
| channel_count | INTEGER DEFAULT 0 | Unique channels mentioning |
| first_seen_at | INTEGER | Unix ts of earliest message |
| last_seen_at | INTEGER | Unix ts of latest message |
| last_summary_at | INTEGER | When summary was last generated |
| created_at | TEXT DEFAULT now | |
| updated_at | TEXT DEFAULT now | |

### wiki_topic_aliases

Prevents topic fragmentation ("BTC ETF", "Bitcoin ETF Inflows", "US Spot ETF" → same topic).

| Column | Type | Description |
|--------|------|-------------|
| alias_id | INTEGER PK AUTO | |
| topic_id | INTEGER FK | Canonical topic |
| alias | TEXT NOT NULL UNIQUE | Normalized alias string |
| created_at | TEXT DEFAULT now | |

**Lookup flow**: Normalize LLM output → search aliases → if match, use that topic_id. If no match, check with LLM against top-5 similar aliases, then create new or merge.

**Topic metadata reconciliation policy**:
- **Title**: First-write-wins. The canonical title is set when the topic is created and never auto-changed. New variations become aliases.
- **Korean title**: First non-null write wins. If created without title_ko and a later classification provides one, it's set once.
- **Category**: Majority-vote with threshold. Track category assignments in wiki_topic_messages (add `assigned_category` column). If >60% of linked messages agree on a different category than the current one AND message_count > 10, update the canonical category. This prevents early misclassification from sticking forever while avoiding flip-flopping on small samples.

### wiki_categories

Controlled taxonomy. LLM maps free-form output to these.

| Column | Type | Description |
|--------|------|-------------|
| category_id | INTEGER PK AUTO | |
| name | TEXT NOT NULL UNIQUE | Display name (e.g., "DeFi") |
| name_ko | TEXT | Korean name (e.g., "디파이") |
| sort_order | INTEGER DEFAULT 0 | For UI pill ordering |

**Seed categories**: DeFi, Trading, L1/L2, NFT, Airdrop, Regulation, Macro, Scam Alert, Other

LLM returns free-form category → backend normalizes (lowercase, strip) → fuzzy match to wiki_categories.name → if no match, assign "Other" and log for review.

### wiki_topic_messages

| Column | Type | Description |
|--------|------|-------------|
| topic_id | INTEGER FK | Canonical topic |
| chat_id | INTEGER FK | |
| message_id | INTEGER FK | |
| relevance | REAL DEFAULT 1.0 | LLM-assigned 0-1 |
| assigned_category | TEXT | Category LLM assigned for this message (for majority-vote reconciliation) |
| PRIMARY KEY | (topic_id, chat_id, message_id) | |

**Multi-topic**: A message CAN belong to multiple topics (e.g., a message about "Solana DeFi airdrop" can link to both "Solana Ecosystem" and "Airdrop Season").

### wiki_pages

| Column | Type | Description |
|--------|------|-------------|
| page_id | INTEGER PK AUTO | |
| topic_id | INTEGER FK | |
| content_ko | TEXT NOT NULL | Korean markdown with citations |
| content_en | TEXT NOT NULL | English markdown with citations |
| source_count | INTEGER | Messages used for this version |
| source_hash | TEXT | Hash of input message IDs (cache key) |
| version | INTEGER NOT NULL | |
| created_at | TEXT DEFAULT now | |
| UNIQUE | (topic_id, version) | One version per topic |

**Cache invalidation**: When viewing a topic, compute hash of current linked message IDs. If it matches latest page's source_hash, serve cached. Otherwise regenerate.

### wiki_page_sources

Anchors citations to specific page versions. When article says [1], this table maps it to the exact message.

| Column | Type | Description |
|--------|------|-------------|
| page_id | INTEGER FK | References wiki_pages |
| citation_index | INTEGER NOT NULL | The [N] number in the article |
| chat_id | INTEGER | Source message |
| message_id | INTEGER | Source message |
| PRIMARY KEY | (page_id, citation_index) | |

**Why**: Without this, if topic membership changes between page versions, rendered `[1]` citations would point to wrong messages. This table freezes the citation mapping per page version.

### wiki_pages_fts

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS wiki_pages_fts USING fts5(
    content_ko, content_en,
    content='wiki_pages',
    tokenize='trigram case_sensitive 0'
);
```

### wiki_classify_queue

Durable job queue. Sync writes here; worker reads.

| Column | Type | Description |
|--------|------|-------------|
| chat_id | INTEGER | |
| message_id | INTEGER | |
| status | TEXT DEFAULT 'pending' | pending, processing, done, failed, skipped |
| attempts | INTEGER DEFAULT 0 | Retry counter |
| error | TEXT | Last error message |
| claimed_at | TEXT | When worker claimed this item (for crash recovery) |
| created_at | TEXT DEFAULT now | |
| processed_at | TEXT | |
| PRIMARY KEY | (chat_id, message_id) | |

**Index**: `idx_queue_status ON wiki_classify_queue(status, created_at)` for efficient polling.

**Crash recovery**: Worker sets `status='processing'` + `claimed_at=now` atomically when claiming items. On startup, any items with `status='processing'` AND `claimed_at < now - 5 minutes` are reset to `pending` (stale claim timeout). This prevents items stuck in `processing` forever after a crash.

**Atomic dequeue pattern**:
```sql
UPDATE wiki_classify_queue
SET status = 'processing', claimed_at = datetime('now')
WHERE (chat_id, message_id) IN (
    SELECT chat_id, message_id FROM wiki_classify_queue
    WHERE status = 'pending'
    ORDER BY created_at
    LIMIT 1
)
```

### topic_stats_daily

Rollup table for efficient trending calculation. No full table scans.

| Column | Type | Description |
|--------|------|-------------|
| topic_id | INTEGER FK | |
| date | TEXT | YYYY-MM-DD (**from message timestamp, not classification time**) |
| msg_count | INTEGER DEFAULT 0 | Messages from that day |
| PRIMARY KEY | (topic_id, date) | |

**IMPORTANT**: The `date` column uses the source message's `timestamp` field (converted to date), NOT the current date when classification runs. During backfill of historical messages, a message from March 15 rolls up to 2026-03-15, not today. This prevents old messages from appearing "hot" on the day they're processed.

**Updated**: On each message classification: `UPSERT INTO topic_stats_daily (topic_id, date, msg_count) VALUES (?, date(?, 'unixepoch'), 1) ON CONFLICT DO UPDATE SET msg_count = msg_count + 1` using the message's timestamp.

### topic_channel_membership

Tracks which channels contributed to which topics per day. Enables accurate unique channel counts without overcounting.

| Column | Type | Description |
|--------|------|-------------|
| topic_id | INTEGER FK | |
| date | TEXT | YYYY-MM-DD (from message timestamp) |
| chat_id | INTEGER FK | |
| PRIMARY KEY | (topic_id, date, chat_id) | |

**Updated**: On each classification: `INSERT OR IGNORE INTO topic_channel_membership`. The PK constraint naturally deduplicates. To get unique channels for a topic in the last 7 days: `SELECT COUNT(DISTINCT chat_id) FROM topic_channel_membership WHERE topic_id = ? AND date >= date('now', '-7 days')`.

## Trending Score Algorithm

```
score = velocity × recency × log2(message_count + 1) × channel_diversity

velocity     = msgs_24h / max(msgs_7d_daily_avg, 1)
                -- uses topic_stats_daily, no full scan
recency      = exp(-0.1 × hours_since_last_message)
channel_div  = unique_channels_7d / total_active_channels
```

**`total_active_channels` definition**: `SELECT COUNT(*) FROM chats WHERE is_excluded = 0`. This is the denominator — all non-excluded chats in the database. Cached in memory on worker startup and refreshed every 100 classifications (cheap query). If 0, default to 1 to avoid division by zero.

**`unique_channels_7d`**: `SELECT COUNT(DISTINCT chat_id) FROM topic_channel_membership WHERE topic_id = ? AND date >= date('now', '-7 days')`. Uses the dedicated membership table for accurate counts.

**Computed from rollup tables** — `SELECT SUM(msg_count) FROM topic_stats_daily WHERE topic_id = ? AND date >= date(?, 'unixepoch', '-1 day')` etc. O(7) rows per topic, not O(N) messages.

- Topics mentioned across many channels rank higher (cross-channel signal)
- Velocity detects spikes (today's volume vs weekly average)
- Recency ensures stale topics fade
- Recalculated after each classification batch (not per-message)

## LLM Prompts

### Per-Message Classification

```
System: You are a crypto/finance message classifier for a Telegram archive.
Classify the message into one or more topics. Return ONLY valid JSON.

Rules:
- topics: array of 1-3 topics this message relates to
- Each topic: concise English title (e.g., "ETH Layer 2 Fees", "Solana Outage")
- topic_ko: Korean title if inferrable, else null
- category: one of [DeFi, Trading, L1/L2, NFT, Airdrop, Regulation, Macro, Scam Alert, Other]
- relevance: 0.0-1.0 how relevant the message is to each topic
- skip: true if message is greeting, spam, bot command, emoji-only, or has no informational value

User: [Channel: {chat_title}] [{timestamp}]
{text_plain}

Response format:
{
  "skip": false,
  "topics": [
    {"topic": "ETH Layer 2 Fees", "topic_ko": "이더리움 L2 수수료", "category": "DeFi", "relevance": 0.9},
    {"topic": "Ethereum Ecosystem", "topic_ko": "이더리움 생태계", "category": "L1/L2", "relevance": 0.5}
  ]
}

If skip=true, topics array should be empty.
```

**Changes from v1**: Multi-topic per message, constrained category set, explicit skip format.

### Summary Generation (on-demand, with citations)

```
System: Write a bilingual wiki article about a crypto/finance topic based on Telegram messages.
Every factual claim MUST cite its source using [N] notation matching the message index.

Structure:
## 요약
(Korean summary: 2-3 paragraphs, factual, cite sources as [1], [2], etc.)

### 핵심 포인트
- (Korean bullet points with citations)

### 타임라인
- (Korean chronological events with citations)

---

## Summary
(English version of the same content with same citations)

### Key Points
- (English bullet points with citations)

### Timeline
- (English chronological events with citations)

Rules:
- EVERY factual claim must have a [N] citation to a source message
- If sources disagree, note the disagreement: "According to [3], X, but [7] claims Y"
- If information is unverified or speculative, mark it as such
- Skip duplicate forwarded messages (same text from different channels)
- If fewer than 3 unique source messages, output "Insufficient sources for wiki article"
- Include specific numbers, prices, dates, and names mentioned
- Keep it concise — reference article, not a blog post

User:
Topic: {title}
Category: {category}

Source messages ({count} total, showing top {n} by relevance):
[1] [{timestamp}] [{chat_title}]: {text_plain}
[2] [{timestamp}] [{chat_title}]: {text_plain}
...
```

**Changes from v1**: Mandatory citations, disagreement handling, duplicate filtering, minimum source threshold.

### Topic Deduplication (Alias-Based)

When LLM returns a topic title:
1. Normalize: lowercase, strip whitespace, remove common suffixes ("update", "news")
2. Search `wiki_topic_aliases` for exact match on normalized form
3. If match → use that canonical topic_id
4. If no match → search top-5 similar aliases (Levenshtein or trigram similarity)
5. If similar found → LLM check: "Is '{new}' the same topic as '{existing}'? Context: both are crypto topics. Answer JSON: {same: bool, confidence: 0-1}"
6. If same with confidence > 0.7 → add as new alias to existing topic
7. Otherwise → create new canonical topic + alias

## UI Design

### Tab Navigation

Add a `TabBar` component at the top of the app (when authenticated):
- **Search** tab (existing functionality)
- **Wiki** tab (new)

### Wiki Tab — Trending Dashboard (Landing)

```
┌──────────────────────────────────────┐
│ [Search wiki...]                     │
│                                      │
│ Categories: [All] [DeFi] [Trading]   │
│ [L1/L2] [NFT] [Airdrop] ...         │
│                                      │
│ 🔥 Trending                          │
│ ┌──────────────────────────────────┐ │
│ │ 1. ETH Layer 2 Fees      ↑ 85%  │ │
│ │    DeFi • 142 msgs • 12 channels │ │
│ │    Updated 2h ago                │ │
│ ├──────────────────────────────────┤ │
│ │ 2. Solana Outage          ↑ 62%  │ │
│ │    L1/L2 • 89 msgs • 8 channels │ │
│ │    Updated 5h ago                │ │
│ ├──────────────────────────────────┤ │
│ │ 3. BTC ETF Inflows        ↑ 45%  │ │
│ │    Trading • 201 msgs • 15 ch   │ │
│ │    Updated 1h ago                │ │
│ └──────────────────────────────────┘ │
│                                      │
│ ⚙ Processing: 94,521/100,000 msgs   │
│   Status: Classifying...             │
└──────────────────────────────────────┘
```

### Wiki Tab — Article View (click topic)

```
┌──────────────────────────────────────┐
│ ← Back to Trending                   │
│                                      │
│ # ETH Layer 2 Fees                   │
│ DeFi • 142 sources • Updated 2h ago  │
│                                      │
│ ## 요약                               │
│ 이더리움 L2 수수료가 덴쿤 업그레이드   │
│ 이후 크게 하락했습니다 [1][3].         │
│                                      │
│ ### 핵심 포인트                        │
│ - Base 수수료: 평균 0.001 gwei [2]    │
│ - Arbitrum: 트랜잭션당 $0.02 [5]      │
│                                      │
│ ---                                  │
│                                      │
│ ## Summary                           │
│ Ethereum L2 fees have dropped        │
│ significantly following the Dencun   │
│ upgrade [1][3]...                    │
│                                      │
│ ### Key Points                       │
│ - Base fees: 0.001 gwei average [2]  │
│ - Arbitrum: $0.02 per transaction [5]│
│                                      │
│ ▼ View 142 source messages           │
│ ┌──────────────────────────────────┐ │
│ │ [1] [CryptoKR] Apr 3, 14:22     │ │
│ │ "아니 L2 가스비 진짜 없어졌네"    │ │
│ │                                  │ │
│ │ [2] [DefiAlpha] Apr 3, 13:01    │ │
│ │ "Base fees are insanely low rn"  │ │
│ └──────────────────────────────────┘ │
└──────────────────────────────────────┘
```

### Wiki Settings (collapsible panel)

```
┌──────────────────────────────────────┐
│ ⚙ Wiki Settings                      │
│                                      │
│ OpenAI API Key: [••••••••••sk-xxxx]  │
│ Model: gpt-4o-mini                   │
│ [Validate Key]                       │
│                                      │
│ Processing Status: Idle              │
│ Queue: 0 pending • 94,521 done       │
│ Topics generated: 347                │
│ Last run: 2h ago                     │
│                                      │
│ [Reprocess All]  [Clear Wiki Data]   │
└──────────────────────────────────────┘
```

## Backend Modules

### New: `src-tauri/src/wiki/`

| File | Responsibility |
|------|---------------|
| `mod.rs` | Module root, re-exports |
| `llm.rs` | OpenAI HTTP client (`LlmClient`) — classify, summarize, dedup. Retry logic, rate limiting, JSON parsing with fallback. |
| `worker.rs` | Background classification worker — polls queue, processes messages, updates topics. Runs on dedicated thread with tokio runtime. |
| `trending.rs` | Trending score calculation from rollup table (pure function + Store query) |

### New store modules: `src-tauri/src/store/`

| File | Responsibility |
|------|---------------|
| `wiki_topic.rs` | CRUD for wiki_topics, wiki_topic_aliases, wiki_topic_messages. Topic metadata reconciliation. |
| `wiki_page.rs` | CRUD for wiki_pages + wiki_page_sources + FTS5 indexing + source_hash cache check |
| `wiki_queue.rs` | Classify queue operations (enqueue, atomic dequeue, mark done/failed, stale claim recovery) |
| `wiki_stats.rs` | topic_stats_daily + topic_channel_membership UPSERT + trending queries |
| `wiki_category.rs` | Category CRUD + normalization mapping |

### New Tauri Commands

**Settings:**
- `save_openai_api_key(key)` — stores in macOS Keychain (via existing security/keychain.rs)
- `get_openai_api_key()` — retrieves (masked for display)
- `validate_openai_api_key(key)` — test call to OpenAI, returns success/error

**Worker Control:**
- `get_wiki_status()` → WikiStatus { queue_pending, queue_done, queue_failed, topics_count, is_running, last_run_at }
- `start_wiki_worker()` — starts background worker if not running
- `stop_wiki_worker()` — gracefully stops worker
- `reprocess_wiki()` — clears queue + processed state, re-enqueues all messages
- `clear_wiki_data()` — drops all wiki table content

**Browsing:**
- `get_trending_topics(limit, offset, category_id?)` → Vec<WikiTopic>
- `get_wiki_categories()` → Vec<WikiCategory>
- `get_topic_detail(topic_id)` → WikiTopicDetail (topic + latest page + source count + aliases)
- `get_topic_sources(topic_id, limit, offset)` → Vec<MessageWithChat>
- `search_wiki(query, limit)` → WikiSearchResult (searches topic titles + page FTS5)

**Events:**
- `wiki-worker-progress`: { processed, total, current_topic, queue_remaining }
- `wiki-worker-error`: { message, recoverable, queue_item }
- `wiki-worker-stopped`: { reason }

### Integration with Existing Collection

In `commands.rs` `start_collection()`, after each message batch is saved to DB:
1. Enqueue new message IDs into `wiki_classify_queue` (simple INSERT, no LLM call)
2. If wiki worker is not running and API key exists → auto-start worker

The worker is **fully decoupled** — it reads from the queue independently. Sync completes normally regardless of wiki processing state.

**Error handling**: If OpenAI API is down, worker marks items as `failed` with attempt count. After 3 failures, items stay in `failed` state. User can retry failed items from settings.

## Frontend Components

### New Files

```
src/
  components/
    TabBar.tsx              — Search/Wiki tab switcher
    wiki/
      TrendingDashboard.tsx — Landing: trending list + category filter
      TopicCard.tsx         — Individual topic card in trending list
      WikiArticle.tsx       — Full article view with bilingual content + citations
      SourceMessages.tsx    — Collapsible source message list (numbered for citations)
      CategoryFilter.tsx    — Category pill selector
      WikiSearch.tsx        — Wiki search bar + results
      WikiSettings.tsx      — API key (keychain), status, worker controls
  hooks/
    useWiki.ts             — Topic browsing, selection, categories
    useWikiWorker.ts       — Worker status, progress events, controls
  pages/
    WikiPage.tsx           — Main wiki container (routes between dashboard/article)
```

### Modified Files

- `src/App.tsx` — Add TabBar, conditional render WikiPage
- `src/App.css` — Wiki-specific styles (dark theme consistent)
- `src/api/tauri.ts` — Add wiki command wrappers + event listeners
- `src/types/index.ts` — Add wiki types

## New Dependencies

**Rust (Cargo.toml):**
- `reqwest = { version = "0.12", features = ["json"] }` — HTTP client for OpenAI API
- `sha2 = "0.10"` — For source_hash computation

**Frontend (package.json):**
- `react-markdown` — Render wiki article markdown

## Cost Model

| Operation | Cost per unit (gpt-4o-mini) | Token estimate | 100K messages |
|-----------|---------------------------|----------------|---------------|
| Per-message classify | ~150 input + 80 output tokens | ~$0.0001 | ~$10 |
| Summary generation | ~2000 input + 500 output | ~$0.005/topic | ~$1.75 (350 topics) |
| Topic dedup check | ~200 input + 20 output | ~$0.0001 | ~$0.50 |
| Retries (~5% fail rate) | | | ~$0.60 |
| **Total initial** | | | **~$13** |
| **Daily ongoing** (1K msgs) | | | **~$0.15** |

## Edge Cases Handled

1. **API down during processing**: Worker retries 3x with backoff, then marks failed. Sync unaffected.
2. **Duplicate forwarded messages**: Summary prompt instructs LLM to skip duplicates. Source dedup by text_plain hash before sending to LLM.
3. **Empty/system messages**: Classifier returns skip=true. Queue marks as skipped.
4. **Topic fragmentation**: Alias system catches "BTC ETF" / "Bitcoin ETF" / "비트코인 ETF". LLM dedup as fallback.
5. **Category drift**: Constrained to seed set. Unknown categories map to "Other". Majority-vote reconciliation for topic categories.
6. **Stale summaries**: source_hash cache invalidation. Only regenerate when source set changes.
7. **Large backlog (100K+)**: Queue-based, processes at own pace. Progress bar in UI. Concurrency capped at 5. Rollup uses message timestamp, not classification time — no false trending during backfill.
8. **API key validation**: Test call before saving. Invalid key rejected with error message.
9. **Edited/deleted messages**: Not currently tracked by collector. V2 concern.
10. **Worker crash mid-processing**: Stale claim timeout (5 min). On startup, reset items with `status='processing'` and `claimed_at < now - 5min` back to `pending`.
11. **Citation stability**: wiki_page_sources freezes the exact message-to-citation mapping per page version. Even if topic membership changes, old pages render correctly.

## Open Questions (Deferred to v2)

1. **Topic merging UI**: Manual merge of duplicate topics
2. **Topic pinning**: Pin topics to dashboard
3. **Export**: Export wiki as markdown files
4. **Notifications**: Alert when a topic spikes
5. **Media/files**: Process images, documents attached to messages
6. **Edit tracking**: Detect message edits and update topics

## Implementation Order

1. Schema migration v4 (all new tables + seed categories)
2. Store layer: wiki_queue, wiki_category
3. Store layer: wiki_topic (with aliases), wiki_topic_messages
4. Store layer: wiki_page, wiki_stats
5. LLM client (OpenAI HTTP, classify + summarize + dedup)
6. Background worker (queue consumer, classification pipeline)
7. Trending calculation (from rollup table)
8. Tauri commands (settings, worker control, browsing, search)
9. Collection integration (enqueue after sync)
10. Frontend: types, API functions, TabBar, WikiPage shell
11. Frontend: TrendingDashboard + TopicCard + CategoryFilter
12. Frontend: WikiArticle + SourceMessages (with citations)
13. Frontend: WikiSettings + worker progress
14. Frontend: WikiSearch
15. Polish: loading states, errors, empty states, dark theme consistency
