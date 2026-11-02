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

    #[test]
    fn stepping_without_matches_stays_at_zero() {
        let mut find = TerminalFind::default();
        find.step(1);
        assert_eq!(find.current, 0);
    }
}
