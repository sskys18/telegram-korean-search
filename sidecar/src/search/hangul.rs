//! Hangul-aware normalizers for the Korean search index.
//!
//! The FTS5 trigram tokenizer matches any three-codepoint substring.
//! That handles `삼성` → `삼성전자` out of the box because both the
//! indexed string and the query contain the same syllable code
//! points. It does *not* handle two important Korean patterns:
//!
//! 1. **자모 (jamo) input** — a user types `ㅅㅏㅁ` expecting to match
//!    `삼`. The syllable `삼` is one Unicode codepoint, not three.
//! 2. **Whitespace** — `삼성 전자` should match `삼성전자`. The
//!    existing `text_stripped` column already has this; this module
//!    only needs to normalize the query to match.
//!
//! The strategy here is to pre-compute one extra searchable form of
//! every message at index time — `text_jamo` — using the
//! compatibility-jamo alphabet (U+3131–U+314E / U+314F–U+3163)
//! that typing a Korean keyboard produces. The query is run through
//! the same normalizers before going to FTS5, so the query and the
//! index always speak the same alphabet. No external Hangul library
//! is needed — all of this is simple codepoint math off the Unicode
//! Hangul Syllables block at U+AC00–U+D7A3.
//!
//! ```text
//!   syllable = 0xAC00 + (cho * 588) + (jung * 28) + jong
//! ```
//!
//! Everything outside the Hangul Syllables block is passed through
//! unchanged. Latin, digits, emoji, CJK Han, kana all survive so
//! mixed queries still work.

/// Compatibility-jamo table for the 19 initial consonants
/// (choseong). Index matches the `cho` component of the syllable
/// decomposition formula.
const CHOSEONG_COMPAT: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ',
    'ㅌ', 'ㅍ', 'ㅎ',
];

/// Compatibility-jamo table for the 21 vowels (jungseong).
const JUNGSEONG_COMPAT: [char; 21] = [
    'ㅏ', 'ㅐ', 'ㅑ', 'ㅒ', 'ㅓ', 'ㅔ', 'ㅕ', 'ㅖ', 'ㅗ', 'ㅘ', 'ㅙ', 'ㅚ', 'ㅛ', 'ㅜ', 'ㅝ', 'ㅞ',
    'ㅟ', 'ㅠ', 'ㅡ', 'ㅢ', 'ㅣ',
];

/// Compatibility-jamo table for the 28 trailing consonants
/// (jongseong). Index 0 means "no final consonant" and maps to the
/// empty string.
const JONGSEONG_COMPAT: [&str; 28] = [
    "", "ㄱ", "ㄲ", "ㄳ", "ㄴ", "ㄵ", "ㄶ", "ㄷ", "ㄹ", "ㄺ", "ㄻ", "ㄼ", "ㄽ", "ㄾ", "ㄿ", "ㅀ",
    "ㅁ", "ㅂ", "ㅄ", "ㅅ", "ㅆ", "ㅇ", "ㅈ", "ㅊ", "ㅋ", "ㅌ", "ㅍ", "ㅎ",
];

const HANGUL_SYLLABLE_START: u32 = 0xAC00;
const HANGUL_SYLLABLE_END: u32 = 0xD7A3;

fn is_hangul_syllable(c: char) -> bool {
    let cp = c as u32;
    (HANGUL_SYLLABLE_START..=HANGUL_SYLLABLE_END).contains(&cp)
}

/// Decomposes a Hangul syllable into its compat-jamo components.
/// Returns `(cho, jung, jong)` where `jong` is an empty string for
/// syllables with no trailing consonant.
fn split_syllable(c: char) -> (char, char, &'static str) {
    let offset = (c as u32) - HANGUL_SYLLABLE_START;
    let cho = (offset / 588) as usize;
    let jung = ((offset % 588) / 28) as usize;
    let jong = (offset % 28) as usize;
    (
        CHOSEONG_COMPAT[cho],
        JUNGSEONG_COMPAT[jung],
        JONGSEONG_COMPAT[jong],
    )
}

/// Decompose every Hangul syllable in `text` into its sequence of
/// compat-jamo characters. Non-Hangul code points are passed through
/// unchanged. Trigram FTS5 over the output lets queries like
/// `ㅅㅏㅁ` match the syllable `삼`.
pub fn decompose_jamo(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        if is_hangul_syllable(c) {
            let (cho, jung, jong) = split_syllable(c);
            out.push(cho);
            out.push(jung);
            out.push_str(jong);
        } else {
            out.push(c);
        }
    }
    out
}

/// True if the string contains any standalone compat-jamo. A mixed
/// query like `ㅅ전자` still benefits from the jamo index because
/// the bare jamo won't match the syllable form.
pub fn contains_bare_jamo(query: &str) -> bool {
    query.chars().any(|c| {
        CHOSEONG_COMPAT.contains(&c)
            || JUNGSEONG_COMPAT.contains(&c)
            || JONGSEONG_COMPAT.iter().any(|j| j.starts_with(c))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decomposes_single_syllable() {
        assert_eq!(decompose_jamo("삼"), "ㅅㅏㅁ");
        assert_eq!(decompose_jamo("가"), "ㄱㅏ");
        assert_eq!(decompose_jamo("닭"), "ㄷㅏㄺ");
    }

    #[test]
    fn decomposes_word() {
        assert_eq!(decompose_jamo("삼성전자"), "ㅅㅏㅁㅅㅓㅇㅈㅓㄴㅈㅏ");
    }

    #[test]
    fn decomposes_mixed_text() {
        assert_eq!(decompose_jamo("삼성 galaxy"), "ㅅㅏㅁㅅㅓㅇ galaxy");
    }

    #[test]
    fn detects_bare_jamo() {
        assert!(contains_bare_jamo("ㅅ전자"));
        assert!(contains_bare_jamo("ㅅㅏㅁ"));
        assert!(!contains_bare_jamo("삼성전자"));
        assert!(!contains_bare_jamo("samsung"));
    }
}
