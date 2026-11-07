use super::*;

#[test]
fn hide_center_overlay_when_settings_dirty_sets_pending_close() {
    let (mut app, _, _dir) = fixture();
    app.open_settings();
    app.settings_draft.shell = "/tmp/custom".into();
    assert!(app.settings_dirty());
    app.hide_center_overlay();
    assert!(app.settings_pending.is_some());
    assert!(!app.settings_open);
}

#[test]
fn open_settings_clears_search_and_worktree() {
    let (mut app, _, _dir) = fixture();
    app.search_open = true;
    app.worktree_open = true;
    app.open_settings();
    assert!(!app.search_open);
    assert!(!app.worktree_open);
    assert!(app.settings_open);
}

#[test]
fn open_player_clears_search_and_worktree() {
    let (mut app, _, _dir) = fixture();
    app.search_open = true;
    app.worktree_open = true;
    app.open_player();
    assert!(!app.search_open);
    assert!(!app.worktree_open);
    assert!(app.player_open);
}

#[test]
fn appearance_preview_cancel_and_external_conflict() {
    let (mut app, ctx, _dir) = fixture();
    app.open_settings();
    app.theme_draft.text = "#123456".into();
    app.preview_appearance(&ctx);
    assert_eq!(app.theme.text, "#123456");
    app.hide_center_overlay();
    app.preview_appearance(&ctx);
    assert_eq!(app.theme, app.theme_committed);
    app.open_settings();
    app.theme_draft.text = "#123456".into();
    let external = AppearanceConfig {
        text: "#654321".into(),
        ..Default::default()
    };
    app.update_tx
        .send(Update::Appearance(Box::new(AppearanceFile {
            config: external.clone(),
            source: "external".into(),
        })))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.theme_conflict);
    assert_eq!(app.theme_draft.text, "#123456");
    assert_eq!(app.theme_committed, external);
}

#[test]
fn opening_settings_keeps_selected_sidebar_and_custom_editor() {
    let (mut app, _, _dir) = fixture();
    app.preferences.tool = SidebarTool::Git;
    app.preferences.visible = false;
    app.state.settings.external_editor = "/custom/editor".into();
    app.state.settings.external_args = vec!["a b".into()];
    app.open_settings();
    assert_eq!(app.preferences.tool, SidebarTool::Git);
    assert!(!app.preferences.visible);
    assert_eq!(app.editor_preset, external_editor::CUSTOM);
    assert_eq!(app.settings_draft.external_args, vec!["a b"]);
}

#[test]
fn settings_and_player_are_center_singletons() {
    let (mut app, _, _dir) = fixture();
    app.open_settings();
    app.settings_draft.shell = "/tmp/custom-shell".into();
    app.settings_section = SettingsSection::Terminal;
    app.settings_search = "shell".into();
    app.hide_center_overlay();
    assert!(!app.settings_open);
    assert!(app.settings_session);
    app.open_settings();
    assert_eq!(app.settings_draft.shell, "/tmp/custom-shell");
    assert_eq!(app.settings_section, SettingsSection::Terminal);
    assert_eq!(app.settings_search, "shell");
    app.open_player();
    assert!(app.player_open);
    assert!(!app.settings_open);
    assert!(app.settings_session);
    app.open_settings();
    assert!(!app.player_open);
    assert_eq!(app.settings_draft.shell, "/tmp/custom-shell");
    app.end_settings_session();
    app.open_settings();
    assert!(app.settings_draft.shell.is_empty());
    assert!(app.settings_search.is_empty());
}

#[test]
fn go_session_hides_center_overlay() {
    let (mut app, _, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.open_player();
    app.go_session("shell");
    assert!(!app.player_open);
    app.open_player();
    assert!(app.player_open);
}

#[test]
fn create_hides_settings_without_ending_session() {
    let (mut app, _, _dir) = fixture();
    app.open_settings();
    app.settings_search = "shell".into();
    app.create(None);
    assert!(!app.settings_open);
    assert!(app.settings_session);
    app.open_settings();
    assert_eq!(app.settings_search, "shell");
}

#[test]
fn hide_center_overlay_closes_command_palette() {
    let (mut app, _, _dir) = fixture();
    app.open_command_palette();
    app.palette.query = "main".into();
    app.hide_center_overlay();
    assert!(!app.palette.open);
    assert!(app.palette.query.is_empty());
}

#[test]
fn palette_takes_focus_only_once_after_opening() {
    let (mut app, ctx, _dir) = fixture();
    app.open_command_palette();
    assert!(app.palette.focus_pending);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.show_palette(ui.ctx()));
    output.textures_delta.clear();
    assert!(!app.palette.focus_pending);
}

