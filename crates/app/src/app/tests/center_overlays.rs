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
