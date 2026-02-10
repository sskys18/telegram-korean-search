use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera::tokenizer::Tokenizer as LinderaTokenizer;

/// Korean POS tags (ko-dic mecab format) to keep: nouns, numerals, proper nouns.
/// NNG = common noun, NNP = proper noun, NNB = dependent noun,
/// NR = numeral, SL = foreign word (Latin), SN = number.
const KEEP_POS: &[&str] = &["NNG", "NNP", "NNB", "NR", "SL", "SN"];

pub struct Tokenizer {
    lindera: LinderaTokenizer,
}

impl Default for Tokenizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Tokenizer {
    pub fn new() -> Self {
        let dictionary =
            load_dictionary("embedded://ko-dic").expect("failed to load ko-dic dictionary");
        let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
        let lindera = LinderaTokenizer::new(segmenter);
        Self { lindera }
    }

    /// Tokenize text into searchable terms.
    /// - Korean text: morpheme analysis, keep only nouns/numerals/proper nouns.
    /// - English/Latin text: lowercase, strip punctuation.
    /// - Mixed text: both pipelines run on their respective segments.
    pub fn tokenize(&self, text: &str) -> Vec<String> {
        let mut result = Vec::new();

        match self.lindera.tokenize(text) {
            Ok(tokens) => {
                for mut token in tokens {
                    // Copy surface before mutable borrow for details()
                    let surface = token.surface.as_ref().to_string();
                    let details = token.details();

                    if details.is_empty() || details[0] == "UNK" {
                        // Unknown token — try as English or fallback
                        let lower = surface.to_lowercase();
                        let cleaned = strip_punctuation(&lower);
                        if !cleaned.is_empty() {
                            result.push(cleaned);
                        }
                        continue;
                    }

                    let pos = details[0].to_string();

                    if KEEP_POS.iter().any(|&k| pos.starts_with(k)) {
                        let normalized = surface.to_lowercase();
                        if !normalized.is_empty() {
                            result.push(normalized);
                        }
                    }
                    // Skip particles, endings, punctuation, etc.
                }
            }
            Err(_) => {
                // Fallback: simple whitespace split + lowercase
                for word in text.split_whitespace() {
                    let lower = word.to_lowercase();
                    let cleaned = strip_punctuation(&lower);
                    if !cleaned.is_empty() {
                        result.push(cleaned);
                    }
                }
            }
        }

        result
    }
}

fn strip_punctuation(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || is_cjk(*c))
        .collect()
}

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{AC00}'..='\u{D7AF}' | // Hangul Syllables
        '\u{1100}'..='\u{11FF}' | // Hangul Jamo
        '\u{3130}'..='\u{318F}' | // Hangul Compatibility Jamo
        '\u{A960}'..='\u{A97F}' | // Hangul Jamo Extended-A
        '\u{D7B0}'..='\u{D7FF}' | // Hangul Jamo Extended-B
        '\u{4E00}'..='\u{9FFF}' | // CJK Unified Ideographs
        '\u{3400}'..='\u{4DBF}'   // CJK Unified Ideographs Extension A
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_korean_tokenization() {
        let tok = Tokenizer::new();
        let tokens = tok.tokenize("삼성전자 주가가 상승했다");
        // Should extract nouns, remove particles and endings
        assert!(!tokens.is_empty());
        // "상승" should be present as a noun, "했다" should be removed
        // "주가" should be present, "가" (particle) should be removed
    }

    #[test]
    fn test_english_tokenization() {
        let tok = Tokenizer::new();
        let tokens = tok.tokenize("Hello World!");
        // Unknown to ko-dic → fallback path
        let lower: Vec<String> = tokens.iter().map(|t| t.to_lowercase()).collect();
        assert!(lower.contains(&"hello".to_string()));
        assert!(lower.contains(&"world".to_string()));
    }

    #[test]
    fn test_mixed_text() {
        let tok = Tokenizer::new();
        let tokens = tok.tokenize("텔레그램에서 search 테스트");
        assert!(!tokens.is_empty());
        // Should have Korean nouns and English words
        assert!(tokens.iter().any(|t| t == "search"));
    }

    #[test]
    fn test_particles_removed() {
        let tok = Tokenizer::new();
        let tokens = tok.tokenize("텔레그램에서 검색이 안됐다");
        // ko-dic may split "텔레그램" into subwords and "검색이" differently
        // Key check: particles like "에서" should not appear as standalone tokens
        assert!(
            !tokens.iter().any(|t| t == "에서"),
            "Particle '에서' should be filtered out, got {:?}",
            tokens
        );
        // Should have some content tokens
        assert!(!tokens.is_empty(), "Expected some tokens from Korean text");
    }

    #[test]
    fn test_empty_input() {
        let tok = Tokenizer::new();
        let tokens = tok.tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_whitespace_only() {
        let tok = Tokenizer::new();
        let tokens = tok.tokenize("   \t\n  ");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_numbers() {
        let tok = Tokenizer::new();
        let tokens = tok.tokenize("2024년 매출 100억");
        // Should keep numerals
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_strip_punctuation() {
        assert_eq!(strip_punctuation("hello!"), "hello");
        assert_eq!(strip_punctuation("test..."), "test");
        assert_eq!(strip_punctuation("한국어!"), "한국어");
        assert_eq!(strip_punctuation(""), "");
    }
}
