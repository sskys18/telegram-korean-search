use serde::{Deserialize, Serialize};

use super::Store;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WikiCategory {
    pub category_id: i64,
    pub name: String,
    pub name_ko: Option<String>,
    pub sort_order: i64,
}

/// Common aliases that should merge into the same category.
/// Maps normalized form → canonical name.
const KNOWN_ALIASES: &[(&[&str], &str)] = &[
    (&["defi", "decentralized finance", "디파이"], "DeFi"),
    (&["nft", "nfts", "non-fungible"], "NFT"),
    (
        &[
            "l1",
            "l2",
            "l1/l2",
            "layer 1",
            "layer 2",
            "layer1",
            "layer2",
            "레이어",
        ],
        "L1/L2",
    ),
    (&["airdrop", "airdrops", "에어드롭"], "Airdrop"),
    (&["trading", "트레이딩", "매매"], "Trading"),
    (&["regulation", "규제", "legal", "compliance"], "Regulation"),
    (
        &["macro", "매크로", "macro economy", "macroeconomy"],
        "Macro",
    ),
    (
        &["scam", "scam alert", "스캠", "rug pull", "rugpull", "fraud"],
        "Scam Alert",
    ),
    (&["meme", "memecoin", "meme coin", "밈코인"], "Memecoin"),
    (&["bitcoin", "btc", "비트코인"], "Bitcoin"),
    (&["ethereum", "eth", "이더리움"], "Ethereum"),
    (&["solana", "sol", "솔라나"], "Solana"),
    (
        &["stablecoin", "stablecoins", "스테이블코인", "usdt", "usdc"],
        "Stablecoin",
    ),
    (&["gaming", "gamefi", "게임파이", "game"], "GameFi"),
    (
        &["ai", "artificial intelligence", "인공지능", "ai crypto"],
        "AI",
    ),
    (&["dex", "decentralized exchange", "탈중앙거래소"], "DEX"),
    (
        &["cex", "centralized exchange", "거래소", "binance", "upbit"],
        "CEX",
    ),
    (&["dao", "governance", "거버넌스"], "DAO"),
    (
        &["privacy", "프라이버시", "zero knowledge", "zk"],
        "Privacy/ZK",
    ),
    (&["news", "뉴스", "announcement", "공지"], "News"),
];

