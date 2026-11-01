use super::*;
use crate::workspace::{Workspace, WorkspaceTab};

fn items(ids: &[&str]) -> Vec<SwitcherItem> {
    ids.iter()
        .map(|id| SwitcherItem {
            id: (*id).into(),
            label: (*id).into(),
            icon: "Terminal",
        })
        .collect()
}

fn ctrl() -> Modifiers {
    Modifiers {
        ctrl: true,
        ..Modifiers::NONE
    }
}

fn workspace(ids: &[&str]) -> Workspace {
    let mut workspace =
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal("s0".into())]));
    workspace.tabs = ids
        .iter()
        .map(|id| WorkspaceTab {
            id: (*id).into(),
            name: None,
            pinned: false,
            primary: Some(Tab::Terminal((*id).into())),
            layout: egui_dock::DockState::new(vec![Tab::Terminal((*id).into())]),
        })
        .collect();
    workspace.recent_tabs.clear();
    workspace
}

#[test]
fn mru_order_puts_active_first_then_recent_then_unvisited() {
    let mut workspace = workspace(&["a", "b", "c", "d"]);
    for id in ["b", "d", "a"] {
        workspace.active = id.into();
        workspace.record_active();
    }
    // A stale ID from a closed tab is skipped.
    workspace.recent_tabs.push("gone".into());
    assert_eq!(workspace.mru_ids(), ["a", "d", "b", "c"]);
}

#[test]
fn forward_opens_on_previous_tab_and_wraps() {
    let mut switcher = Switcher::open("p".into(), items(&["a", "b", "c"]), false, ctrl()).unwrap();
    assert_eq!(switcher.selected(), Some("b"));
    switcher.step(false);
    assert_eq!(switcher.selected(), Some("c"));
    switcher.step(false);
    assert_eq!(switcher.selected(), Some("a"));
}

#[test]
fn backward_opens_on_oldest_tab_and_wraps() {
    let mut switcher = Switcher::open("p".into(), items(&["a", "b", "c"]), true, ctrl()).unwrap();
    assert_eq!(switcher.selected(), Some("c"));
    switcher.step(true);
    switcher.step(true);
    assert_eq!(switcher.selected(), Some("a"));
    switcher.step(true);
    assert_eq!(switcher.selected(), Some("c"));
}

#[test]
fn single_tab_does_not_open_switcher() {
    assert!(Switcher::open("p".into(), items(&["a"]), false, ctrl()).is_none());
}

#[test]
fn switcher_stays_open_only_while_opening_modifier_is_held() {
    let switcher = Switcher::open("p".into(), items(&["a", "b"]), false, ctrl()).unwrap();
    assert!(switcher.held(ctrl()));
    assert!(switcher.held(Modifiers {
        shift: true,
        ..ctrl()
    }));
    assert!(!switcher.held(Modifiers::NONE));
    assert!(!switcher.held(Modifiers::SHIFT));
}
