use serde::{Deserialize, Serialize};

/// A highlight range representing a match in the text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HighlightRange {
    /// Byte offset of the match start.
    pub start: usize,
    /// Byte offset of the match end (exclusive).
    pub end: usize,
}

/// Find all occurrences of query tokens in the text (case-insensitive).
/// Returns non-overlapping highlight ranges sorted by start position.
pub fn find_highlights(text: &str, tokens: &[String]) -> Vec<HighlightRange> {
    let text_lower = text.to_lowercase();
    let mut ranges: Vec<HighlightRange> = Vec::new();

    for token in tokens {
        let token_lower = token.to_lowercase();
        if token_lower.is_empty() {
            continue;
        }
        let mut search_from = 0;
        while let Some(pos) = text_lower[search_from..].find(&token_lower) {
            let byte_start = search_from + pos;
            let byte_end = byte_start + token_lower.len();
            ranges.push(HighlightRange {
                start: byte_start,
                end: byte_end,
            });
            search_from = byte_end;
        }
    }

    // Sort by start position
    ranges.sort_by_key(|r| r.start);

    // Merge overlapping ranges
    merge_overlapping(&mut ranges);

    ranges
}

fn merge_overlapping(ranges: &mut Vec<HighlightRange>) {
    if ranges.len() <= 1 {
        return;
    }
    let mut merged: Vec<HighlightRange> = Vec::new();
    merged.push(ranges[0].clone());
    for r in ranges.iter().skip(1) {
        let last = merged.last_mut().unwrap();
        if r.start <= last.end {
            last.end = last.end.max(r.end);
        } else {
            merged.push(r.clone());
        }
    }
    *ranges = merged;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_highlight() {
        let ranges = find_highlights("Hello World", &["hello".to_string()]);
        assert_eq!(ranges, vec![HighlightRange { start: 0, end: 5 }]);
    }

    #[test]
    fn test_multiple_tokens() {
        let ranges = find_highlights("Hello World", &["hello".to_string(), "world".to_string()]);
        assert_eq!(
            ranges,
            vec![
                HighlightRange { start: 0, end: 5 },
                HighlightRange { start: 6, end: 11 },
            ]
        );
    }

    #[test]
    fn test_overlapping_ranges_merged() {
        // "abcabc" with tokens "abc" and "bca" → overlapping ranges
        let ranges = find_highlights("abcabc", &["abc".to_string(), "bca".to_string()]);
        // "abc" at 0..3 and 3..6, "bca" at 2..5
        // After merge: 0..6
        assert_eq!(ranges, vec![HighlightRange { start: 0, end: 6 }]);
    }

    #[test]
    fn test_korean_highlight() {
        let text = "삼성전자 주가가 상승했다";
        let ranges = find_highlights(text, &["삼성".to_string()]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 0);
        // "삼성" is 6 bytes in UTF-8
        assert_eq!(ranges[0].end, 6);
    }

    #[test]
    fn test_no_match() {
        let ranges = find_highlights("Hello World", &["xyz".to_string()]);
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_empty_text() {
        let ranges = find_highlights("", &["hello".to_string()]);
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_empty_tokens() {
        let ranges = find_highlights("Hello World", &[]);
        assert!(ranges.is_empty());
    }

    #[test]
    fn test_multiple_occurrences() {
        let ranges = find_highlights("hello hello hello", &["hello".to_string()]);
        assert_eq!(ranges.len(), 3);
    }

    #[test]
    fn test_case_insensitive() {
        let ranges = find_highlights("HELLO hello Hello", &["hello".to_string()]);
        assert_eq!(ranges.len(), 3);
    }
}
