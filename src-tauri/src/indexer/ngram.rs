use unicode_segmentation::UnicodeSegmentation;

/// Generate character bigrams from text using grapheme clusters.
/// "삼성전자" → ["삼성", "성전", "전자"]
/// "ab" → ["ab"]
/// "a" → [] (too short)
pub fn bigrams(text: &str) -> Vec<String> {
    let graphemes: Vec<&str> = text.graphemes(true).collect();
    if graphemes.len() < 2 {
        return vec![];
    }

    let mut result = Vec::with_capacity(graphemes.len() - 1);
    for window in graphemes.windows(2) {
        let mut bigram = String::with_capacity(window[0].len() + window[1].len());
        bigram.push_str(window[0]);
        bigram.push_str(window[1]);
        result.push(bigram);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_korean_bigrams() {
        let result = bigrams("삼성전자");
        assert_eq!(result, vec!["삼성", "성전", "전자"]);
    }

    #[test]
    fn test_two_char() {
        let result = bigrams("삼성");
        assert_eq!(result, vec!["삼성"]);
    }

    #[test]
    fn test_single_char() {
        let result = bigrams("삼");
        assert!(result.is_empty());
    }

    #[test]
    fn test_empty() {
        let result = bigrams("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_english_bigrams() {
        let result = bigrams("hello");
        assert_eq!(result, vec!["he", "el", "ll", "lo"]);
    }

    #[test]
    fn test_mixed_bigrams() {
        let result = bigrams("삼성ab");
        assert_eq!(result, vec!["삼성", "성a", "ab"]);
    }

    #[test]
    fn test_longer_korean() {
        let result = bigrams("삼성전자주가상승");
        assert_eq!(
            result,
            vec!["삼성", "성전", "전자", "자주", "주가", "가상", "상승"]
        );
    }
}
