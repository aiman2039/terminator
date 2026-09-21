//! Literal in-terminal search over extracted grid rows.
//!
//! The matcher is UI-free so it can be unit tested in isolation. The backend
//! feeds it one string per grid row (scrollback included); column mapping back
//! to grid coordinates happens at the call site. Matching is intentionally
//! per-row: hits spanning wrapped rows are a known v1 limit.

use std::ops::Range;

/// One hit: index of the row in the searched slice plus char offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowMatch {
    pub row: usize,
    pub chars: Range<usize>,
}

/// Upper bound on reported hits; keeps highlight painting bounded.
pub const MAX_MATCHES: usize = 1000;

/// Non-overlapping literal matches of `query` across `rows`.
pub fn find_in_rows(rows: &[&str], query: &str, case_insensitive: bool) -> Vec<RowMatch> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle: Vec<char> = if case_insensitive {
        query.to_lowercase().chars().collect()
    } else {
        query.chars().collect()
    };
    let mut out = Vec::new();
    for (row, text) in rows.iter().enumerate() {
        let haystack: Vec<char> = if case_insensitive {
            text.to_lowercase().chars().collect()
        } else {
            text.chars().collect()
        };
        if haystack.len() < needle.len() {
            continue;
        }
        let mut start = 0;
        while start + needle.len() <= haystack.len() {
            if haystack[start..start + needle.len()] == needle[..] {
                out.push(RowMatch {
                    row,
                    chars: start..start + needle.len(),
                });
                if out.len() >= MAX_MATCHES {
                    return out;
                }
                start += needle.len();
            } else {
                start += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows<'a>(text: &[&'a str]) -> Vec<&'a str> {
        text.to_vec()
    }

    #[test]
    fn empty_query_matches_nothing() {
        assert!(find_in_rows(&rows(&["hello"]), "", false).is_empty());
    }

    #[test]
    fn literal_match_reports_char_offsets() {
        let hits = find_in_rows(&rows(&["hello world", "none"]), "world", false);
        assert_eq!(
            hits,
            vec![RowMatch {
                row: 0,
                chars: 6..11
            }]
        );
    }

    #[test]
    fn case_toggle_changes_results() {
        let data = rows(&["Hello"]);
        assert!(find_in_rows(&data, "hello", false).is_empty());
        assert_eq!(find_in_rows(&data, "hello", true).len(), 1);
    }

    #[test]
    fn matches_do_not_overlap() {
        let hits = find_in_rows(&rows(&["aaa"]), "aa", false);
        assert_eq!(
            hits,
            vec![RowMatch {
                row: 0,
                chars: 0..2
            }]
        );
    }

    #[test]
    fn unicode_offsets_are_char_based() {
        let hits = find_in_rows(&rows(&["日本語テスト"]), "テスト", false);
        assert_eq!(
            hits,
            vec![RowMatch {
                row: 0,
                chars: 3..6
            }]
        );
    }

    #[test]
    fn results_are_capped() {
        let big = "a ".repeat(2000);
        let data = rows(&[big.as_str()]);
        let hits = find_in_rows(&data, "a", false);
        assert_eq!(hits.len(), MAX_MATCHES);
    }
}
