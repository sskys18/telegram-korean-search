use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPage {
    pub page_id: i64,
    pub topic_id: i64,
    pub content_ko: String,
    pub content_en: String,
    pub source_count: Option<i64>,
    pub source_hash: Option<String>,
    pub version: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageSource {
    pub citation_index: i64,
    pub chat_id: i64,
    pub message_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiPageSearchResult {
    pub topic_id: i64,
    pub topic_title: String,
    pub snippet: String,
}

impl Store {
    pub fn insert_wiki_page(
        &self,
        topic_id: i64,
        content_ko: &str,
        content_en: &str,
        sources: &[(i64, i64)],
    ) -> Result<i64, sqlite::Error> {
        let version = {
            let mut stmt = self.conn().prepare(
                "SELECT COALESCE(MAX(version), 0) + 1 FROM wiki_pages WHERE topic_id = ?",
            )?;
            stmt.bind((1, topic_id))?;
            stmt.next()?;
            stmt.read::<i64, _>(0)?
        };

        let source_hash = compute_source_hash(sources);

        let mut stmt = self.conn().prepare(
            "INSERT INTO wiki_pages (topic_id, content_ko, content_en, source_count, source_hash, version)
             VALUES (?, ?, ?, ?, ?, ?)",
        )?;
        stmt.bind((1, topic_id))?;
        stmt.bind((2, content_ko))?;
        stmt.bind((3, content_en))?;
        stmt.bind((4, sources.len() as i64))?;
        stmt.bind((5, source_hash.as_str()))?;
        stmt.bind((6, version))?;
        stmt.next()?;

        let page_id = self.last_insert_rowid()?;

        let mut fts_stmt = self.conn().prepare(
            "INSERT INTO wiki_pages_fts (rowid, content_ko, content_en) VALUES (?, ?, ?)",
        )?;
        fts_stmt.bind((1, page_id))?;
        fts_stmt.bind((2, content_ko))?;
        fts_stmt.bind((3, content_en))?;
        fts_stmt.next()?;

        let mut src_stmt = self.conn().prepare(
            "INSERT INTO wiki_page_sources (page_id, citation_index, chat_id, message_id)
             VALUES (?, ?, ?, ?)",
        )?;
        for (i, &(chat_id, message_id)) in sources.iter().enumerate() {
            src_stmt.bind((1, page_id))?;
            src_stmt.bind((2, (i + 1) as i64))?;
            src_stmt.bind((3, chat_id))?;
            src_stmt.bind((4, message_id))?;
            src_stmt.next()?;
            src_stmt.reset()?;
        }

        self.conn().execute(format!(
            "UPDATE wiki_topics SET last_summary_at = strftime('%s', 'now') WHERE topic_id = {}",
            topic_id
        ))?;

        Ok(page_id)
    }

    pub fn get_latest_page(&self, topic_id: i64) -> Result<Option<WikiPage>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT page_id, topic_id, content_ko, content_en, source_count, source_hash, version, created_at
             FROM wiki_pages WHERE topic_id = ? ORDER BY version DESC LIMIT 1",
        )?;
        stmt.bind((1, topic_id))?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(Some(WikiPage {
                page_id: stmt.read::<i64, _>("page_id")?,
                topic_id: stmt.read::<i64, _>("topic_id")?,
                content_ko: stmt.read::<String, _>("content_ko")?,
                content_en: stmt.read::<String, _>("content_en")?,
                source_count: stmt.read::<Option<i64>, _>("source_count")?,
                source_hash: stmt.read::<Option<String>, _>("source_hash")?,
                version: stmt.read::<i64, _>("version")?,
                created_at: stmt.read::<String, _>("created_at")?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_page_sources(&self, page_id: i64) -> Result<Vec<PageSource>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT citation_index, chat_id, message_id FROM wiki_page_sources
             WHERE page_id = ? ORDER BY citation_index",
        )?;
        stmt.bind((1, page_id))?;
        let mut sources = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            sources.push(PageSource {
                citation_index: stmt.read::<i64, _>("citation_index")?,
                chat_id: stmt.read::<i64, _>("chat_id")?,
                message_id: stmt.read::<i64, _>("message_id")?,
            });
        }
        Ok(sources)
    }

    pub fn needs_regeneration(&self, topic_id: i64) -> Result<bool, sqlite::Error> {
        let page = self.get_latest_page(topic_id)?;
        match page {
            None => Ok(true),
            Some(p) => {
                let mut stmt = self.conn().prepare(
                    "SELECT chat_id, message_id FROM wiki_topic_messages
                     WHERE topic_id = ? ORDER BY chat_id, message_id",
                )?;
                stmt.bind((1, topic_id))?;
                let mut sources = Vec::new();
                while let sqlite::State::Row = stmt.next()? {
                    sources.push((
                        stmt.read::<i64, _>("chat_id")?,
                        stmt.read::<i64, _>("message_id")?,
                    ));
                }
                let current_hash = compute_source_hash(&sources);
                Ok(p.source_hash.as_deref() != Some(current_hash.as_str()))
            }
        }
    }

    pub fn search_wiki_pages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<WikiPageSearchResult>, sqlite::Error> {
        if query.len() < 3 {
            return Ok(Vec::new());
        }
        let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
        let mut stmt = self.conn().prepare(format!(
            "SELECT wp.topic_id, wt.title, snippet(wiki_pages_fts, 0, '<b>', '</b>', '...', 32) as snippet
             FROM wiki_pages_fts fts
             JOIN wiki_pages wp ON wp.page_id = fts.rowid
             JOIN wiki_topics wt ON wt.topic_id = wp.topic_id
             WHERE wiki_pages_fts MATCH ?
             GROUP BY wp.topic_id
             LIMIT {}",
            limit
        ))?;
        stmt.bind((1, fts_query.as_str()))?;
        let mut results = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            results.push(WikiPageSearchResult {
                topic_id: stmt.read::<i64, _>("topic_id")?,
                topic_title: stmt.read::<String, _>("title")?,
                snippet: stmt.read::<String, _>("snippet")?,
            });
        }
        Ok(results)
    }

    pub fn clear_wiki_pages(&self) -> Result<(), sqlite::Error> {
        self.conn().execute("DELETE FROM wiki_page_sources")?;
        self.conn().execute("DELETE FROM wiki_pages")?;
        self.conn()
            .execute("INSERT INTO wiki_pages_fts(wiki_pages_fts) VALUES('rebuild')")?;
        Ok(())
    }
}

/// PageRef: returned by classify validator after dedup_or_insert.
#[derive(Debug, Clone)]
pub struct PageRefV2 {
    pub id: i64,
    pub state: String,
    pub kind: String,
}

#[derive(Debug, Clone)]
pub struct CandidatePage {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct EvidenceForRewrite {
    pub id: i64,
    pub msg_id: i64,
    pub chat_id: i64,
    pub ts: i64,
    pub excerpt: String,
    pub salience: f64,
    pub cited: i64,
}

#[derive(Debug, Clone)]
pub struct PageForRewrite {
    pub id: i64,
    pub kind: String,
    pub title: String,
    pub state: String,
    pub summary_md: String,
    pub facts: Option<String>,
    pub evidence_count: i64,
    pub last_rewrite_at: Option<i64>,
    pub last_rewrite_evidence_count: i64,
    pub last_rewrite_max_evidence_id: i64,
}

#[derive(Debug, Clone)]
pub struct NewEvidenceV2<'a> {
    pub page_id: i64,
    pub msg_id: i64,
    pub chat_id: i64,
    pub sender_id: i64,
    pub ts: i64,
    pub excerpt: &'a str,
    pub salience: f64,
}

// ---- Phase 8 trending (spec §6.4) -----------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendingWindow {
    H1,
    H24,
    D7,
}

impl TrendingWindow {
    pub fn label(self) -> &'static str {
        match self {
            Self::H1 => "1h",
            Self::H24 => "24h",
            Self::D7 => "7d",
        }
    }
    pub fn span_secs(self) -> i64 {
        match self {
            Self::H1 => 3_600,
            Self::H24 => 86_400,
            Self::D7 => 7 * 86_400,
        }
    }
    /// Spec §6.4 "minimum refresh gap": 5 min for 1h+24h, 1h for 7d.
    pub fn min_refresh_gap_secs(self) -> i64 {
        match self {
            Self::H1 | Self::H24 => 300,
            Self::D7 => 3_600,
        }
    }
    pub fn all() -> [Self; 3] {
        [Self::H1, Self::H24, Self::D7]
    }
    pub fn from_label(s: &str) -> Option<Self> {
        match s {
            "1h" => Some(Self::H1),
            "24h" => Some(Self::H24),
            "7d" => Some(Self::D7),
            _ => None,
        }
    }
}

/// Captured once per refresh tick; threaded through every store call so
/// post-snapshot inserts can never silently leak in. Mirrors the same
/// fix phase 7 applied to rewrite delta selection.
#[derive(Debug, Clone, Copy)]
pub struct TrendingSnapshot {
    pub window: TrendingWindow,
    pub window_start: i64,
    pub prior_start: i64,
    pub now: i64,
    pub max_evidence_id: i64,
}

