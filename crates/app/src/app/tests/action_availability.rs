//! #78: shortcuts, the palette and context menus share one availability
//! rule and one presentation per action.
use super::*;
use crate::palette::PaletteItem;

fn palette_lists(app: &App, action: Action) -> bool {
    app.palette_items()
        .iter()
        .any(|item| matches!(item, PaletteItem::Action(listed) if *listed == action))
}

#[test]
fn palette_hides_move_actions_that_shortcuts_would_ignore() {
    let (mut app, _ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.state.sessions = vec![session_fixture("main", SessionKind::Shell)];
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![Tab::Terminal("main".into())])),
    );
    app.active_session = Some("main".into());
    // IDE mode off: there is no lower pane and nothing in a strip.
    assert!(!app.shortcut_applies(Action::MoveToStrip));
    assert!(!palette_lists(&app, Action::MoveToStrip));
    assert!(!palette_lists(&app, Action::MoveToMain));
    app.preferences.ide_mode = true;
    assert!(app.shortcut_applies(Action::MoveToStrip));
    assert!(palette_lists(&app, Action::MoveToStrip));
    assert!(!palette_lists(&app, Action::MoveToMain));
}

#[test]
fn session_actions_need_a_focused_session_everywhere() {
    let (mut app, _ctx, _dir) = fixture();
    app.active_session = None;
    for &action in crate::app::commands::SESSION_ACTIONS {
        assert!(!app.shortcut_applies(action), "{}", action.id());
        assert!(!palette_lists(&app, action), "{}", action.id());
    }
    app.state.sessions = vec![session_fixture("s", SessionKind::Shell)];
    app.active_session = Some("s".into());
    assert!(app.shortcut_applies(Action::CopyWorkingDirectory));
    assert!(palette_lists(&app, Action::CopyWorkingDirectory));
}

#[test]
fn editor_actions_follow_the_targeted_session() {
    let (app, _ctx, _dir) = fixture();
    let shell = session_fixture("shell", SessionKind::Shell);
    let editor = session_fixture("edit", SessionKind::Editor);
    let mut review = session_fixture("review", SessionKind::Editor);
    review.review = true;
    for action in [Action::EditorSave, Action::CompareDisk] {
        assert!(!app.session_action_applies(action, &shell));
        assert!(app.session_action_applies(action, &editor));
        assert!(!app.session_action_applies(action, &review));
    }
}

#[test]
fn copy_directory_uses_the_remote_root_for_ssh_sessions() {
    let local = session_fixture("local", SessionKind::Shell);
    let mut remote = session_fixture("remote", SessionKind::Shell);
    remote.remote = Some(terminator_core::remote::RemoteSession {
        ssh: terminator_core::ssh::SshOptions::default(),
        root: "/srv/project".into(),
        cwd: "/srv/project/sub".into(),
        editor_socket: None,
    });
    assert_eq!(crate::app::session_directory(&local), "/a");
    assert_eq!(crate::app::session_directory(&remote), "/srv/project");
    assert_eq!(
        crate::app::copy_directory_label(&local),
        Action::CopyWorkingDirectory.label()
    );
    assert_eq!(
        crate::app::copy_directory_label(&remote),
        "Copy project directory"
    );
}

#[test]
fn every_action_has_a_known_icon_and_a_unique_id() {
    let mut ids = std::collections::HashSet::new();
    for &action in Action::ALL {
        assert!(ids.insert(action.id()), "duplicate id {}", action.id());
        assert!(
            crate::icons::is_ui_icon(action.icon()),
            "{} uses unknown icon {}",
            action.id(),
            action.icon()
        );
    }
}
