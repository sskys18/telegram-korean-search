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
                    evidence_count, last_rewrite_at, last_rewrite_evidence_count
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
            }))
        } else {
            Ok(None)
        }
    }

    /// Pick ≤50 evidence rows: delta since `last_rewrite_at` (≤30) +
    /// top-K by salience from the remainder (≤20) + always-keep
    /// `cited > 0` rows. De-dup by id, cap at 50 total.
    pub fn select_rewrite_evidence(
        &self,
        page_id: i64,
        last_rewrite_at: Option<i64>,
    ) -> Result<Vec<EvidenceForRewrite>, sqlite::Error> {
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

        // Delta is "inserted since last rewrite", not "message ts > last
        // rewrite". Backfill / historical re-classify inserts evidence with
        // old `ts` but recent `created_at`; comparing on `ts` would
        // permanently lose those rows from the delta window. Mirrors spec
        // §6.4 trending watermark which switched from ts to monotonic id
        // for the same reason.
        let cutoff = last_rewrite_at.unwrap_or(0);

        // 1. delta since last rewrite, ≤30 newest first
        {
            let mut s = self.conn().prepare(
                "SELECT id, msg_id, chat_id, ts, excerpt, salience, cited
                   FROM wiki_evidence
                  WHERE page_id = ? AND created_at > ?
                  ORDER BY created_at DESC
                  LIMIT 30",
            )?;
            s.bind((1, page_id))?;
            s.bind((2, cutoff))?;
            while let sqlite::State::Row = s.next()? {
                push_row(&mut s, &mut seen, &mut out)?;
                if out.len() >= 50 {
                    return Ok(out);
                }
            }
        }

        // 2. top-K by salience from the rest, ≤20
        {
            let mut s = self.conn().prepare(
                "SELECT id, msg_id, chat_id, ts, excerpt, salience, cited
                   FROM wiki_evidence
                  WHERE page_id = ? AND created_at <= ?
                  ORDER BY salience DESC, ts DESC
                  LIMIT 20",
            )?;
            s.bind((1, page_id))?;
            s.bind((2, cutoff))?;
            while let sqlite::State::Row = s.next()? {
                push_row(&mut s, &mut seen, &mut out)?;
                if out.len() >= 50 {
                    return Ok(out);
                }
            }
        }

        // 3. always-keep cited rows (only those that fit)
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

        Ok(out)
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
                    last_rewrite_evidence_count = evidence_count,
                    updated_at = ?
              WHERE id = ?",
        )?;
        s.bind((1, r.summary_md))?;
        s.bind((2, r.facts_json))?;
        s.bind((3, r.state))?;
        s.bind((4, now))?;
        s.bind((5, now))?;
        s.bind((6, r.page_id))?;
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

        // 4. Retention sweep — copy spec §6.3 CTE.
        // Keep: cited>0, last 24h, top-2 per chat by (ts DESC, salience DESC).
        // Then drop lowest-salience among the remainder until total ≤ cap.
        let drop_q = "
            WITH keep AS (
                SELECT id FROM wiki_evidence
                 WHERE page_id = ?1
                   AND ( cited > 0
                         OR ts >= (strftime('%s','now') - 86400)
                         OR id IN (
                             SELECT id FROM (
                                 SELECT id,
                                     row_number() OVER (
                                         PARTITION BY chat_id
                                         ORDER BY ts DESC, salience DESC
                                     ) AS rn
                                   FROM wiki_evidence WHERE page_id = ?1
                             ) WHERE rn <= 2
                         )
                       )
            ), candidates AS (
                SELECT id FROM wiki_evidence
                 WHERE page_id = ?1 AND id NOT IN (SELECT id FROM keep)
                 ORDER BY salience ASC, ts ASC
                 LIMIT MAX(0, (SELECT COUNT(*) FROM wiki_evidence WHERE page_id = ?1) - ?2)
            )
            SELECT id FROM candidates";
        let drop_ids: Vec<i64> = {
            let mut q = self.conn().prepare(drop_q)?;
            q.bind((1, r.page_id))?;
            q.bind((2, r.retention_cap))?;
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
        // 60 rows, ascending ts; last 5 marked cited.
        for i in 0..60_i64 {
            add_evidence(&store, pid, 1_000 + i, 10_000 + i, 0.1 + (i as f64) * 0.005);
        }
        store
            .conn()
            .execute(
                "UPDATE wiki_evidence SET cited = 1 WHERE msg_id IN (1000, 1001, 1002, 1003, 1004)",
            )
            .unwrap();

        let rows = store.select_rewrite_evidence(pid, Some(0)).unwrap();
        assert!(rows.len() <= 50);
        // delta cutoff 0 → all rows are "newer", so first 30 newest pulled by ts DESC.
        // cited rows (oldest ts) are also present via category 3.
        let cited_present = rows.iter().any(|r| r.cited > 0);
        assert!(cited_present);
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
}