#[derive(Debug, Clone)]
pub struct TrendingCandidate {
    pub page_id: i64,
    pub kind: String,
    pub title: String,
    pub created_at: i64,
    pub ec: i64,
    pub chats: i64,
    pub senders: i64,
    pub last_ts: i64,
    pub prior_ec: i64,
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct TrendingApplyRow {
    pub page_id: i64,
    pub rank: i64,
    pub hook: String,
    pub reason_code: String,
    pub reason_metrics: String,
    pub sparkline: String,
}

impl Store {
    /// Dedup by `title_norm`, then alias hits, otherwise insert a v2 page.
    /// Must be called inside the caller's transaction.
    pub fn dedup_or_insert_page_v2(
        &self,
        kind: &str,
        title: &str,
        aliases: &[String],
    ) -> Result<PageRefV2, sqlite::Error> {
        use crate::wiki::norm::{nfc, title_norm};

        let title_n = title_norm(title);
        let now = crate::wiki::norm::unix_now();

        let mut existing_id = {
            let mut s = self
                .conn()
                .prepare("SELECT id FROM wiki_pages_v2 WHERE title_norm = ?")?;
            s.bind((1, title_n.as_str()))?;
            if let sqlite::State::Row = s.next()? {
                Some(s.read::<i64, _>(0)?)
            } else {
                None
            }
        };

        if existing_id.is_none() && !aliases.is_empty() {
            let mut alias_norms: Vec<String> = aliases
                .iter()
                .map(|a| title_norm(a))
                .filter(|a| !a.is_empty())
                .collect();
            alias_norms.push(title_n.clone());
            alias_norms.sort();
            alias_norms.dedup();

            let placeholders = alias_norms
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            let q = format!(
                "SELECT page_id, COUNT(*) AS hits
                   FROM wiki_page_aliases
                  WHERE alias_norm IN ({})
                  GROUP BY page_id
                  ORDER BY hits DESC, page_id
                  LIMIT 1",
                placeholders
            );
            let mut s = self.conn().prepare(q)?;
            for (i, a) in alias_norms.iter().enumerate() {
                s.bind((i + 1, a.as_str()))?;
            }
            if let sqlite::State::Row = s.next()? {
                existing_id = Some(s.read::<i64, _>("page_id")?);
            }
        }

        let page_id = match existing_id {
            Some(id) => id,
            None => {
                let mut s = self.conn().prepare(
                    "INSERT INTO wiki_pages_v2
                        (kind, title, title_norm, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?)",
                )?;
                s.bind((1, kind))?;
                s.bind((2, nfc(title).as_str()))?;
                s.bind((3, title_n.as_str()))?;
                s.bind((4, now))?;
                s.bind((5, now))?;
                s.next()?;
                self.last_insert_rowid()?
            }
        };

        let mut alias_stmt = self.conn().prepare(
            "INSERT OR IGNORE INTO wiki_page_aliases (page_id, alias_norm, alias_raw)
             VALUES (?, ?, ?)",
        )?;
        for a in aliases {
            let an = title_norm(a);
            if an.is_empty() {
                continue;
            }
            alias_stmt.bind((1, page_id))?;
            alias_stmt.bind((2, an.as_str()))?;
            alias_stmt.bind((3, nfc(a).as_str()))?;
            alias_stmt.next()?;
            alias_stmt.reset()?;
        }
        alias_stmt.bind((1, page_id))?;
        alias_stmt.bind((2, title_n.as_str()))?;
        alias_stmt.bind((3, nfc(title).as_str()))?;
        alias_stmt.next()?;

        self.refresh_pages_index(page_id)?;

        let mut s = self
            .conn()
            .prepare("SELECT state, kind FROM wiki_pages_v2 WHERE id = ?")?;
        s.bind((1, page_id))?;
        s.next()?;
        let state = s.read::<String, _>("state")?;
        let kind_out = s.read::<String, _>("kind")?;

        Ok(PageRefV2 {
            id: page_id,
            state,
            kind: kind_out,
        })
    }

    /// Rebuild `wiki_pages_index` and `pages_fts` for one page.
    /// Must be called inside the caller's transaction.
    pub fn refresh_pages_index(&self, page_id: i64) -> Result<(), sqlite::Error> {
        use crate::search::hangul::decompose_jamo;

        let (title, summary_md): (String, String) = {
            let mut s = self
                .conn()
                .prepare("SELECT title, summary_md FROM wiki_pages_v2 WHERE id = ?")?;
            s.bind((1, page_id))?;
            s.next()?;
            (s.read::<String, _>(0)?, s.read::<String, _>(1)?)
        };
        let aliases = {
            let mut s = self.conn().prepare(
                "SELECT alias_raw FROM wiki_page_aliases WHERE page_id = ? ORDER BY alias_norm",
            )?;
            s.bind((1, page_id))?;
            let mut parts = Vec::new();
            while let sqlite::State::Row = s.next()? {
                parts.push(s.read::<String, _>(0)?);
            }
            parts.join(" ")
        };
        let title_jamo = decompose_jamo(&title);
        let aliases_jamo = decompose_jamo(&aliases);
        let summary_jamo = decompose_jamo(&summary_md);

        self.conn()
            .execute(format!("DELETE FROM pages_fts WHERE rowid = {}", page_id))?;
        self.conn().execute(format!(
            "DELETE FROM wiki_pages_index WHERE page_id = {}",
            page_id
        ))?;

        let mut ins = self.conn().prepare(
            "INSERT INTO wiki_pages_index
                (page_id, title, aliases, summary_md, title_jamo, aliases_jamo, summary_jamo)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )?;
        ins.bind((1, page_id))?;
        ins.bind((2, title.as_str()))?;
        ins.bind((3, aliases.as_str()))?;
        ins.bind((4, summary_md.as_str()))?;
        ins.bind((5, title_jamo.as_str()))?;
        ins.bind((6, aliases_jamo.as_str()))?;
        ins.bind((7, summary_jamo.as_str()))?;
        ins.next()?;

        let mut fts = self.conn().prepare(
            "INSERT INTO pages_fts
                (rowid, title, aliases, summary_md, title_jamo, aliases_jamo, summary_jamo)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )?;
        fts.bind((1, page_id))?;
        fts.bind((2, title.as_str()))?;
        fts.bind((3, aliases.as_str()))?;
        fts.bind((4, summary_md.as_str()))?;
        fts.bind((5, title_jamo.as_str()))?;
        fts.bind((6, aliases_jamo.as_str()))?;
        fts.bind((7, summary_jamo.as_str()))?;
        fts.next()?;
        Ok(())
    }

    /// Insert evidence row, bump page counters, and insert `evidence_fts`.
    /// Returns `None` on duplicate `(page_id,msg_id,chat_id)`.
    /// Must be called inside the caller's transaction.
    pub fn insert_evidence_v2(
        &self,
        evidence: &NewEvidenceV2<'_>,
    ) -> Result<Option<i64>, sqlite::Error> {
        use crate::search::hangul::decompose_jamo;
        use crate::wiki::norm::{evidence_source_hash, nfc};

        let excerpt_nfc = nfc(evidence.excerpt);
        let excerpt_jamo = decompose_jamo(&excerpt_nfc);
        let source_hash = evidence_source_hash(
            evidence.page_id,
            evidence.msg_id,
            evidence.chat_id,
            &excerpt_nfc,
        );
        let now = crate::wiki::norm::unix_now();

        {
            let mut s = self.conn().prepare(
                "SELECT 1 FROM wiki_evidence WHERE page_id = ? AND msg_id = ? AND chat_id = ?",
            )?;
            s.bind((1, evidence.page_id))?;
            s.bind((2, evidence.msg_id))?;
            s.bind((3, evidence.chat_id))?;
            if let sqlite::State::Row = s.next()? {
                return Ok(None);
            }
        }

        let mut ins = self.conn().prepare(
            "INSERT INTO wiki_evidence
                (page_id, msg_id, chat_id, sender_id, ts,
                 excerpt, excerpt_jamo, source_hash, salience, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )?;
        ins.bind((1, evidence.page_id))?;
        ins.bind((2, evidence.msg_id))?;
        ins.bind((3, evidence.chat_id))?;
        ins.bind((4, evidence.sender_id))?;
        ins.bind((5, evidence.ts))?;
        ins.bind((6, excerpt_nfc.as_str()))?;
        ins.bind((7, excerpt_jamo.as_str()))?;
        ins.bind((8, source_hash.as_slice()))?;
        ins.bind((9, evidence.salience))?;
        ins.bind((10, now))?;
        ins.next()?;
        let evid_id = self.last_insert_rowid()?;

        let mut bump = self.conn().prepare(
            "UPDATE wiki_pages_v2
                SET evidence_count = evidence_count + 1,
                    last_evidence_at = MAX(COALESCE(last_evidence_at, 0), ?),
                    updated_at = ?
              WHERE id = ?",
        )?;
        bump.bind((1, evidence.ts))?;
        bump.bind((2, now))?;
        bump.bind((3, evidence.page_id))?;
        bump.next()?;

        let mut fts = self
            .conn()
            .prepare("INSERT INTO evidence_fts (rowid, excerpt, excerpt_jamo) VALUES (?, ?, ?)")?;
        fts.bind((1, evid_id))?;
        fts.bind((2, excerpt_nfc.as_str()))?;
        fts.bind((3, excerpt_jamo.as_str()))?;
        fts.next()?;
        Ok(Some(evid_id))
    }

    /// Build candidates per spec §6.2: alias-direct first, then FTS fill.
    pub fn classify_candidates_v2(
        &self,
        normalized_tokens: &[String],
        fts_query: &str,
        cap: usize,
    ) -> Result<Vec<CandidatePage>, sqlite::Error> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();

        if !normalized_tokens.is_empty() {
            let placeholders = normalized_tokens
                .iter()
                .map(|_| "?")
                .collect::<Vec<_>>()
                .join(",");
            let q = format!(
                "SELECT DISTINCT a.page_id
                   FROM wiki_page_aliases a
                   JOIN wiki_pages_v2 p ON p.id = a.page_id
                  WHERE a.alias_norm IN ({})
                    AND p.state IN ('active','resolved')",
                placeholders
            );
            let mut s = self.conn().prepare(q)?;
            for (i, t) in normalized_tokens.iter().enumerate() {
                s.bind((i + 1, t.as_str()))?;
            }
            while let sqlite::State::Row = s.next()? {
                let id = s.read::<i64, _>(0)?;
                if seen.insert(id) {
                    out.push(self.load_candidate(id)?);
                    if out.len() >= cap {
                        return Ok(out);
                    }
                }
            }
        }

        if !fts_query.trim().is_empty() {
            let mut s = self.conn().prepare(
                "SELECT p.id
                   FROM pages_fts f
                   JOIN wiki_pages_v2 p ON p.id = f.rowid
                  WHERE pages_fts MATCH ?
                    AND p.state IN ('active','resolved')
                  ORDER BY bm25(pages_fts)
                  LIMIT 30",
            )?;
            s.bind((1, fts_query))?;
            while let sqlite::State::Row = s.next()? {
                let id = s.read::<i64, _>(0)?;
                if seen.insert(id) {
                    out.push(self.load_candidate(id)?);
                    if out.len() >= cap {
                        break;
                    }
                }
            }
        }

        Ok(out)
    }

