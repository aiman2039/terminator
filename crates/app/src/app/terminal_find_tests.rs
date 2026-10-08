use super::super::*;
#[cfg(test)]
mod tests {
    use super::*;

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
    #[test]
    fn stepping_without_matches_stays_at_zero() {
        let mut find = TerminalFind::default();
        find.step(1);
        assert_eq!(find.current, 0);
    }
}
