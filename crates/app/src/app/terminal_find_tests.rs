use super::super::*;
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::window::TERMINAL_FIND_RESCAN;

    #[test]
    fn stepping_wraps_around_matches() {
        let mut find = TerminalFind {
            outcome: egui_term::FindOutcome {
                matches: vec![
                    egui_term::FoundMatch {
                        line: -3,
                        start_col: 0,
                        end_col: 2,
                    },
                    egui_term::FoundMatch {
                        line: 0,
                        start_col: 5,
                        end_col: 7,
                    },
                ],
                truncated: false,
            },
            ..TerminalFind::default()
        };
        find.step(1);
        assert_eq!(find.current, 1);
        find.step(1);
        assert_eq!(find.current, 0);
        find.step(-1);
        assert_eq!(find.current, 1);
    }
    fn searched(query: &str, at: std::time::Instant) -> TerminalFind {
        TerminalFind {
            query: query.into(),
            searched_query: query.into(),
            last_search: Some(at),
            ..TerminalFind::default()
        }
    }

    #[test]
    fn find_does_not_rescan_without_new_output() {
        let now = std::time::Instant::now();
        let find = searched("foo", now.checked_sub(TERMINAL_FIND_RESCAN * 4).unwrap());
        assert_eq!(find.rescan_in(now), None);
    }

    #[test]
    fn find_rescans_at_once_when_query_changes() {
        let now = std::time::Instant::now();
        let mut find = searched("foo", now);
        find.query.push('b');
        assert_eq!(find.rescan_in(now), Some(std::time::Duration::ZERO));
    }

    #[test]
    fn new_output_rescans_no_more_than_once_per_interval() {
        let now = std::time::Instant::now();
        let mut find = searched("foo", now);
        find.content_stale = true;
        assert_eq!(find.rescan_in(now), Some(TERMINAL_FIND_RESCAN));
        let later = now + TERMINAL_FIND_RESCAN;
        assert_eq!(find.rescan_in(later), Some(std::time::Duration::ZERO));
    }

    fn hits(lines: &[i32]) -> egui_term::FindOutcome {
        egui_term::FindOutcome {
            matches: lines
                .iter()
                .map(|&line| egui_term::FoundMatch {
                    line,
                    start_col: 0,
                    end_col: 2,
                })
                .collect(),
            truncated: false,
        }
    }

    #[test]
    fn output_rescan_does_not_scroll_back_to_the_same_hit() {
        let now = std::time::Instant::now();
        let mut find = TerminalFind {
            query: "foo".into(),
            ..TerminalFind::default()
        };
        // A new query reveals its first hit.
        assert_eq!(find.apply_search(hits(&[-10, -2]), now), Some(-10));
        find.step(1);
        // New output shifts the lines; the selected hit stays the same.
        find.content_stale = true;
        assert_eq!(find.apply_search(hits(&[-12, -4, 0]), now), None);
        assert_eq!(find.current, 1);
        // The selected hit left the history: the new selection is revealed.
        find.content_stale = true;
        assert_eq!(find.apply_search(hits(&[-1]), now), Some(-1));
        assert_eq!(find.current, 0);
    }

    #[test]
    fn first_hit_after_output_is_revealed_once() {
        let now = std::time::Instant::now();
        let mut find = TerminalFind {
            query: "foo".into(),
            ..TerminalFind::default()
        };
        assert_eq!(find.apply_search(hits(&[]), now), None);
        assert_eq!(find.apply_search(hits(&[-1]), now), Some(-1));
        assert_eq!(find.apply_search(hits(&[-2]), now), None);
    }

    #[test]
    fn stepping_without_matches_stays_at_zero() {
        let mut find = TerminalFind::default();
        find.step(1);
        assert_eq!(find.current, 0);
    }
}