    fn load_candidate(&self, page_id: i64) -> Result<CandidatePage, sqlite::Error> {
        let (kind, title) = {
            let mut s = self
                .conn()
                .prepare("SELECT kind, title FROM wiki_pages_v2 WHERE id = ?")?;
            s.bind((1, page_id))?;
            s.next()?;
            (s.read::<String, _>(0)?, s.read::<String, _>(1)?)
        };
        let mut aliases = Vec::new();
        let mut s = self.conn().prepare(
            "SELECT alias_raw FROM wiki_page_aliases WHERE page_id = ? ORDER BY alias_norm",
        )?;
        s.bind((1, page_id))?;
        while let sqlite::State::Row = s.next()? {
            aliases.push(s.read::<String, _>(0)?);
        }
        Ok(CandidatePage {
            id: page_id,
            kind,
            title,
            aliases,
        })
    }
}

/// Validated rewrite payload to apply in a single txn.
pub struct RewriteApply<'a> {
    pub page_id: i64,
    pub summary_md: &'a str,
    pub facts_json: &'a str,
    pub state: &'a str,
    pub new_aliases: &'a [String],
    pub retention_cap: i64,
    /// Wall-clock snapshot at the start of `select_rewrite_evidence`
    /// (stored as `last_rewrite_at`, drives the 24h trigger fallback).
    pub snapshot_at: i64,
    /// MAX(wiki_evidence.id) at the snapshot — this is the real
    /// delta watermark; using id avoids the same-second clock race
    /// that comes with `created_at`.
    pub max_evidence_id: i64,
}