impl Store {
    pub fn get_all_categories(&self) -> Result<Vec<WikiCategory>, sqlite::Error> {
        // Order by message count (most used first), then name
        let mut stmt = self.conn().prepare(
            "SELECT c.category_id, c.name, c.name_ko, c.sort_order
             FROM wiki_categories c
             LEFT JOIN wiki_topics t ON t.category_id = c.category_id
             GROUP BY c.category_id
             ORDER BY COALESCE(SUM(t.message_count), 0) DESC, c.name",
        )?;
        let mut cats = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            cats.push(WikiCategory {
                category_id: stmt.read::<i64, _>("category_id")?,
                name: stmt.read::<String, _>("name")?,
                name_ko: stmt.read::<Option<String>, _>("name_ko")?,
                sort_order: stmt.read::<i64, _>("sort_order")?,
            });
        }
        Ok(cats)
    }

    pub fn get_category_by_id(
        &self,
        category_id: i64,
    ) -> Result<Option<WikiCategory>, sqlite::Error> {
        let mut stmt = self.conn().prepare(
            "SELECT category_id, name, name_ko, sort_order FROM wiki_categories WHERE category_id = ?",
        )?;
        stmt.bind((1, category_id))?;
        if let sqlite::State::Row = stmt.next()? {
            Ok(Some(WikiCategory {
                category_id: stmt.read::<i64, _>("category_id")?,
                name: stmt.read::<String, _>("name")?,
                name_ko: stmt.read::<Option<String>, _>("name_ko")?,
                sort_order: stmt.read::<i64, _>("sort_order")?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Resolve a free-form category string from the LLM into a category_id.
    /// 1. Check known aliases → canonical name
    /// 2. Exact match (case-insensitive) against existing categories
    /// 3. Substring match against existing categories
    /// 4. Auto-create new category if no match
    pub fn resolve_category(&self, raw: &str, raw_ko: Option<&str>) -> Result<i64, sqlite::Error> {
        let normalized = raw.trim().to_lowercase();

        if normalized.is_empty() {
            return self.resolve_category("Other", None);
        }

        // Step 1: Check known aliases
        let canonical = find_canonical_name(&normalized);

        // Step 2: Exact match (case-insensitive) on canonical or raw
        let search_name = canonical.unwrap_or(&normalized);
        let mut stmt = self
            .conn()
            .prepare("SELECT category_id FROM wiki_categories WHERE LOWER(name) = ?")?;
        stmt.bind((1, search_name.to_lowercase().as_str()))?;
        if let sqlite::State::Row = stmt.next()? {
            return stmt.read::<i64, _>(0);
        }

        // Also try the original raw name if canonical was different
        if canonical.is_some() {
            let mut stmt2 = self
                .conn()
                .prepare("SELECT category_id FROM wiki_categories WHERE LOWER(name) = ?")?;
            stmt2.bind((1, normalized.as_str()))?;
            if let sqlite::State::Row = stmt2.next()? {
                return stmt2.read::<i64, _>(0);
            }
        }

        // Step 3: Substring match — check if any existing category contains or is contained by the query
        let mut stmt = self
            .conn()
            .prepare("SELECT category_id, name FROM wiki_categories ORDER BY category_id")?;
        let mut candidates = Vec::new();
        while let sqlite::State::Row = stmt.next()? {
            candidates.push((
                stmt.read::<i64, _>("category_id")?,
                stmt.read::<String, _>("name")?,
            ));
        }

        for (id, name) in &candidates {
            let name_lower = name.to_lowercase();
            if name_lower.contains(&normalized) || normalized.contains(&name_lower) {
                return Ok(*id);
            }
        }

        // Step 4: Auto-create new category
        let display_name = canonical
            .map(|s| s.to_string())
            .unwrap_or_else(|| titlecase(raw.trim()));

        let name_ko_val = raw_ko.map(|s| s.trim().to_string());
        let next_order = candidates.len() as i64 + 1;

        let mut ins = self
            .conn()
            .prepare("INSERT INTO wiki_categories (name, name_ko, sort_order) VALUES (?, ?, ?)")?;
        ins.bind((1, display_name.as_str()))?;
        ins.bind((2, name_ko_val.as_deref()))?;
        ins.bind((3, next_order))?;
        ins.next()?;

        let mut last = self.conn().prepare("SELECT last_insert_rowid()")?;
        last.next()?;
        last.read::<i64, _>(0)
    }

    /// Backwards-compatible alias for resolve_category
    pub fn normalize_category(&self, raw: &str) -> Result<i64, sqlite::Error> {
        self.resolve_category(raw, None)
    }
}

/// Check if the normalized input matches any known alias.
/// Returns the canonical display name if found.
fn find_canonical_name(normalized: &str) -> Option<&'static str> {
    for (aliases, canonical) in KNOWN_ALIASES {
        for alias in *aliases {
            if *alias == normalized {
                return Some(canonical);
            }
        }
    }
    None
}

/// Simple titlecase: capitalize first letter of each word.
fn titlecase(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    format!("{}{}", upper, chars.as_str())
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn test_resolve_creates_new_category() {
        let store = Store::open_in_memory().unwrap();
        let id = store.resolve_category("DeFi", Some("디파이")).unwrap();
        assert!(id > 0);

        let cat = store.get_category_by_id(id).unwrap().unwrap();
        assert_eq!(cat.name, "DeFi");
        assert_eq!(cat.name_ko, Some("디파이".to_string()));
    }

    #[test]
    fn test_resolve_deduplicates() {
        let store = Store::open_in_memory().unwrap();
        let id1 = store.resolve_category("DeFi", None).unwrap();
        let id2 = store.resolve_category("defi", None).unwrap();
        let id3 = store
            .resolve_category("Decentralized Finance", None)
            .unwrap();
        assert_eq!(id1, id2);
        assert_eq!(id1, id3);
    }

    #[test]
    fn test_resolve_known_aliases() {
        let store = Store::open_in_memory().unwrap();
        let id1 = store.resolve_category("btc", None).unwrap();
        let id2 = store.resolve_category("Bitcoin", None).unwrap();
        let id3 = store.resolve_category("비트코인", None).unwrap();
        assert_eq!(id1, id2);
        assert_eq!(id1, id3);

        let cat = store.get_category_by_id(id1).unwrap().unwrap();
        assert_eq!(cat.name, "Bitcoin");
    }

    #[test]
    fn test_resolve_new_unknown_category() {
        let store = Store::open_in_memory().unwrap();
        let id = store.resolve_category("real world assets", None).unwrap();
        let cat = store.get_category_by_id(id).unwrap().unwrap();
        assert_eq!(cat.name, "Real World Assets"); // titlecased
    }

    #[test]
    fn test_get_all_categories_ordered_by_usage() {
        let store = Store::open_in_memory().unwrap();
        store.resolve_category("Alpha", None).unwrap();
        store.resolve_category("Beta", None).unwrap();
        let cats = store.get_all_categories().unwrap();
        assert_eq!(cats.len(), 2);
    }

    #[test]
    fn test_titlecase() {
        assert_eq!(titlecase("hello world"), "Hello World");
        assert_eq!(titlecase("DeFi"), "DeFi");
        assert_eq!(titlecase("real world assets"), "Real World Assets");
    }

    #[test]
    fn test_find_canonical() {
        assert_eq!(find_canonical_name("defi"), Some("DeFi"));
        assert_eq!(find_canonical_name("btc"), Some("Bitcoin"));
        assert_eq!(find_canonical_name("something random"), None);
    }
}
