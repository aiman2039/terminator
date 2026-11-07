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
fn cmd_shift_r_renames_a_focused_terminal_and_replaces_in_files_elsewhere() {
    let (mut app, ctx, _dir) = fixture();
    app.state.sessions = vec![session_fixture("live", SessionKind::Shell)];
    let keys = &app.state.settings.keybindings;
    let chord = shortcuts::binding(keys, Action::ReplaceInFiles.id());
    assert_eq!(chord, shortcuts::binding(keys, Action::RenameTerminal.id()));
    let chord = shortcuts::parse(&chord).unwrap();
    let press = |app: &mut App| {
        let event = egui::Event::Key {
            key: chord.key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: chord.modifiers,
        };
        let input = egui::RawInput {
            events: vec![event],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.run_shortcuts(ui.ctx()));
        output.textures_delta.clear();
    };
    let replacing = |app: &App| {
        app.find_in_files
            .as_ref()
            .is_some_and(|find| find.shown && find.replace.on)
    };
    // A focused terminal keeps Rename terminal.
    app.active_session = Some("live".into());
    press(&mut app);
    let renaming = app.rename_session.as_ref().map(|(id, _)| id.as_str());
    assert_eq!(renaming, Some("live"));
    assert!(!replacing(&app));
    app.clear_session_rename();
    // The Explorer tree holds the keyboard, not the terminal.
    let tree = egui::Id::unique("explorer-tree-under-test");
    ctx.memory_mut(|memory| memory.request_focus(tree));
    crate::native_editor::note_explorer_focus(&ctx, tree);
    assert!(!app.terminal_has_keyboard(&ctx));
    ctx.memory_mut(|memory| memory.surrender_focus(tree));
    assert!(app.terminal_has_keyboard(&ctx));
    // The terminal's own find bar still counts as the terminal.
    app.find_in_active_terminal(&ctx);
    let find_bar = egui::Id::unique(("terminal-find", "live"));
    egui::text_edit::TextEditState::default().store(&ctx, find_bar);
    assert!(ctx.memory(|memory| memory.has_focus(find_bar)));
    assert!(app.text_input_focused(&ctx));
    assert!(app.terminal_has_keyboard(&ctx));
    press(&mut app);
    let renaming = app.rename_session.as_ref().map(|(id, _)| id.as_str());
    assert_eq!(renaming, Some("live"));
    assert!(!replacing(&app));
    app.clear_session_rename();
    ctx.memory_mut(|memory| memory.surrender_focus(find_bar));
    // A text field over the terminal pane: Replace in Files takes it.
    let field = egui::Id::unique("text-field-under-test");
    egui::text_edit::TextEditState::default().store(&ctx, field);
    ctx.memory_mut(|memory| memory.request_focus(field));
    press(&mut app);
    assert!(replacing(&app));
    assert!(app.rename_session.is_none());
    // Another pane (a native editor, a diff) has focus: Replace in Files.
    app.find_in_files.as_mut().unwrap().hide();
    ctx.memory_mut(|memory| memory.surrender_focus(field));
    app.active_session = None;
    press(&mut app);
    assert!(replacing(&app));
    assert!(app.rename_session.is_none());
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
