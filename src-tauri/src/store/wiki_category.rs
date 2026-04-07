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
    // --- Crypto core ---
    (&["defi", "decentralized finance", "디파이"], "DeFi"),
    (
        &["nft", "nfts", "non-fungible", "collectibles", "수집품"],
        "NFT",
    ),
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
    (
        &["airdrop", "airdrops", "에어드롭", "campaign", "rewards"],
        "Airdrop",
    ),
    (&["trading", "트레이딩", "매매"], "Trading"),
    (
        &[
            "regulation",
            "규제",
            "legal",
            "compliance",
            "legislation",
            "입법",
            "enforcement",
        ],
        "Regulation",
    ),
    (
        &["macro", "매크로", "macro economy", "macroeconomy"],
        "Macro",
    ),
    (
        &[
            "scam",
            "scam alert",
            "scams",
            "스캠",
            "rug pull",
            "rugpull",
            "fraud",
            "사기",
            "사기주의",
        ],
        "Scam Alert",
    ),
    (
        &["meme", "memecoin", "meme coin", "밈코인", "밈"],
        "Memecoin",
    ),
    (&["bitcoin", "btc", "비트코인", "btcfi"], "Bitcoin"),
    (&["ethereum", "eth", "이더리움"], "Ethereum"),
    (&["solana", "sol", "솔라나"], "Solana"),
    (
        &["stablecoin", "stablecoins", "스테이블코인", "usdt", "usdc"],
        "Stablecoin",
    ),
    (
        &[
            "gaming",
            "gamefi",
            "게임파이",
            "game",
            "game economy",
            "gaming economy",
            "게임 경제",
        ],
        "GameFi",
    ),
    (
        &[
            "ai",
            "artificial intelligence",
            "인공지능",
            "ai crypto",
            "agi",
        ],
        "AI",
    ),
    (&["dex", "decentralized exchange", "탈중앙거래소"], "DEX"),
    (
        &[
            "cex",
            "centralized exchange",
            "거래소",
            "binance",
            "upbit",
            "바이낸스",
        ],
        "CEX",
    ),
    (&["dao", "governance", "거버넌스"], "DAO"),
    (
        &[
            "privacy",
            "프라이버시",
            "zero knowledge",
            "zk",
            "privacy/zk",
            "privacy coin",
            "프라이버시 코인",
        ],
        "Privacy/ZK",
    ),
    (&["news", "뉴스", "announcement", "공지"], "News"),
    (&["token", "토큰", "tokenomics"], "Token"),
    (&["altcoin", "알트코인"], "Altcoin"),
    (
        &["prediction market", "prediction markets", "예측시장"],
        "Prediction Market",
    ),
    (&["rwa", "real world assets", "실물자산토큰화"], "RWA"),
    (
        &["staking", "스테이킹", "yield farming", "예치작"],
        "Staking",
    ),
    // --- Markets & finance ---
    (
        &[
            "equities",
            "equity",
            "stocks",
            "stock market",
            "증시",
            "주식",
        ],
        "Equities",
    ),
    (
        &[
            "market sentiment",
            "market psychology",
            "sentiment",
            "시장 심리",
            "investor sentiment",
            "투자심리",
        ],
        "Market Sentiment",
    ),
    (
        &[
            "market analysis",
            "market data",
            "market overview",
            "market commentary",
            "market recap",
            "시장분석",
            "시장 해설",
            "시장정리",
            "시장 개요",
        ],
        "Market Analysis",
    ),
    (
        &[
            "market structure",
            "market microstructure",
            "시장 구조",
            "시장 미시구조",
        ],
        "Market Structure",
    ),
    (
        &[
            "market moves",
            "market activity",
            "market snapshot",
            "market movers",
            "시장 변동",
            "시장 동향",
            "시장 등락",
        ],
        "Market Activity",
    ),
    (
        &[
            "market trend",
            "market cycle",
            "market narrative",
            "cycle analysis",
            "시장 트렌드",
            "시장 사이클",
            "시장 내러티브",
            "사이클 분석",
        ],
        "Market Trend",
    ),
    (
        &[
            "market update",
            "market alert",
            "market calendar",
            "market schedule",
            "시황",
            "시장 경고",
            "증시일정",
            "시장 일정",
        ],
        "Market Update",
    ),
    (&["commodities", "commodity", "원자재"], "Commodities"),
    (
        &[
            "derivatives",
            "perps",
            "perpetuals",
            "perp dex",
            "perpdex",
            "futures",
            "파생상품",
            "무기한선물",
            "파생 dex",
            "퍼프덱스",
        ],
        "Derivatives",
    ),
    (&["options", "옵션"], "Options"),
    (&["fx", "forex", "환율", "외환", "currency", "통화"], "FX"),
    (
        &["etf", "etf/fund", "etf/펀드", "index", "지수"],
        "ETF/Fund",
    ),
    (
        &[
            "technical analysis",
            "tech analysis",
            "기술적분석",
            "기술 분석",
            "price action",
            "가격 흐름",
        ],
        "Technical Analysis",
    ),
    (
        &[
            "corporate finance",
            "corporate",
            "corporate action",
            "기업금융",
            "기업행동",
        ],
        "Corporate Finance",
    ),
    (&["investing", "investment", "투자"], "Investing"),
    (
        &[
            "earnings",
            "실적",
            "revenue",
            "수익",
            "profitability",
            "수익성",
        ],
        "Earnings",
    ),
    (
        &["valuation", "밸류에이션", "fundamentals", "펀더멘털"],
        "Valuation",
    ),
    // --- Tech ---
    (
        &[
            "semiconductors",
            "반도체",
            "semiconductor equipment",
            "반도체 장비",
        ],
        "Semiconductors",
    ),
    (
        &["tech stocks", "기술주", "us tech", "미국 기술주"],
        "Tech Stocks",
    ),
    (
        &["developer tools", "dev tools", "devtools", "개발도구"],
        "Developer Tools",
    ),
    (
        &["data center", "데이터센터", "cloud", "클라우드"],
        "Data Center",
    ),
    // --- Geopolitics ---
    (
        &[
            "geopolitics",
            "지정학",
            "international relations",
            "국제관계",
        ],
        "Geopolitics",
    ),
    (
        &["war", "전쟁", "conflict", "분쟁", "military", "군사"],
        "War",
    ),
    (
        &["sanctions", "제재", "export controls", "수출 규제"],
        "Sanctions",
    ),
    (&["diplomacy", "외교"], "Diplomacy"),
    // --- Economy ---
    (
        &[
            "energy",
            "에너지",
            "oil",
            "원유",
            "natural gas",
            "천연가스",
            "lng",
        ],
        "Energy",
    ),
    (&["infrastructure", "인프라"], "Infrastructure"),
    (
        &[
            "jobs",
            "hiring",
            "recruiting",
            "careers",
            "채용",
            "employment",
            "고용",
        ],
        "Jobs",
    ),
    (
        &[
            "security",
            "보안",
            "cybercrime",
            "사이버범죄",
            "hack",
            "exploit",
            "익스플로잇",
        ],
        "Security",
    ),
    (&["payments", "결제", "fintech"], "Payments"),
    (&["defense", "방산", "aerospace", "항공우주"], "Defense"),
    (
        &[
            "automotive",
            "autos",
            "자동차",
            "autonomous driving",
            "자율주행",
        ],
        "Automotive",
    ),
    (&["battery", "batteries", "배터리"], "Battery"),
    (&["film", "movie", "영화"], "Film"),
    (
        &["tv", "broadcast", "방송", "streaming", "스트리밍"],
        "Broadcast",
    ),
    (&["humor", "유머", "satire", "풍자"], "Humor"),
    (&["food", "음식", "diet", "다이어트"], "Food"),
    (&["society", "사회", "public safety"], "Society"),
    (
        &[
            "people",
            "profile",
            "personality",
            "인물",
            "biography",
            "약력",
            "celebrity",
            "셀럽",
        ],
        "People",
    ),
    (&["due diligence", "duediligence", "실사"], "Due Diligence"),
    (&["pnl", "p&l", "손익"], "PnL"),
    (&["pre-market", "premarket", "프리마켓"], "Pre-Market"),
    (&["resale", "flipping", "reselling", "리셀"], "Resale"),
    (
        &["fiscal", "public finance", "재정", "budget", "예산"],
        "Fiscal",
    ),
    (&["controversy", "scandal", "논란"], "Controversy"),
    (&["event", "giveaway", "이벤트"], "Event"),
    (
        &["elections", "선거", "election law", "선거법"],
        "Elections",
    ),
    (
        &["gambling", "casino", "betting", "도박", "카지노", "베팅"],
        "Gambling",
    ),
    (&["venture", "vc", "벤처", "funding", "투자유치"], "Venture"),
    (&["정치/사회", "sociopolitics", "사회정치"], "Society"),
    (&["엔터테인먼트", "entertainment"], "Entertainment"),
    (&["암호화폐", "crypto", "크립토"], "Crypto"),
    (&["일상/감성", "lifestyle", "라이프스타일"], "Lifestyle"),
    (
        &["korean politics", "한국 정치", "local politics", "지방정치"],
        "Korean Politics",
    ),
    (
        &[
            "us politics",
            "미국 정치",
            "us policy",
            "미국 정책",
            "us foreign policy",
            "미국 대외정책",
        ],
        "US Politics",
    ),
    (
        &[
            "central bank",
            "중앙은행",
            "fed",
            "연준",
            "monetary policy",
            "통화정책",
            "rates",
            "금리",
        ],
        "Central Bank",
    ),
    (&["gold", "금"], "Gold"),
    (&["insurance", "보험"], "Insurance"),
    (
        &["real estate", "housing policy", "주거정책"],
        "Real Estate",
    ),
    (
        &[
            "biotech",
            "pharma",
            "바이오",
            "제약",
            "healthcare",
            "헬스케어",
        ],
        "Biotech",
    ),
    (
        &[
            "korea market",
            "korean stocks",
            "korea stocks",
            "국내증시",
            "한국 시장",
            "kosdaq",
            "코스닥",
        ],
        "Korean Stocks",
    ),
    (
        &["us stocks", "미국증시", "blue chips", "대형주"],
        "US Stocks",
    ),
    (&["inflation", "물가"], "Inflation"),
    (
        &["exchange", "listing", "exchange listing", "상장"],
        "Exchange",
    ),
    (&["community", "커뮤니티", "social", "소셜"], "Community"),
    (&["trump", "트럼프"], "Trump"),
    (&["china", "중국"], "China"),
    (&["tariffs", "관세", "trade", "무역"], "Trade"),
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

    pub fn clear_wiki_categories(&self) -> Result<(), sqlite::Error> {
        self.conn().execute("DELETE FROM wiki_categories")?;
        Ok(())
    }

    /// Resolve a category and return both the id and canonical name.
    pub fn resolve_category_with_name(
        &self,
        raw: &str,
        raw_ko: Option<&str>,
    ) -> Result<(i64, String), sqlite::Error> {
        let id = self.resolve_category(raw, raw_ko)?;
        let name = self
            .get_category_by_id(id)?
            .map(|c| c.name)
            .unwrap_or_else(|| raw.to_string());
        Ok((id, name))
    }
}

/// Public version for use by schema migrations.
pub fn find_canonical_name_pub(normalized: &str) -> Option<&'static str> {
    find_canonical_name(normalized)
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
        // "real world assets" is now a known alias for "RWA"
        let id = store.resolve_category("real world assets", None).unwrap();
        let cat = store.get_category_by_id(id).unwrap().unwrap();
        assert_eq!(cat.name, "RWA");

        // Truly unknown category gets titlecased
        let id2 = store
            .resolve_category("quantum computing trends", None)
            .unwrap();
        let cat2 = store.get_category_by_id(id2).unwrap().unwrap();
        assert_eq!(cat2.name, "Quantum Computing Trends");
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