impl Store {
    /// Spec §6.3 trigger:
    ///   evidence_count - last_rewrite_evidence_count >= 20
    ///   OR (last_rewrite_at IS NULL AND evidence_count > 0)
    ///   OR (now - last_rewrite_at >= 86400 AND evidence_count > last_rewrite_evidence_count)
    pub fn maybe_enqueue_rewrite(&self, page_id: i64) -> Result<bool, sqlite::Error> {
        let mut s = self.conn().prepare(
            "SELECT evidence_count, last_rewrite_evidence_count, last_rewrite_at
               FROM wiki_pages_v2 WHERE id = ?",
        )?;
        s.bind((1, page_id))?;
        if let sqlite::State::Row = s.next()? {
            let ec: i64 = s.read::<i64, _>(0)?;
            let lec: i64 = s.read::<i64, _>(1)?;
            let lra: Option<i64> = s.read::<Option<i64>, _>(2)?;
            let now = crate::wiki::norm::unix_now();
            let delta = ec - lec;
            let trigger = delta >= 20
                || (lra.is_none() && ec > 0)
                || (lra.is_some_and(|t| now - t >= 86_400) && delta > 0);
            if trigger {
                self.enqueue_rewrite(page_id)?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn get_page_for_rewrite(
        &self,
        page_id: i64,
    ) -> Result<Option<PageForRewrite>, sqlite::Error> {
        let mut s = self.conn().prepare(
            "SELECT id, kind, title, state, summary_md, facts,
                    evidence_count, last_rewrite_at, last_rewrite_evidence_count,
                    last_rewrite_max_evidence_id
               FROM wiki_pages_v2 WHERE id = ?",
        )?;
        s.bind((1, page_id))?;
        if let sqlite::State::Row = s.next()? {
            Ok(Some(PageForRewrite {
                id: s.read::<i64, _>(0)?,
                kind: s.read::<String, _>(1)?,
                title: s.read::<String, _>(2)?,
                state: s.read::<String, _>(3)?,
                summary_md: s.read::<String, _>(4)?,
                facts: s.read::<Option<String>, _>(5)?,
                evidence_count: s.read::<i64, _>(6)?,
                last_rewrite_at: s.read::<Option<i64>, _>(7)?,
                last_rewrite_evidence_count: s.read::<i64, _>(8)?,
                last_rewrite_max_evidence_id: s.read::<i64, _>(9)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Pick ≤50 evidence rows: delta since last rewrite (≤30) +
    /// top-K by salience from any remaining rows (≤20) + always-keep
    /// `cited > 0` rows. De-dup by id, cap at 50 total.
    ///
    /// Returns `(rows, snapshot_at, max_evidence_id_seen)`. Watermark is
    /// keyed on the monotonic `id` (not `created_at`) so same-second
    /// concurrent inserts can never be skipped: any row not in this
    /// selection has `id > max_id_seen` and the next delta picks it up.
    /// `snapshot_at` is also returned so the time-based 24h trigger
    /// reads a stable wall-clock anchor.
    pub fn select_rewrite_evidence(
        &self,
        page_id: i64,
        last_rewrite_max_evidence_id: i64,
    ) -> Result<(Vec<EvidenceForRewrite>, i64, i64), sqlite::Error> {
        let snapshot_at = crate::wiki::norm::unix_now();
        let mut out: Vec<EvidenceForRewrite> = Vec::new();
        let mut seen = std::collections::HashSet::<i64>::new();

        let push_row = |stmt: &mut sqlite::Statement<'_>,
                        seen: &mut std::collections::HashSet<i64>,
                        out: &mut Vec<EvidenceForRewrite>|
         -> Result<(), sqlite::Error> {
            let id: i64 = stmt.read::<i64, _>("id")?;
            if !seen.insert(id) {
                return Ok(());
            }
            out.push(EvidenceForRewrite {
                id,
                msg_id: stmt.read::<i64, _>("msg_id")?,
                chat_id: stmt.read::<i64, _>("chat_id")?,
                ts: stmt.read::<i64, _>("ts")?,
                excerpt: stmt.read::<String, _>("excerpt")?,
                salience: stmt.read::<f64, _>("salience")?,
                cited: stmt.read::<i64, _>("cited")?,
            });
            Ok(())
        };

        // Snapshot the upper id bound BEFORE selecting; pass it to apply
        // so the watermark advances exactly to "max id we could have
        // seen at select time". Any row inserted later has a strictly
        // greater id and surfaces in the next delta.
        let max_id_at_snapshot: i64 = {
            let mut s = self
                .conn()
                .prepare("SELECT COALESCE(MAX(id), 0) FROM wiki_evidence WHERE page_id = ?")?;
            s.bind((1, page_id))?;
            s.next()?;
            s.read::<i64, _>(0)?
        };

        // 1. delta since last rewrite — ≤30 newest first by id.
        {
            let mut s = self.conn().prepare(
                "SELECT id, msg_id, chat_id, ts, excerpt, salience, cited
                   FROM wiki_evidence
                  WHERE page_id = ? AND id > ? AND id <= ?
                  ORDER BY id DESC
                  LIMIT 30",
            )?;
            s.bind((1, page_id))?;
            s.bind((2, last_rewrite_max_evidence_id))?;
            s.bind((3, max_id_at_snapshot))?;
            while let sqlite::State::Row = s.next()? {
                push_row(&mut s, &mut seen, &mut out)?;
                if out.len() >= 50 {
                    return Ok((out, snapshot_at, max_id_at_snapshot));
                }
            }
        }

        // 2. top-K by salience from ANY remaining rows the delta did not
        // cover — older rows AND any delta-overflow above the 30-cap.
        // Spec §6.3 says "top-K by salience from the remainder"; the
        // earlier read excluding delta rows lost overflow on hot pages
        // (>30 new evidence since last rewrite), giving them zero top-K
        // representation. Filter by NOT IN (already-selected ids).
        if out.len() < 50 {
            let placeholders = if seen.is_empty() {
                "(NULL)".to_string()
            } else {
                let inner = seen.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                format!("({inner})")
            };
            let q = format!(
                "SELECT id, msg_id, chat_id, ts, excerpt, salience, cited
                   FROM wiki_evidence
                  WHERE page_id = ? AND id <= ? AND id NOT IN {placeholders}
                  ORDER BY salience DESC, ts DESC
                  LIMIT 20"
            );
            let mut s = self.conn().prepare(q)?;
            s.bind((1, page_id))?;
            s.bind((2, max_id_at_snapshot))?;
            for (i, id) in seen.iter().enumerate() {
                s.bind((3 + i, *id))?;
            }
            while let sqlite::State::Row = s.next()? {
                push_row(&mut s, &mut seen, &mut out)?;
                if out.len() >= 50 {
                    return Ok((out, snapshot_at, max_id_at_snapshot));
                }
            }
        }

        // 3. always-keep cited rows (only those that fit).
        if out.len() < 50 {
            let mut s = self.conn().prepare(
                "SELECT id, msg_id, chat_id, ts, excerpt, salience, cited
                   FROM wiki_evidence
                  WHERE page_id = ? AND cited > 0
                  ORDER BY cited DESC, ts DESC",
            )?;
            s.bind((1, page_id))?;
            while let sqlite::State::Row = s.next()? {
                push_row(&mut s, &mut seen, &mut out)?;
                if out.len() >= 50 {
                    break;
                }
            }
        }

        Ok((out, snapshot_at, max_id_at_snapshot))
    }

    /// Apply a rewrite per spec §6.3 in a single txn.
    /// Returns true on success. Caller wraps with BEGIN IMMEDIATE / COMMIT.
    pub fn apply_rewrite_v2(&self, r: &RewriteApply<'_>) -> Result<(), sqlite::Error> {
        use crate::wiki::norm::{nfc, title_norm, unix_now};
        let now = unix_now();

        // 1. Update page row.
        let mut s = self.conn().prepare(
            "UPDATE wiki_pages_v2
                SET summary_md = ?,
                    summary_rev = summary_rev + 1,
                    facts = ?,
                    state = ?,
                    last_rewrite_at = ?,
                    last_rewrite_max_evidence_id = MAX(last_rewrite_max_evidence_id, ?),
                    last_rewrite_evidence_count = evidence_count,
                    updated_at = ?
              WHERE id = ?",
        )?;
        s.bind((1, r.summary_md))?;
        s.bind((2, r.facts_json))?;
        s.bind((3, r.state))?;
        s.bind((4, r.snapshot_at))?;
        s.bind((5, r.max_evidence_id))?;
        s.bind((6, now))?;
        s.bind((7, r.page_id))?;
        s.next()?;

        // 2. Insert new aliases.
        let mut alias = self.conn().prepare(
            "INSERT OR IGNORE INTO wiki_page_aliases (page_id, alias_norm, alias_raw)
             VALUES (?, ?, ?)",
        )?;
        for a in r.new_aliases {
            let n = title_norm(a);
            if n.is_empty() {
                continue;
            }
            alias.bind((1, r.page_id))?;
            alias.bind((2, n.as_str()))?;
            alias.bind((3, nfc(a).as_str()))?;
            alias.next()?;
            alias.reset()?;
        }

        // 3. Refresh wiki_pages_index + pages_fts for this page.
        self.refresh_pages_index(r.page_id)?;

        // 4. Retention sweep — spec §6.3 CTE bounded by `max_evidence_id`.
        // Without the bound, evidence inserted between select snapshot
        // and apply (id > max_evidence_id) is in the table when retention
        // runs; if the new row sits low on salience the sweep would
        // delete evidence the rewrite never even saw. Restrict every
        // sub-query to id ≤ max_evidence_id so post-snapshot rows stay
        // untouched and become eligible for the next rewrite cycle.
        let drop_q = "
            WITH keep AS (
                SELECT id FROM wiki_evidence
                 WHERE page_id = ?1 AND id <= ?3
                   AND ( cited > 0
                         OR ts >= (strftime('%s','now') - 86400)
                         OR id IN (
                             SELECT id FROM (
                                 SELECT id,
                                     row_number() OVER (
                                         PARTITION BY chat_id
                                         ORDER BY ts DESC, salience DESC
                                     ) AS rn
                                   FROM wiki_evidence
                                  WHERE page_id = ?1 AND id <= ?3
                             ) WHERE rn <= 2
                         )
                       )
            ), candidates AS (
                SELECT id FROM wiki_evidence
                 WHERE page_id = ?1 AND id <= ?3 AND id NOT IN (SELECT id FROM keep)
                 ORDER BY salience ASC, ts ASC
                 LIMIT MAX(0,
                     (SELECT COUNT(*) FROM wiki_evidence WHERE page_id = ?1 AND id <= ?3)
                     - ?2)
            )
            SELECT id FROM candidates";
        let drop_ids: Vec<i64> = {
            let mut q = self.conn().prepare(drop_q)?;
            q.bind((1, r.page_id))?;
            q.bind((2, r.retention_cap))?;
            q.bind((3, r.max_evidence_id))?;
            let mut out = Vec::new();
            while let sqlite::State::Row = q.next()? {
                out.push(q.read::<i64, _>(0)?);
            }
            out
        };
        if !drop_ids.is_empty() {
            let mut del_fts = self
                .conn()
                .prepare("DELETE FROM evidence_fts WHERE rowid = ?")?;
            let mut del_evi = self
                .conn()
                .prepare("DELETE FROM wiki_evidence WHERE id = ?")?;
            for id in &drop_ids {
                del_fts.bind((1, *id))?;
                del_fts.next()?;
                del_fts.reset()?;
                del_evi.bind((1, *id))?;
                del_evi.next()?;
                del_evi.reset()?;
            }
        }

        // 5. Recompute evidence_count + LREC. Spec §6.3 step 1 sets LREC =
        // evidence_count *before* the sweep; we use the post-sweep count
        // instead. With pre-sweep LREC, a page that hits retention (e.g.
        // 200 → 50 rows) ends up with `delta = 50 - 200 < 0`, and the 24h
        // fallback also requires `evidence_count > LREC`, so the trigger
        // can never fire again until 170+ new rows arrive. Anchoring LREC
        // to the post-sweep count keeps the trigger reachable.
        self.conn().execute(format!(
            "UPDATE wiki_pages_v2
                SET evidence_count = (SELECT COUNT(*) FROM wiki_evidence WHERE page_id = {pid}),
                    last_rewrite_evidence_count = (SELECT COUNT(*) FROM wiki_evidence WHERE page_id = {pid})
              WHERE id = {pid}",
            pid = r.page_id
        ))?;

        // 6. Mark queue done.
        self.mark_rewrite_done(r.page_id)?;
        Ok(())
    }
}

/// Derive `(reason_code, reason_metrics_json)` for a candidate. Pure
/// function (no DB) so the worker can call this once per shortlisted page.
/// Spec §6.4 table priority — most specific first:
///   fresh_event > surge > cross_chat > spread > default.
/// Deferred:
///   - sustained: needs cross-refresh history (rolling median across
///     ≥3 prior refreshes); not in v9 schema. Track in handoff.
///   - pinned_active: spec puts pinned pages in a separate UI slot, not
///     part of the ranked top-10. Filter from shortlist (`pinned = 0`)
///     and surface separately when the UI lands.
pub fn derive_reason_code(c: &TrendingCandidate, now: i64) -> (String, String) {
    let velocity_ratio = if c.prior_ec >= 3 {
        (c.ec as f64) / (c.prior_ec as f64)
    } else {
        0.0
    };
    let age_secs = (now - c.created_at).max(0);
    let code = if c.kind == "event" && age_secs <= 7_200 {
        "fresh_event"
    } else if c.prior_ec >= 3 && velocity_ratio >= 2.0 {
        "surge"
    } else if c.chats >= 3 && c.senders >= 5 {
        "cross_chat"
    } else if c.chats >= 4 {
        "spread"
    } else {
        "default"
    };
    let metrics = serde_json::json!({
        "ec": c.ec,
        "chats": c.chats,
        "senders": c.senders,
        "prior_ec": c.prior_ec,
        "velocity": (velocity_ratio * 100.0).round() / 100.0,
        "last_ts": c.last_ts,
        "age_secs": age_secs,
    });
    (code.to_string(), metrics.to_string())
}

// ---- Phase 8 trending (spec §6.4) impls -----------------------------------

impl Store {
    /// Global `MAX(wiki_evidence.id)`. Snapshotted once per refresh tick;
    /// passing this through every downstream call keeps post-snapshot
    /// inserts off the shortlist + sparkline + sample queries (same race
    /// phase 7 closed for rewrite delta selection).
    pub fn current_max_evidence_id(&self) -> Result<i64, sqlite::Error> {
        let mut s = self
            .conn()
            .prepare("SELECT COALESCE(MAX(id), 0) FROM wiki_evidence")?;
        s.next()?;
        s.read::<i64, _>(0)
    }

    /// Returns `(last_evidence_id, last_computed_at)`. Missing row → (0, 0)
    /// so the strict `MAX(id) > last_evidence_id` dirty test fires for any
    /// non-empty evidence table on first run.
    pub fn read_trending_watermark(
        &self,
        window: TrendingWindow,
    ) -> Result<(i64, i64), sqlite::Error> {
        let mut s = self.conn().prepare(
            "SELECT last_evidence_id, last_computed_at \
             FROM trending_watermark WHERE window = ?",
        )?;
        s.bind((1, window.label()))?;
        if let sqlite::State::Row = s.next()? {
            Ok((s.read::<i64, _>(0)?, s.read::<i64, _>(1)?))
        } else {
            Ok((0, 0))
        }
    }

    /// Spec §6.4 shortlist. Score is computed in Rust because SQLite's
    /// `LN`/`LEAST` are compile-time-optional; bundled sqlcipher may not
    /// have math functions. SQL just emits aggregates; ranking happens here.
    /// Bounded by `snap.max_evidence_id` per the locked rewrite-phase rule.
    pub fn shortlist_trending(
        &self,
        snap: &TrendingSnapshot,
        limit: i64,
    ) -> Result<Vec<TrendingCandidate>, sqlite::Error> {
        let q = "
            WITH window_e AS (
                SELECT page_id, chat_id, sender_id, ts
                  FROM wiki_evidence
                 WHERE id <= ?1 AND ts >= ?2 AND ts < ?3
            ),
            agg AS (
                SELECT page_id,
                       COUNT(*) AS ec,
                       COUNT(DISTINCT chat_id) AS chats,
                       COUNT(DISTINCT sender_id) AS senders,
                       MAX(ts) AS last_ts
                  FROM window_e
                 GROUP BY page_id
            ),
            prior AS (
                SELECT page_id, COUNT(*) AS ec2
                  FROM wiki_evidence
                 WHERE id <= ?1 AND ts >= ?4 AND ts < ?2
                 GROUP BY page_id
            )
            SELECT p.id, p.kind, p.title, p.created_at,
                   a.ec, a.chats, a.senders, a.last_ts,
                   COALESCE(pr.ec2, 0) AS prior_ec
              FROM wiki_pages_v2 p
              JOIN agg a ON a.page_id = p.id
              LEFT JOIN prior pr ON pr.page_id = p.id
             WHERE p.state = 'active' AND p.pinned = 0";
        let mut s = self.conn().prepare(q)?;
        s.bind((1, snap.max_evidence_id))?;
        s.bind((2, snap.window_start))?;
        s.bind((3, snap.now))?;
        s.bind((4, snap.prior_start))?;
        let mut rows: Vec<TrendingCandidate> = Vec::new();
        while let sqlite::State::Row = s.next()? {
            let last_ts = s.read::<i64, _>("last_ts")?;
            let ec = s.read::<i64, _>("ec")?;
            let chats = s.read::<i64, _>("chats")?;
            let senders = s.read::<i64, _>("senders")?;
            let prior_ec = s.read::<i64, _>("prior_ec")?;
            // Spec §6.4 score:
            //   ln(1+ec) + 0.5*ln(1+chats) + 0.3*ln(1+senders)
            //   + (velocity term, capped at 3×, requires prior_ec ≥ 3)
            //   - 0.1*(now - last_ts)/3600
            let velocity = if prior_ec >= 3 {
                ((ec as f64) / (prior_ec as f64)).min(3.0) - 1.0
            } else {
                0.0
            };
            let recency = -0.1 * ((snap.now - last_ts).max(0) as f64) / 3_600.0;
            let score = (1.0 + ec as f64).ln()
                + 0.5 * (1.0 + chats as f64).ln()
                + 0.3 * (1.0 + senders as f64).ln()
                + velocity
                + recency;
            rows.push(TrendingCandidate {
                page_id: s.read::<i64, _>("id")?,
                kind: s.read::<String, _>("kind")?,
                title: s.read::<String, _>("title")?,
                created_at: s.read::<i64, _>("created_at")?,
                ec,
                chats,
                senders,
                last_ts,
                prior_ec,
                score,
            });
        }
        // Rank by score DESC, tie-break by last_ts DESC.
        rows.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.last_ts.cmp(&a.last_ts))
        });
        if limit > 0 {
            rows.truncate(limit as usize);
        }
        Ok(rows)
    }

    /// Top-N highest-salience excerpts in the window for a given page —
    /// fed to the codex reranker as `samples`.
    pub fn trending_sample_excerpts(
        &self,
        page_id: i64,
        snap: &TrendingSnapshot,
        n: i64,
    ) -> Result<Vec<String>, sqlite::Error> {
        if n <= 0 {
            return Ok(Vec::new());
        }
        let mut s = self.conn().prepare(
            "SELECT excerpt FROM wiki_evidence
              WHERE page_id = ? AND id <= ? AND ts >= ? AND ts < ?
              ORDER BY salience DESC, ts DESC
              LIMIT ?",
        )?;
        s.bind((1, page_id))?;
        s.bind((2, snap.max_evidence_id))?;
        s.bind((3, snap.window_start))?;
        s.bind((4, snap.now))?;
        s.bind((5, n))?;
        let mut out = Vec::new();
        while let sqlite::State::Row = s.next()? {
            out.push(s.read::<String, _>(0)?);
        }
        Ok(out)
    }

    /// 24 equal-width buckets across `[window_start, now)`. Counts evidence
    /// timestamps per bucket, capped at u32::MAX.
    pub fn compute_sparkline(
        &self,
        page_id: i64,
        snap: &TrendingSnapshot,
    ) -> Result<[u32; 24], sqlite::Error> {
        let span = (snap.now - snap.window_start).max(1);
        let bucket_w = (span as f64) / 24.0;
        let mut buckets = [0u32; 24];
        let mut s = self.conn().prepare(
            "SELECT ts FROM wiki_evidence
              WHERE page_id = ? AND id <= ? AND ts >= ? AND ts < ?",
        )?;
        s.bind((1, page_id))?;
        s.bind((2, snap.max_evidence_id))?;
        s.bind((3, snap.window_start))?;
        s.bind((4, snap.now))?;
        while let sqlite::State::Row = s.next()? {
            let ts = s.read::<i64, _>(0)?;
            let off = ((ts - snap.window_start).max(0) as f64) / bucket_w;
            let idx = (off as usize).min(23);
            buckets[idx] = buckets[idx].saturating_add(1);
        }
        Ok(buckets)
    }

    /// Atomic spec §6.4 apply: replace cache rows for the window + UPSERT
    /// watermark. Caller wraps with BEGIN IMMEDIATE / COMMIT so a crash
    /// mid-write never half-publishes.
    ///
    /// Returns `Ok(false)` and writes nothing if a concurrent (newer)
    /// apply already advanced the watermark past `snap.max_evidence_id`.
    /// Without this guard a slow tick can stomp a newer tick's cache:
    /// tick B (snap=200) finishes first → cache+watermark = 200. Tick A
    /// (snap=100) lands later → DELETE wipes B's cache, INSERT writes A's
    /// stale rows. The `MAX(...)` watermark UPSERT keeps last_evidence_id
    /// at 200, so the dirty test sees "clean" forever and the stale cache
    /// is never reconciled. The pre-check below makes A bail.
    pub fn apply_trending(
        &self,
        snap: &TrendingSnapshot,
        rows: &[TrendingApplyRow],
    ) -> Result<bool, sqlite::Error> {
        // 0. Stale-snapshot guard. Read the current watermark inside the
        // txn; if a newer snapshot already wrote, abort silently.
        let (current_last_id, _) = self.read_trending_watermark(snap.window)?;
        if current_last_id > snap.max_evidence_id {
            return Ok(false);
        }

        // 1. Wipe prior cache for this window.
        let mut del = self
            .conn()
            .prepare("DELETE FROM trending_cache WHERE window = ?")?;
        del.bind((1, snap.window.label()))?;
        del.next()?;

        // 2. Insert new rows. UNIQUE(window, rank) guarantees rank dedup.
        if !rows.is_empty() {
            let mut ins = self.conn().prepare(
                "INSERT INTO trending_cache
                    (window, page_id, rank, hook, reason_code,
                     reason_metrics, sparkline, computed_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )?;
            for r in rows {
                ins.bind((1, snap.window.label()))?;
                ins.bind((2, r.page_id))?;
                ins.bind((3, r.rank))?;
                ins.bind((4, r.hook.as_str()))?;
                ins.bind((5, r.reason_code.as_str()))?;
                ins.bind((6, r.reason_metrics.as_str()))?;
                ins.bind((7, r.sparkline.as_str()))?;
                ins.bind((8, snap.now))?;
                ins.next()?;
                ins.reset()?;
            }
        }

        // 3. Watermark UPSERT — both fields strictly monotonic so a late
        // retry (crash recovery, reordered txns) never rewinds either.
        let mut wm = self.conn().prepare(
            "INSERT INTO trending_watermark (window, last_evidence_id, last_computed_at)
                 VALUES (?, ?, ?)
                 ON CONFLICT(window) DO UPDATE SET
                     last_evidence_id = MAX(last_evidence_id, excluded.last_evidence_id),
                     last_computed_at = MAX(last_computed_at, excluded.last_computed_at)",
        )?;
        wm.bind((1, snap.window.label()))?;
        wm.bind((2, snap.max_evidence_id))?;
        wm.bind((3, snap.now))?;
        wm.next()?;
        Ok(true)
    }
}

pub fn compute_source_hash(sources: &[(i64, i64)]) -> String {
    let mut hasher = Sha256::new();
    for &(chat_id, message_id) in sources {
        hasher.update(chat_id.to_le_bytes());
        hasher.update(message_id.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::{NewEvidenceV2, RewriteApply};
    use crate::store::message::MessageRow;
    use crate::store::Store;

    fn setup() -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .conn()
            .execute("INSERT INTO chats (chat_id, title, chat_type) VALUES (1, 'Test', 'channel')")
            .unwrap();
        store
            .insert_messages_batch(&[
                MessageRow {
                    message_id: 1,
                    chat_id: 1,
                    timestamp: 1000,
                    text_plain: "test msg 1".to_string(),
                    text_stripped: "testmsg1".to_string(),
                    link: None,
                    sender_id: 0,
                },
                MessageRow {
                    message_id: 2,
                    chat_id: 1,
                    timestamp: 2000,
                    text_plain: "test msg 2".to_string(),
                    text_stripped: "testmsg2".to_string(),
                    link: None,
                    sender_id: 0,
                },
            ])
            .unwrap();
        store
    }

    #[test]
    fn test_insert_and_get_page() {
        let store = setup();
        let topic = crate::store::wiki_topic::NewTopic {
            title: "Test Topic".to_string(),
            title_ko: None,
            category_id: store.resolve_category("Test", None).unwrap(),
        };
        let topic_id = store.create_topic(&topic).unwrap();

        let page_id = store
            .insert_wiki_page(
                topic_id,
                "한국어 내용",
                "English content",
                &[(1, 1), (1, 2)],
            )
            .unwrap();
        assert!(page_id > 0);

        let page = store.get_latest_page(topic_id).unwrap().unwrap();
        assert_eq!(page.content_ko, "한국어 내용");
        assert_eq!(page.content_en, "English content");
        assert_eq!(page.version, 1);
        assert_eq!(page.source_count, Some(2));

        let sources = store.get_page_sources(page_id).unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].citation_index, 1);
        assert_eq!(sources[1].citation_index, 2);
    }

    #[test]
    fn test_needs_regeneration() {
        let store = setup();
        let topic = crate::store::wiki_topic::NewTopic {
            title: "Regen Test".to_string(),
            title_ko: None,
            category_id: store.resolve_category("Test", None).unwrap(),
        };
        let topic_id = store.create_topic(&topic).unwrap();

        assert!(store.needs_regeneration(topic_id).unwrap());

        let link = crate::store::wiki_topic::TopicMessageLink {
            topic_id,
            chat_id: 1,
            message_id: 1,
            relevance: 1.0,
            assigned_category: "DeFi".to_string(),
        };
        store.link_message_to_topic(&link).unwrap();
        store
            .insert_wiki_page(topic_id, "ko", "en", &[(1, 1)])
            .unwrap();

        assert!(!store.needs_regeneration(topic_id).unwrap());

        let link2 = crate::store::wiki_topic::TopicMessageLink {
            topic_id,
            chat_id: 1,
            message_id: 2,
            relevance: 0.8,
            assigned_category: "DeFi".to_string(),
        };
        store.link_message_to_topic(&link2).unwrap();
        assert!(store.needs_regeneration(topic_id).unwrap());
    }

    #[test]
    fn dedup_or_insert_page_v2_dedups_by_title_norm() {
        let store = setup();
        store.conn().execute("BEGIN").unwrap();
        let a = store
            .dedup_or_insert_page_v2("topic", "Bitcoin ETF", &["BTC ETF".into()])
            .unwrap();
        let b = store
            .dedup_or_insert_page_v2("topic", "  bitcoin   ETF  ", &[])
            .unwrap();
        store.conn().execute("COMMIT").unwrap();
        assert_eq!(a.id, b.id);

        let mut s = store
            .conn()
            .prepare("SELECT COUNT(*) FROM wiki_page_aliases WHERE page_id = ?")
            .unwrap();
        s.bind((1, a.id)).unwrap();
        s.next().unwrap();
        let n: i64 = s.read(0).unwrap();
        assert!(n >= 2);
    }

    #[test]
    fn dedup_or_insert_page_v2_dedups_by_alias() {
        let store = setup();
        store.conn().execute("BEGIN").unwrap();
        let a = store
            .dedup_or_insert_page_v2("topic", "Strategy Bitcoin Purchases", &["MSTR Buys".into()])
            .unwrap();
        let b = store
            .dedup_or_insert_page_v2("topic", "MicroStrategy Bitcoin", &["MSTR Buys".into()])
            .unwrap();
        store.conn().execute("COMMIT").unwrap();
        assert_eq!(a.id, b.id);
    }

    #[test]
    fn insert_evidence_v2_idempotent_and_bumps_count() {
        let store = setup();
        store.conn().execute("BEGIN").unwrap();
        let p = store.dedup_or_insert_page_v2("topic", "Test", &[]).unwrap();
        let evidence = NewEvidenceV2 {
            page_id: p.id,
            msg_id: 1,
            chat_id: 1,
            sender_id: 0,
            ts: 1000,
            excerpt: "hello",
            salience: 0.7,
        };
        let id1 = store.insert_evidence_v2(&evidence).unwrap();
        let id2 = store.insert_evidence_v2(&evidence).unwrap();
        store.conn().execute("COMMIT").unwrap();
        assert!(id1.is_some());
        assert!(id2.is_none());
        let mut s = store
            .conn()
            .prepare("SELECT evidence_count FROM wiki_pages_v2 WHERE id = ?")
            .unwrap();
        s.bind((1, p.id)).unwrap();
        s.next().unwrap();
        assert_eq!(s.read::<i64, _>(0).unwrap(), 1);
    }

    #[test]
    fn classify_candidates_v2_alias_then_fts() {
        let store = setup();
        store.conn().execute("BEGIN").unwrap();
        let p1 = store
            .dedup_or_insert_page_v2("topic", "Bitcoin ETF", &["BTC ETF".into()])
            .unwrap();
        let _p2 = store
            .dedup_or_insert_page_v2("topic", "Ethereum Layer 2", &["L2".into()])
            .unwrap();
        store.conn().execute("COMMIT").unwrap();

        let cands = store
            .classify_candidates_v2(&["btc etf".to_string()], "ethereum", 30)
            .unwrap();
        assert!(cands.iter().any(|c| c.id == p1.id));
    }

    fn add_evidence(store: &Store, page_id: i64, msg_id: i64, ts: i64, salience: f64) {
        store.conn().execute("BEGIN").unwrap();
        let n = NewEvidenceV2 {
            page_id,
            msg_id,
            chat_id: 1,
            sender_id: 0,
            ts,
            excerpt: "x",
            salience,
        };
        store.insert_evidence_v2(&n).unwrap();
        store.conn().execute("COMMIT").unwrap();
    }

    fn make_page(store: &Store, title: &str) -> i64 {
        store.conn().execute("BEGIN").unwrap();
        let p = store.dedup_or_insert_page_v2("topic", title, &[]).unwrap();
        store.conn().execute("COMMIT").unwrap();
        p.id
    }

    #[test]
    fn maybe_enqueue_rewrite_first_evidence_triggers() {
        let store = setup();
        let pid = make_page(&store, "Bitcoin");
        // No evidence yet: no trigger.
        assert!(!store.maybe_enqueue_rewrite(pid).unwrap());
        add_evidence(&store, pid, 100, 1_000, 0.5);
        // first evidence with NULL last_rewrite_at → trigger.
        assert!(store.maybe_enqueue_rewrite(pid).unwrap());
        let stats = store.get_rewrite_stats().unwrap();
        assert_eq!(stats.pending, 1);
    }

    #[test]
    fn maybe_enqueue_rewrite_delta_threshold() {
        let store = setup();
        let pid = make_page(&store, "ETH");
        // Pretend a rewrite already happened.
        store
            .conn()
            .execute(format!(
                "UPDATE wiki_pages_v2
                    SET last_rewrite_at = strftime('%s','now'),
                        last_rewrite_evidence_count = 0
                  WHERE id = {pid}"
            ))
            .unwrap();
        for i in 0..19 {
            add_evidence(&store, pid, 100 + i, 2_000 + i, 0.5);
        }
        assert!(!store.maybe_enqueue_rewrite(pid).unwrap());
        add_evidence(&store, pid, 200, 3_000, 0.5);
        assert!(store.maybe_enqueue_rewrite(pid).unwrap());
    }

    #[test]
    fn select_rewrite_evidence_caps_at_50_and_keeps_cited() {
        let store = setup();
        let pid = make_page(&store, "Topic");
        // 40 rows: 30 fit in delta + 10 remain for top-K. cited rows
        // therefore have room (spec §6.3 cited-keep is "if fit"). Mark
        // 5 oldest as cited.
        for i in 0..40_i64 {
            add_evidence(&store, pid, 1_000 + i, 10_000 + i, 0.1 + (i as f64) * 0.005);
        }
        store
            .conn()
            .execute(
                "UPDATE wiki_evidence SET cited = 1 WHERE msg_id IN (1000, 1001, 1002, 1003, 1004)",
            )
            .unwrap();

        let (rows, snap, max_id) = store.select_rewrite_evidence(pid, 0).unwrap();
        assert!(snap > 0);
        assert!(max_id > 0);
        assert!(rows.len() <= 50);
        let cited_present = rows.iter().any(|r| r.cited > 0);
        assert!(
            cited_present,
            "cited rows must be present when there's room"
        );
    }

    #[test]
    fn apply_rewrite_v2_updates_summary_and_drops_excess() {
        let store = setup();
        let pid = make_page(&store, "Retain");
        for i in 0..20_i64 {
            add_evidence(&store, pid, 500 + i, 100 + i, 0.1);
        }
        store.enqueue_rewrite(pid).unwrap();
        let _ = store.claim_rewrite_batch(1).unwrap();
        store.conn().execute("BEGIN IMMEDIATE").unwrap();
        store
            .apply_rewrite_v2(&RewriteApply {
                page_id: pid,
                summary_md: "Updated summary",
                facts_json: "{\"facts_version\":1}",
                state: "active",
                new_aliases: &["Alias One".to_string()],
                retention_cap: 5,
                snapshot_at: crate::wiki::norm::unix_now(),
                max_evidence_id: 99_999,
            })
            .unwrap();
        store.conn().execute("COMMIT").unwrap();

        let p = store.get_page_for_rewrite(pid).unwrap().unwrap();
        assert_eq!(p.summary_md, "Updated summary");
        assert!(p.last_rewrite_at.is_some());
        // Retention cap = 5 but spec retention also keeps last 24h + top-2/chat;
        // all 20 evidence rows are recent (ts ≈ 100s ago in test wall-clock?
        // No — strftime('%s','now') - 86400 → far future relative to ts=100..119;
        // so "last 24h" check fails and rows are NOT auto-kept by recency).
        // top-2/chat keeps 2 (only chat_id=1). cited=0 throughout. So drop down
        // to retention_cap=5 worth of rows. Final ≥ 2 (top-per-chat) and ≤ 5
        // depending on overlap.
        let mut s = store
            .conn()
            .prepare(format!(
                "SELECT COUNT(*) FROM wiki_evidence WHERE page_id = {pid}"
            ))
            .unwrap();
        s.next().unwrap();
        let n: i64 = s.read(0).unwrap();
        assert!(
            n <= 5,
            "expected ≤5 evidence after retention sweep, got {n}"
        );
        assert!(n >= 1);
        assert_eq!(p.evidence_count, n);

        let stats = store.get_rewrite_stats().unwrap();
        assert_eq!(stats.done, 1);
    }

    #[test]
    fn apply_rewrite_v2_retention_keeps_cited_rows() {
        let store = setup();
        let pid = make_page(&store, "Cited");
        for i in 0..15_i64 {
            add_evidence(&store, pid, 700 + i, 100 + i, 0.1);
        }
        // Mark one low-salience, mid-ts row as cited; retention must keep it.
        store
            .conn()
            .execute("UPDATE wiki_evidence SET cited = 3 WHERE msg_id = 705")
            .unwrap();

        store.enqueue_rewrite(pid).unwrap();
        let _ = store.claim_rewrite_batch(1).unwrap();
        store.conn().execute("BEGIN IMMEDIATE").unwrap();
        store
            .apply_rewrite_v2(&RewriteApply {
                page_id: pid,
                summary_md: "S",
                facts_json: "{\"facts_version\":1}",
                state: "active",
                new_aliases: &[],
                retention_cap: 1,
                snapshot_at: crate::wiki::norm::unix_now(),
                max_evidence_id: 99_999,
            })
            .unwrap();
        store.conn().execute("COMMIT").unwrap();

        let mut s = store
            .conn()
            .prepare("SELECT 1 FROM wiki_evidence WHERE page_id = ? AND msg_id = 705")
            .unwrap();
        s.bind((1, pid)).unwrap();
        assert!(matches!(s.next().unwrap(), sqlite::State::Row));
    }

    #[test]
    fn select_then_apply_uses_select_snapshot_for_watermark() {
        // Spec compliance: last_rewrite_at must equal the snapshot taken
        // at select time, NOT now() at apply time. Otherwise rows
        // inserted between select and apply (e.g. during the LLM call)
        // are permanently skipped from the next delta.
        let store = setup();
        let pid = make_page(&store, "Snap");
        for i in 0..5_i64 {
            add_evidence(&store, pid, 800 + i, 100 + i, 0.5);
        }
        let (_rows, snap, max_id) = store.select_rewrite_evidence(pid, 0).unwrap();

        // Simulate "LLM took some time" — sleep one second so wall clock
        // advances past snap.
        std::thread::sleep(std::time::Duration::from_millis(1100));

        store.enqueue_rewrite(pid).unwrap();
        let _ = store.claim_rewrite_batch(1).unwrap();
        store.conn().execute("BEGIN IMMEDIATE").unwrap();
        store
            .apply_rewrite_v2(&RewriteApply {
                page_id: pid,
                summary_md: "ok",
                facts_json: "{\"facts_version\":1}",
                state: "active",
                new_aliases: &[],
                retention_cap: 200,
                snapshot_at: snap,
                max_evidence_id: max_id,
            })
            .unwrap();
        store.conn().execute("COMMIT").unwrap();

        let p = store.get_page_for_rewrite(pid).unwrap().unwrap();
        assert_eq!(
            p.last_rewrite_at,
            Some(snap),
            "watermark must equal select-time snapshot, not apply-time now()"
        );
    }

    #[test]
    fn same_second_insert_after_select_not_lost_on_next_delta() {
        // Even when the new insert lands in the same wall-clock second
        // as the select snapshot, the id-based watermark must still
        // surface it on the next select.
        let store = setup();
        let pid = make_page(&store, "Race");
        for i in 0..3_i64 {
            add_evidence(&store, pid, 900 + i, 1_000 + i, 0.5);
        }
        let (_rows1, snap1, max_id1) = store.select_rewrite_evidence(pid, 0).unwrap();
        // Apply with id watermark.
        store.enqueue_rewrite(pid).unwrap();
        let _ = store.claim_rewrite_batch(1).unwrap();
        store.conn().execute("BEGIN IMMEDIATE").unwrap();
        store
            .apply_rewrite_v2(&RewriteApply {
                page_id: pid,
                summary_md: "ok",
                facts_json: "{\"facts_version\":1}",
                state: "active",
                new_aliases: &[],
                retention_cap: 200,
                snapshot_at: snap1,
                max_evidence_id: max_id1,
            })
            .unwrap();
        store.conn().execute("COMMIT").unwrap();

        // New insert in the SAME wall-clock second as the apply; lands
        // with id > max_id1.
        add_evidence(&store, pid, 999, 1_500, 0.5);

        let p = store.get_page_for_rewrite(pid).unwrap().unwrap();
        let (rows2, _, _) = store
            .select_rewrite_evidence(pid, p.last_rewrite_max_evidence_id)
            .unwrap();
        assert!(
            rows2.iter().any(|r| r.msg_id == 999),
            "new same-second evidence must appear in next delta"
        );
    }

    #[test]
    fn retention_sweep_preserves_post_snapshot_inserts() {
        // The rewrite path:
        //   1. select snapshots max_id = N
        //   2. (LLM call — lock released; classify lands new evidence
        //      with id N+1)
        //   3. apply retention with retention_cap small
        // The new id-N+1 row must survive: the rewrite never saw it,
        // so retention has no business touching it.
        let store = setup();
        let pid = make_page(&store, "Bound");
        for i in 0..10_i64 {
            add_evidence(&store, pid, 600 + i, 1_000 + i, 0.1);
        }
        let (_rows, snap, max_id) = store.select_rewrite_evidence(pid, 0).unwrap();

        // Simulate concurrent classify between select and apply.
        add_evidence(&store, pid, 700, 9_999, 0.05);
        let new_id: i64 = {
            let mut s = store
                .conn()
                .prepare("SELECT id FROM wiki_evidence WHERE msg_id = 700")
                .unwrap();
            s.next().unwrap();
            s.read(0).unwrap()
        };
        assert!(new_id > max_id, "test invariant: post-snapshot id");

        store.enqueue_rewrite(pid).unwrap();
        let _ = store.claim_rewrite_batch(1).unwrap();
        store.conn().execute("BEGIN IMMEDIATE").unwrap();
        store
            .apply_rewrite_v2(&RewriteApply {
                page_id: pid,
                summary_md: "ok",
                facts_json: "{\"facts_version\":1}",
                state: "active",
                new_aliases: &[],
                retention_cap: 1,
                snapshot_at: snap,
                max_evidence_id: max_id,
            })
            .unwrap();
        store.conn().execute("COMMIT").unwrap();

        let mut s = store
            .conn()
            .prepare("SELECT 1 FROM wiki_evidence WHERE id = ?")
            .unwrap();
        s.bind((1, new_id)).unwrap();
        assert!(
            matches!(s.next().unwrap(), sqlite::State::Row),
            "post-snapshot evidence must survive retention"
        );
    }

    #[test]
    fn top_k_pulls_from_delta_overflow() {
        // 35 new rows since last rewrite; delta caps at 30. Spec §6.3
        // top-K should pick from the 5-row overflow, not just from
        // older history. Mark the overflow rows highest-salience so
        // top-K must surface them.
        let store = setup();
        let pid = make_page(&store, "Overflow");
        for i in 0..35_i64 {
            add_evidence(&store, pid, 1_000 + i, 100 + i, 0.1 + (i as f64) * 0.01);
        }
        let (rows, _, _) = store.select_rewrite_evidence(pid, 0).unwrap();
        // Should be 50 capped — but only 35 exist, so 35.
        assert_eq!(
            rows.len(),
            35,
            "all 35 rows should surface (30 delta + 5 top-K)"
        );
    }

    // ---- Phase 8 trending tests --------------------------------------------

    use super::{TrendingApplyRow, TrendingSnapshot, TrendingWindow};

    fn add_evidence_chat(
        store: &Store,
        page_id: i64,
        msg_id: i64,
        chat_id: i64,
        sender_id: i64,
        ts: i64,
        salience: f64,
    ) {
        store.conn().execute("BEGIN").unwrap();
        let n = NewEvidenceV2 {
            page_id,
            msg_id,
            chat_id,
            sender_id,
            ts,
            excerpt: "x",
            salience,
        };
        store.insert_evidence_v2(&n).unwrap();
        store.conn().execute("COMMIT").unwrap();
    }

    fn snap(window: TrendingWindow, now: i64, max_id: i64) -> TrendingSnapshot {
        TrendingSnapshot {
            window,
            window_start: now - window.span_secs(),
            prior_start: now - 2 * window.span_secs(),
            now,
            max_evidence_id: max_id,
        }
    }

    #[test]
    fn trending_watermark_default_zero() {
        let store = setup();
        let (id, ts) = store.read_trending_watermark(TrendingWindow::H24).unwrap();
        assert_eq!(id, 0);
        assert_eq!(ts, 0);
    }

    #[test]
    fn shortlist_excludes_pinned_and_inactive() {
        let store = setup();
        // Need second chat for pinned page evidence.
        store
            .conn()
            .execute("INSERT INTO chats (chat_id, title, chat_type) VALUES (2, 'B', 'channel')")
            .unwrap();
        let active = make_page(&store, "Active");
        let pinned = make_page(&store, "Pinned");
        let inactive = make_page(&store, "Inactive");
        store
            .conn()
            .execute(format!(
                "UPDATE wiki_pages_v2 SET pinned = 1 WHERE id = {pinned}"
            ))
            .unwrap();
        store
            .conn()
            .execute(format!(
                "UPDATE wiki_pages_v2 SET state = 'hidden' WHERE id = {inactive}"
            ))
            .unwrap();
        let now = 10_000;
        for (pid, m) in &[(active, 10), (pinned, 20), (inactive, 30)] {
            add_evidence_chat(&store, *pid, *m, 1, 100, now - 100, 0.5);
        }
        let max_id = store.current_max_evidence_id().unwrap();
        let s = snap(TrendingWindow::H1, now, max_id);
        let rows = store.shortlist_trending(&s, 30).unwrap();
        let ids: Vec<i64> = rows.iter().map(|c| c.page_id).collect();
        assert!(ids.contains(&active));
        assert!(!ids.contains(&pinned));
        assert!(!ids.contains(&inactive));
    }

    #[test]
    fn shortlist_bounded_by_max_evidence_id() {
        // Snapshot caps shortlist; later inserts must not show up until
        // a new snapshot covers them.
        let store = setup();
        let pid = make_page(&store, "P");
        let now = 10_000;
        add_evidence_chat(&store, pid, 1, 1, 0, now - 100, 0.5);
        let snap1_max = store.current_max_evidence_id().unwrap();
        // Post-snapshot insert.
        add_evidence_chat(&store, pid, 2, 1, 0, now - 50, 0.5);
        let s1 = snap(TrendingWindow::H1, now, snap1_max);
        let rows = store.shortlist_trending(&s1, 30).unwrap();
        let row = rows.iter().find(|c| c.page_id == pid).unwrap();
        assert_eq!(row.ec, 1, "post-snapshot row must not leak in");
    }

    #[test]
    fn shortlist_velocity_pushes_surge() {
        // page_a: low ec, low prior. page_b: 5× prior in window — should outrank.
        let store = setup();
        let a = make_page(&store, "Calm");
        let b = make_page(&store, "Surge");
        let now = 100_000;
        let win_start = now - 3_600;
        let prior_start = now - 7_200;
        // page_a: 5 in window, 5 prior.
        for i in 0..5_i64 {
            add_evidence_chat(&store, a, 100 + i, 1, 0, win_start + 10 + i, 0.5);
            add_evidence_chat(&store, a, 200 + i, 1, 0, prior_start + 10 + i, 0.5);
        }
        // page_b: 15 in window, 3 prior.
        for i in 0..15_i64 {
            add_evidence_chat(&store, b, 300 + i, 1, 0, win_start + 10 + i, 0.5);
        }
        for i in 0..3_i64 {
            add_evidence_chat(&store, b, 400 + i, 1, 0, prior_start + 10 + i, 0.5);
        }
        let max_id = store.current_max_evidence_id().unwrap();
        let s = snap(TrendingWindow::H1, now, max_id);
        let rows = store.shortlist_trending(&s, 30).unwrap();
        let pos_a = rows.iter().position(|c| c.page_id == a).unwrap();
        let pos_b = rows.iter().position(|c| c.page_id == b).unwrap();
        assert!(pos_b < pos_a, "surge page must rank above steady page");
    }

    #[test]
    fn sparkline_buckets_correctly() {
        let store = setup();
        let pid = make_page(&store, "Spark");
        let now = 24_000;
        let span = 24_000_i64;
        let win_start = now - span;
        // Place one evidence at the start of every other bucket: bucket 0,2,4,...
        // bucket width = 1000s.
        for b in (0..24_i64).step_by(2) {
            add_evidence_chat(&store, pid, 1_000 + b, 1, 0, win_start + b * 1_000 + 1, 0.5);
        }
        let max_id = store.current_max_evidence_id().unwrap();
        let s = TrendingSnapshot {
            window: TrendingWindow::H24,
            window_start: win_start,
            prior_start: win_start - span,
            now,
            max_evidence_id: max_id,
        };
        let buckets = store.compute_sparkline(pid, &s).unwrap();
        let total: u32 = buckets.iter().sum();
        assert_eq!(total, 12, "12 evidences total");
        for (i, &count) in buckets.iter().enumerate() {
            let expected = if i % 2 == 0 { 1 } else { 0 };
            assert_eq!(count, expected, "bucket {i}");
        }
    }

    #[test]
    fn apply_trending_replaces_window_atomically() {
        let store = setup();
        let pid_a = make_page(&store, "A");
        let pid_b = make_page(&store, "B");
        let now = 10_000;
        let s = snap(TrendingWindow::H24, now, 0);

        // First apply: 2 rows.
        store.conn().execute("BEGIN IMMEDIATE").unwrap();
        store
            .apply_trending(
                &s,
                &[
                    TrendingApplyRow {
                        page_id: pid_a,
                        rank: 1,
                        hook: "first".into(),
                        reason_code: "default".into(),
                        reason_metrics: "{}".into(),
                        sparkline: "[]".into(),
                    },
                    TrendingApplyRow {
                        page_id: pid_b,
                        rank: 2,
                        hook: "second".into(),
                        reason_code: "default".into(),
                        reason_metrics: "{}".into(),
                        sparkline: "[]".into(),
                    },
                ],
            )
            .unwrap();
        store.conn().execute("COMMIT").unwrap();

        let count = |store: &Store, w: &str| -> i64 {
            let mut q = store
                .conn()
                .prepare("SELECT COUNT(*) FROM trending_cache WHERE window = ?")
                .unwrap();
            q.bind((1, w)).unwrap();
            q.next().unwrap();
            q.read::<i64, _>(0).unwrap()
        };
        assert_eq!(count(&store, "24h"), 2);

        // Second apply for the same window: 1 row — must replace.
        store.conn().execute("BEGIN IMMEDIATE").unwrap();
        store
            .apply_trending(
                &s,
                &[TrendingApplyRow {
                    page_id: pid_a,
                    rank: 1,
                    hook: "rerun".into(),
                    reason_code: "default".into(),
                    reason_metrics: "{}".into(),
                    sparkline: "[]".into(),
                }],
            )
            .unwrap();
        store.conn().execute("COMMIT").unwrap();
        assert_eq!(count(&store, "24h"), 1);

        // Watermark advanced.
        let (last_id, last_ts) = store.read_trending_watermark(TrendingWindow::H24).unwrap();
        assert_eq!(last_id, 0);
        assert_eq!(last_ts, now);
    }

    #[test]
    fn apply_trending_watermark_monotonic() {
        let store = setup();
        let now = 10_000;
        let s_low = snap(TrendingWindow::H1, now, 5);
        let s_high = snap(TrendingWindow::H1, now + 10, 12);
        store.conn().execute("BEGIN IMMEDIATE").unwrap();
        let applied_high = store.apply_trending(&s_high, &[]).unwrap();
        store.conn().execute("COMMIT").unwrap();
        assert!(applied_high);
        // Late retry with smaller max_evidence_id: stale guard returns
        // false → no DELETE, no INSERT, no watermark mutation.
        store.conn().execute("BEGIN IMMEDIATE").unwrap();
        let applied_low = store.apply_trending(&s_low, &[]).unwrap();
        store.conn().execute("COMMIT").unwrap();
        assert!(!applied_low, "stale snapshot must skip apply");
        let (last_id, _) = store.read_trending_watermark(TrendingWindow::H1).unwrap();
        assert_eq!(last_id, 12, "watermark must not regress");
    }

    #[test]
    fn apply_trending_stale_snapshot_preserves_newer_cache() {
        // Race: tick B (snap=200) commits first, cache=B. Tick A (snap=100)
        // commits second. Without the stale guard, A's DELETE wipes B's
        // rows and INSERT writes A's stale rows; the watermark MAX() pin
        // would then say "clean" forever and lock in stale cache. The
        // pre-check makes A bail.
        let store = setup();
        let pid_a = make_page(&store, "A");
        let pid_b = make_page(&store, "B");
        let now = 10_000;
        let snap_b_high = snap(TrendingWindow::H24, now, 200);
        let snap_a_low = snap(TrendingWindow::H24, now, 100);

        store.conn().execute("BEGIN IMMEDIATE").unwrap();
        store
            .apply_trending(
                &snap_b_high,
                &[TrendingApplyRow {
                    page_id: pid_b,
                    rank: 1,
                    hook: "newer".into(),
                    reason_code: "default".into(),
                    reason_metrics: "{}".into(),
                    sparkline: "[]".into(),
                }],
            )
            .unwrap();
        store.conn().execute("COMMIT").unwrap();

        // Stale tick A arrives.
        store.conn().execute("BEGIN IMMEDIATE").unwrap();
        let applied = store
            .apply_trending(
                &snap_a_low,
                &[TrendingApplyRow {
                    page_id: pid_a,
                    rank: 1,
                    hook: "stale".into(),
                    reason_code: "default".into(),
                    reason_metrics: "{}".into(),
                    sparkline: "[]".into(),
                }],
            )
            .unwrap();
        store.conn().execute("COMMIT").unwrap();
        assert!(!applied, "stale apply must report not-applied");

        // Cache must still be B's row, untouched.
        let mut q = store
            .conn()
            .prepare("SELECT page_id, hook FROM trending_cache WHERE window = '24h'")
            .unwrap();
        q.next().unwrap();
        assert_eq!(q.read::<i64, _>(0).unwrap(), pid_b);
        assert_eq!(q.read::<String, _>(1).unwrap(), "newer");
    }

    use super::derive_reason_code;

    fn make_cand(
        kind: &str,
        ec: i64,
        chats: i64,
        senders: i64,
        prior_ec: i64,
        age: i64,
    ) -> super::TrendingCandidate {
        super::TrendingCandidate {
            page_id: 1,
            kind: kind.into(),
            title: "T".into(),
            created_at: 10_000 - age,
            ec,
            chats,
            senders,
            last_ts: 10_000,
            prior_ec,
            score: 0.0,
        }
    }

    #[test]
    fn reason_fresh_event_first() {
        // Even with surge metrics, fresh_event wins because it's first in priority.
        let c = make_cand("event", 30, 5, 8, 5, 3_600);
        let (code, _) = derive_reason_code(&c, 10_000);
        assert_eq!(code, "fresh_event");
    }

    #[test]
    fn reason_event_too_old_falls_through() {
        // Event > 2h old; fresh_event off, surge takes over.
        let c = make_cand("event", 30, 1, 1, 5, 7_201);
        let (code, _) = derive_reason_code(&c, 10_000);
        assert_eq!(code, "surge");
    }

    #[test]
    fn reason_surge_needs_prior_ge_3() {
        // velocity 100/2 huge, but prior < 3 → no surge.
        let c = make_cand("topic", 100, 2, 2, 2, 0);
        let (code, _) = derive_reason_code(&c, 10_000);
        assert_eq!(code, "default");
    }

    #[test]
    fn reason_surge_classic() {
        let c = make_cand("topic", 30, 1, 1, 5, 10_000);
        let (code, _) = derive_reason_code(&c, 10_000);
        assert_eq!(code, "surge");
    }

    #[test]
    fn reason_cross_chat_before_spread() {
        // 3 chats + 5 senders → cross_chat (more specific than spread's
        // chats ≥ 4 but with only 3 here).
        let c = make_cand("topic", 5, 3, 5, 0, 10_000);
        let (code, _) = derive_reason_code(&c, 10_000);
        assert_eq!(code, "cross_chat");
    }

    #[test]
    fn reason_spread_when_no_cross_chat() {
        let c = make_cand("topic", 5, 4, 1, 0, 10_000);
        let (code, _) = derive_reason_code(&c, 10_000);
        assert_eq!(code, "spread");
    }

    #[test]
    fn reason_default_fallthrough() {
        let c = make_cand("topic", 5, 1, 1, 0, 10_000);
        let (code, metrics) = derive_reason_code(&c, 10_000);
        assert_eq!(code, "default");
        assert!(metrics.contains("\"ec\":5"));
    }

    #[test]
    fn shortlist_limit_caps_results() {
        let store = setup();
        for i in 0..5_i64 {
            let pid = make_page(&store, &format!("P{i}"));
            add_evidence_chat(&store, pid, 100 + i, 1, 0, 9_000 + i, 0.5);
        }
        let max_id = store.current_max_evidence_id().unwrap();
        let s = snap(TrendingWindow::H1, 10_000, max_id);
        let rows = store.shortlist_trending(&s, 3).unwrap();
        assert_eq!(rows.len(), 3);
    }
}