#[test]
fn search_popup_floats_near_the_top_with_room_around_it() {
    let content = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
    let popup = crate::palette::palette_rect(content);
    assert!((popup.center().x - content.center().x).abs() < 1.0);
    assert!((popup.width() - 800.0).abs() < 1.0, "{popup:?}");
    assert!(popup.top() > content.top() && popup.top() < content.center().y);
    assert!(popup.bottom() < content.bottom(), "{popup:?}");
    let narrow = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 400.0));
    let popup = crate::palette::palette_rect(narrow);
    assert!(narrow.contains_rect(popup), "{popup:?}");
}

#[test]
fn press_outside_the_search_popup_closes_it_but_the_opening_frame_does_not() {
    let (mut app, ctx, _dir) = fixture();
    app.open_command_palette();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0));
    let inside = crate::palette::palette_rect(screen).center();
    let corner = egui::pos2(4.0, 4.0);
    let press = |pos| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    };
    let frame = |app: &mut crate::App, events| {
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.show_palette(ui.ctx()));
        output.textures_delta.clear();
    };
    // The click that opened the popup lands in its first frame.
    frame(&mut app, press(corner));
    assert!(app.palette.open, "the opening frame keeps the popup");
    frame(&mut app, press(inside));
    assert!(app.palette.open, "a press inside keeps the popup");
    frame(&mut app, press(corner));
    assert!(!app.palette.open, "a press outside closes the popup");
}

#[test]
fn escape_closes_command_palette() {
    let (mut app, ctx, _dir) = fixture();
    app.open_command_palette();
    let input = egui::RawInput {
        events: vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| app.show_palette(ui.ctx()));
    output.textures_delta.clear();
    assert!(!app.palette.open);
}

#[test]
fn navigation_closes_worktree_draft_form() {
    let (mut app, _, _dir) = fixture();
    app.worktree_draft = Some(worktree_ui::WorktreeDraft {
        source: "a".into(),
        start: "HEAD".into(),
        branch: "task".into(),
        dest: "/tmp/task".into(),
        open_terminal: true,
        ..Default::default()
    });
    app.hide_center_overlay();
    assert!(app.worktree_draft.is_none());
    assert!(app.terminal_input_enabled("shell"));
}

#[test]
fn worktree_form_stays_open_until_creation_succeeds() {
    let (mut app, _, _dir) = fixture();
    let draft = worktree_ui::WorktreeDraft {
        source: "a".into(),
        start: "HEAD".into(),
        branch: "task".into(),
        dest: "/tmp/task".into(),
        open_terminal: false,
        ..Default::default()
    };
    app.worktree_draft = Some(draft.clone());
    app.submit_worktree(&draft);
    assert!(app.worktree_draft.is_some());
    let state = app.state.clone();
    app.handle_worktree_created(state, "a".into(), worktree_ui::TaskStart::default());
    assert!(app.worktree_draft.is_none());
}

#[test]
fn worktree_wizard_opens_only_after_the_git_probe_confirms() {
    let (mut app, _, _dir) = fixture();
    let draft = worktree_ui::WorktreeDraft {
        source: "a".into(),
        start: "HEAD".into(),
        branch: "task".into(),
        dest: "/tmp/task".into(),
        open_terminal: true,
        ..Default::default()
    };
    app.apply_worktree_probe(draft.clone(), false);
    assert!(app.worktree_draft.is_none());
    assert!(
        app.info
            .as_deref()
            .is_some_and(|info| info.contains("not a git checkout"))
    );
    app.apply_worktree_probe(draft, true);
    assert!(app.worktree_draft.is_some());
}
