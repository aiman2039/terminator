use super::*;

const CMD: egui::Modifiers = egui::Modifiers::COMMAND;
const SHIFT: egui::Modifiers = egui::Modifiers::SHIFT;
const NONE: egui::Modifiers = egui::Modifiers::NONE;

fn painted(rows: &[&str]) -> SessionSelection {
    let mut selection = SessionSelection::default();
    for row in rows {
        selection.painted(row);
    }
    selection.begin_frame();
    selection
}

#[test]
fn plain_click_opens_and_selects_only_that_row() {
    let mut selection = painted(&["a", "b", "c"]);
    assert!(selection.click("b", None, NONE));
    assert!(!selection.selected("b"), "one row is no multi-selection");
    assert_eq!(selection.bulk("b"), None);
}

#[test]
fn command_click_adds_the_current_session_and_toggles_rows() {
    let mut selection = painted(&["a", "b", "c"]);
    assert!(!selection.click("c", Some("a"), CMD));
    assert_eq!(selection.bulk("c"), Some(vec!["a".into(), "c".into()]));
    assert!(!selection.click("a", Some("a"), CMD));
    assert!(!selection.selected("c"), "toggling back leaves one row");
}

#[test]
fn shift_click_selects_the_painted_range_from_the_anchor() {
    let mut selection = painted(&["a", "b", "c", "d"]);
    assert!(selection.click("d", None, NONE));
    assert!(!selection.click("b", None, SHIFT));
    assert_eq!(
        selection.bulk("c"),
        Some(vec!["b".into(), "c".into(), "d".into()])
    );
    assert!(!selection.selected("a"));
}

#[test]
fn shift_click_without_anchor_starts_at_the_current_session() {
    let mut selection = painted(&["a", "b", "a", "c"]);
    assert!(!selection.click("c", Some("a"), SHIFT));
    assert_eq!(
        selection.bulk("a"),
        Some(vec!["a".into(), "b".into(), "c".into()])
    );
}

#[test]
fn ended_sessions_leave_the_selection_and_close_queue_runs_in_order() {
    let mut selection = painted(&["a", "b", "c"]);
    selection.click("a", None, NONE);
    selection.click("b", None, CMD);
    selection.click("c", None, CMD);
    selection.retain(|sid| sid != "b");
    assert_eq!(selection.bulk("a"), Some(vec!["a".into(), "c".into()]));
    selection.close_queue = ["a".to_string(), "c".to_string()].into();
    assert_eq!(selection.next_close().as_deref(), Some("a"));
    assert_eq!(selection.next_close().as_deref(), Some("c"));
    assert_eq!(selection.next_close(), None);
}
