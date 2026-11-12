use super::*;
use crate::workspace::{Workspace, WorkspaceTab};
use egui_dock::DockState;
use std::sync::mpsc;

#[test]
fn press_hides_only_when_the_scratch_terminal_has_focus() {
    assert_eq!(decide(true, Some("s"), Some("s"), Some("a")), Toggle::Hide);
    assert_eq!(
        decide(false, Some("s"), Some("s"), Some("a")),
        Toggle::Focus("s".into()),
        "an unfocused window comes forward"
    );
    assert_eq!(
        decide(true, Some("other"), Some("s"), Some("a")),
        Toggle::Focus("s".into())
    );
    assert_eq!(
        decide(true, None, None, Some("a")),
        Toggle::Create("a".into())
    );
    assert_eq!(decide(true, None, None, None), Toggle::NoProject);
}

#[test]
fn settings_bindings_become_system_hotkeys_with_a_modifier() {
    use global_hotkey::hotkey::{Code, Modifiers};
    let hotkey = to_hotkey("ctrl+alt+T").unwrap();
    assert_eq!(hotkey.key, Code::KeyT);
    assert!(hotkey.mods.contains(Modifiers::CONTROL | Modifiers::ALT));
    assert_eq!(to_hotkey("alt+F12").unwrap().key, Code::F12);
    assert_eq!(to_hotkey("ctrl+`").unwrap().key, Code::Backquote);
    assert_eq!(to_hotkey("ctrl+space").unwrap().key, Code::Space);
    assert!(to_hotkey("T").is_none(), "a bare key is refused");
    assert!(to_hotkey("").is_none());
}

#[test]
fn quick_terminal_is_unbound_by_default() {
    let defaults = terminator_core::Settings::default().keybindings;
    assert!(
        shortcuts::binding(&defaults, Action::QuickTerminal.id()).is_empty(),
        "no system-wide key is taken until the user sets one"
    );
}

#[test]
fn first_press_opens_one_scratch_terminal_and_the_next_finds_it() {
    let (mut app, ctx, _dir) = crate::app::fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.toggle_quick_terminal(&ctx);
    app.toggle_quick_terminal(&ctx);
    let creates: Vec<_> = received
        .try_iter()
        .filter_map(|job| match job {
            Job::Control(request, After::Workspace(tab, _))
                if matches!(*request, Request::Create { editor: false, .. }) =>
            {
                Some(tab)
            }
            _ => None,
        })
        .collect();
    let [tab] = creates.as_slice() else {
        panic!("one scratch terminal while the first is pending: {creates:?}");
    };
    // What `Update::WorkspaceCreated` does with the reply.
    let mut workspace = Workspace::from_layout(DockState::new(Vec::new()));
    workspace.tabs = vec![WorkspaceTab {
        id: tab.clone(),
        name: None,
        pinned: false,
        primary: None,
        layout: DockState::new(vec![Tab::Terminal("scratch".into())]),
    }];
    app.layouts.insert("a".into(), workspace);
    app.adopt_quick_terminal();
    assert_eq!(
        app.preferences.quick_terminal.session.as_deref(),
        Some("scratch")
    );
    let mut state = app.state.clone();
    state.sessions = vec![crate::app::tests::support::session_fixture(
        "scratch",
        SessionKind::Shell,
    )];
    state.revision = state.revision.saturating_add(1);
    app.apply_state(state);
    assert_eq!(app.quick_terminal_session().as_deref(), Some("scratch"));
    app.toggle_quick_terminal(&ctx);
    assert!(
        received
            .try_iter()
            .all(|job| !matches!(job, Job::Control(request, _) if matches!(*request, Request::Create { .. }))),
        "a live scratch terminal is reused"
    );
}
