//! Title/alias/text normalization helpers used by classify v2.

use unicode_normalization::UnicodeNormalization;

/// NFC-normalize text. Used for `text_hash` input and excerpt hashing.
pub fn nfc(s: &str) -> String {
    s.nfc().collect()
}

/// Normalize a title or alias for dedup keys: NFKC + lowercase + whitespace squash.
pub fn title_norm(s: &str) -> String {
    let nfkc: String = s.nfkc().collect();
    let lower = nfkc.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut last_was_ws = true;
    for c in lower.chars() {
        if c.is_whitespace() {
            if !last_was_ws {
                out.push(' ');
                last_was_ws = true;
            }
        } else {
            out.push(c);
            last_was_ws = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Count NFC-normalized characters (for `min_classify_chars` gate).
pub fn nfc_char_count(s: &str) -> usize {
    s.nfc().count()
}

/// 16-byte BLAKE3 of NFC-normalized text.
pub fn blake3_16_nfc(text: &str) -> Vec<u8> {
    let nfc_bytes = nfc(text);
    blake3::hash(nfc_bytes.as_bytes()).as_bytes()[..16].to_vec()
}

/// Source-hash composition per spec §5.2:
/// BLAKE3(decimal page_id || decimal msg_id || decimal chat_id || NFC(excerpt)) -> 16 bytes.
/// Length-prefixed so distinct fields cannot collide.
pub fn evidence_source_hash(page_id: i64, msg_id: i64, chat_id: i64, excerpt: &str) -> Vec<u8> {
    let mut h = blake3::Hasher::new();
    let p = page_id.to_string();
    let m = msg_id.to_string();
    let c = chat_id.to_string();
    let e = nfc(excerpt);
    h.update(&(p.len() as u32).to_le_bytes());
    h.update(p.as_bytes());
    h.update(&(m.len() as u32).to_le_bytes());
    h.update(m.as_bytes());
    h.update(&(c.len() as u32).to_le_bytes());
    h.update(c.as_bytes());
    h.update(&(e.len() as u32).to_le_bytes());
    h.update(e.as_bytes());
    h.finalize().as_bytes()[..16].to_vec()
}

pub fn unix_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfc_idempotent() {
        let a = "café";
        let b = "cafe\u{0301}";
        assert_eq!(nfc(a), nfc(b));
    }

    #[test]
    fn title_norm_collapses_ws_and_lowercases() {
        assert_eq!(title_norm("  Bitcoin   ETF\tNews "), "bitcoin etf news");
    }

    #[test]
    fn title_norm_nfkc_compat() {
        let a = title_norm("Bitcoin 2024");
        let b = title_norm("Bitcoin ２０２４");
        assert_eq!(a, b);
    }

    #[test]
    fn blake3_16_nfc_collapses_forms() {
        let a = blake3_16_nfc("café");
        let b = blake3_16_nfc("cafe\u{0301}");
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn evidence_source_hash_stable_and_collision_resistant() {
        let h1 = evidence_source_hash(1, 2, 3, "hello");
        let h2 = evidence_source_hash(1, 2, 3, "hello");
        assert_eq!(h1, h2);
        let h3 = evidence_source_hash(12, 3, 3, "hello");
        assert_ne!(h1, h3);
    }
}
