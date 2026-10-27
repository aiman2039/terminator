use super::*;

#[test]
fn sidebar_and_menu_shortcuts_run_their_actions() {
    let (mut app, ctx, _dir) = fixture();
    assert!(app.preferences.left_visible);
    assert!(app.preferences.visible);
    app.run_shortcut(&ctx, Action::ToggleLeftSidebar);
    app.run_shortcut(&ctx, Action::ToggleRightSidebar);
    assert!(!app.preferences.left_visible);
    assert!(!app.preferences.visible);
    app.state.sessions = vec![session_fixture("live", SessionKind::Shell)];
    app.active_session = Some("live".into());
    app.run_shortcut(&ctx, Action::SearchScrollback);
    assert!(app.search_open);
    assert_eq!(app.search_session.as_deref(), Some("live"));
    app.run_shortcut(&ctx, Action::RenameTerminal);
    assert_eq!(
        app.rename_session.as_ref().map(|(id, _)| id.as_str()),
        Some("live")
    );
    app.rename_session = None;
    app.run_shortcut(&ctx, Action::CloseSession);
    assert_eq!(app.close_session.as_deref(), Some("live"));
    assert!(app.active_editor_id().is_none());
    app.run_shortcut(&ctx, Action::EditorSave);
    assert_eq!(app.close_session.as_deref(), Some("live"));
}

#[test]
#[cfg(feature = "test-support")]
fn toolbar_switches_projects_opens_git_and_search() {
    let (mut app, ctx, _dir) = fixture();
    app.metadata = Some(terminator_core::metadata::Metadata {
        cwd: PathBuf::from("/a"),
        branch: Some("main".into()),
        ..Default::default()
    });
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::NativeEditor {
            path: PathBuf::from("/a/src/main.rs"),
        }])),
    );
    paint_header(&mut app, &ctx, &[]);
    paint_header(&mut app, &ctx, &[]);
    let project = header_target(&ctx)("toolbar-project").expect("project switcher");
    let branch = header_target(&ctx)("toolbar-branch").expect("branch button");
    let search = header_target(&ctx)("toolbar-search").expect("search button");
    assert!(project.left() < branch.left() && branch.left() < search.left());
    let click = |app: &mut App, rect: egui::Rect| {
        let pos = rect.center();
        paint_header(app, &ctx, &[egui::Event::PointerMoved(pos)]);
        paint_header(
            app,
            &ctx,
            &[egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
        );
        paint_header(
            app,
            &ctx,
            &[egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
        );
    };
    // The switcher menu lists projects; picking one selects it.
    click(&mut app, project);
    paint_header(&mut app, &ctx, &[]);
    let other = header_target(&ctx)("b").expect("project b menu item");
    click(&mut app, other);
    assert_eq!(app.selected.as_deref(), Some("b"));
    app.selected = Some("a".into());
    click(&mut app, branch);
    assert_eq!(app.preferences.tool, SidebarTool::Git);
    assert!(app.preferences.visible);
    assert!(!app.preferences.right_projects);
    click(&mut app, search);
    assert!(app.palette.open);
    // Status context: breadcrumb names the project and active file.
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 40.0),
            )),
            ..Default::default()
        },
        |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                app.status_context(ui);
            });
        },
    );
    output.textures_delta.clear();
    paint_header(&mut app, &ctx, &[]);
    assert!(header_target(&ctx)("status-breadcrumb").is_some());
    assert!(header_target(&ctx)("status-breadcrumb-project").is_some());
}

#[test]
fn strip_input_stays_enabled_under_center_views() {
    let (mut app, _ctx, _dir) = fixture();
    assert!(app.strip_terminal_input_enabled("shell"));
    app.settings_open = true;
    assert!(app.strip_terminal_input_enabled("shell"));
    app.player_open = true;
    assert!(app.strip_terminal_input_enabled("shell"));
    app.palette.open = true;
    assert!(app.strip_terminal_input_enabled("shell"));
    app.palette.open = false;
    app.player_open = false;
    app.settings_open = false;
    // True modals still suspend the strip.
    app.add_project = true;
    assert!(!app.strip_terminal_input_enabled("shell"));
}

#[test]
fn hover_popup_blocks_terminal_input_through_its_closing_frame() {
    let (mut app, ctx, _dir) = fixture();
    app.hover_popup = Some(HoverPopup {
        session: "shell".into(),
        key: "first-file".into(),
        target: services::Target::File("/first.rs".into(), None, None),
        rect: egui::Rect::ZERO,
    });
    assert!(!app.terminal_input_enabled("shell"));
    assert!(!app.strip_terminal_input_enabled("other-shell"));
    assert!(app.terminal_copy_enabled("shell"));
    assert!(app.strip_terminal_copy_enabled("shell"));
    assert!(app.float_terminal_copy_enabled("shell"));
    app.hover_popup_blocks_input = true;
    app.dismiss_hover_popup(&ctx);
    assert!(!app.terminal_input_enabled("shell"));
    assert!(!app.strip_terminal_input_enabled("other-shell"));
    assert!(app.terminal_copy_enabled("shell"));
    assert!(app.strip_terminal_copy_enabled("shell"));
    assert!(app.float_terminal_copy_enabled("shell"));
    app.hover_popup_blocks_input = false;
    assert!(app.terminal_input_enabled("shell"));
    assert!(app.strip_terminal_input_enabled("other-shell"));
}

#[test]
fn copy_still_respects_modal_and_editor_close_gates() {
    let (mut app, _ctx, _dir) = fixture();
    app.picker_active = true;
    assert!(!app.terminal_copy_enabled("shell"));
    assert!(!app.strip_terminal_copy_enabled("shell"));
    assert!(!app.float_terminal_copy_enabled("shell"));
    app.picker_active = false;
    app.editor_close_sessions.insert("shell".into());
    assert!(!app.terminal_copy_enabled("shell"));
    assert!(!app.strip_terminal_copy_enabled("shell"));
    assert!(!app.float_terminal_copy_enabled("shell"));
    assert!(app.terminal_copy_enabled("other"));
    app.editor_close_sessions.clear();
    app.settings_open = true;
    assert!(!app.terminal_copy_enabled("shell"));
    assert!(app.strip_terminal_copy_enabled("shell"));
    assert!(app.float_terminal_copy_enabled("shell"));
}

#[test]
#[cfg(feature = "test-support")]
fn header_paints_a_sidebar_toggle_on_each_side() {
    let (mut app, ctx, _dir) = fixture();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 40.0),
            )),
            ..Default::default()
        },
        |ui| app.window_header(ui),
    );
    output.textures_delta.clear();
    let rect = |name: &str| {
        ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", name))))
    };
    let left = rect("toggle-left-sidebar").expect("left toggle");
    let right = rect("toggle-right-sidebar").expect("right toggle");
    assert!(left.right() < right.left());
    assert!(left.width() > 0.0 && right.width() > 0.0);
    app.preferences.left_visible = false;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 40.0),
            )),
            ..Default::default()
        },
        |ui| app.window_header(ui),
    );
    output.textures_delta.clear();
    assert!(rect("toggle-left-sidebar").is_some());
    assert!(rect("toggle-right-sidebar").is_some());
}

/// The tab strip must sit below the 40px drag band (or press-and-move
/// gestures on tabs move the whole window instead of reordering tabs)
/// and beside the sidebars, which run full height underneath the band.
#[test]
#[cfg(feature = "test-support")]
fn tab_strip_sits_below_the_drag_band_and_beside_full_height_sidebars() {
    let (mut app, ctx, _dir) = fixture();
    let workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 600.0),
            )),
            ..Default::default()
        },
        |ui| {
            // Same order as the real panels: drag band, sidebars,
            // then the center-only tab strip.
            egui::Panel::top("window-header")
                .exact_size(40.0)
                .frame(egui::Frame::NONE)
                .show(ui, |ui| app.window_header(ui));
            assert!(
                header_target(&ctx)("workspace-strip").is_none(),
                "the drag band must not paint the tab strip"
            );
            egui::Panel::left("projects")
                .exact_size(225.0)
                .frame(egui::Frame::NONE)
                .show(ui, |ui| {
                    ui.label("sidebar");
                });
            egui::Panel::top("workspace-tabs")
                .exact_size(36.0)
                .frame(egui::Frame::NONE)
                .show(ui, |ui| app.window_header_tabs(ui));
        },
    );
    output.textures_delta.clear();
    let strip = header_target(&ctx)("workspace-strip").expect("strip geometry");
    assert!(
        strip.top() >= 40.0,
        "tab strip must clear the native drag band, got {strip:?}"
    );
    assert!(
        strip.left() >= 225.0,
        "tab strip must sit beside the full-height sidebar, got {strip:?}"
    );
}

#[test]
#[cfg(feature = "test-support")]
fn narrow_header_puts_search_behind_the_overflow_menu() {
    let (mut app, ctx, _dir) = fixture();
    app.preferences.width = UiPreferences::default().width;
    paint_header(&mut app, &ctx, &[]);
    let rect = header_target(&ctx);
    assert!(rect("tool-Info").is_some());
    assert!(rect("palette").is_none(), "search must leave the bar");
    assert!(rect("settings").is_some());
    let overflow = rect("header-overflow").expect("overflow menu");
    let pos = overflow.center();
    paint_header(&mut app, &ctx, &[egui::Event::PointerMoved(pos)]);
    paint_header(
        &mut app,
        &ctx,
        &[egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        }],
    );
    paint_header(
        &mut app,
        &ctx,
        &[egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        }],
    );
    // The opening frame is a sizing pass. Read the item after the menu settles.
    paint_header(&mut app, &ctx, &[]);
    let search = rect("palette").expect("search in the menu");
    assert!(!app.palette.open);
    let pos = search.center();
    paint_header(&mut app, &ctx, &[egui::Event::PointerMoved(pos)]);
    paint_header(
        &mut app,
        &ctx,
        &[egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        }],
    );
    paint_header(
        &mut app,
        &ctx,
        &[egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::default(),
        }],
    );
    assert!(app.palette.open);
}

#[test]
#[cfg(feature = "test-support")]
fn wide_header_shows_search_without_an_overflow_menu() {
    let (mut app, ctx, _dir) = fixture();
    app.preferences.width = 480.0;
    paint_header(&mut app, &ctx, &[]);
    let rect = header_target(&ctx);
    assert!(rect("palette").is_some());
    assert!(rect("header-overflow").is_none());
}

#[test]
fn command_dialogs_suspend_terminal_input_until_dismissed() {
    let (mut app, _, _dir) = fixture();
    assert!(app.terminal_input_enabled("shell"));
    app.palette.open = true;
    assert!(!app.terminal_input_enabled("shell"));
    app.palette.open = false;
    assert!(app.terminal_input_enabled("shell"));
    app.player_open = true;
    assert!(!app.terminal_input_enabled("shell"));
    app.player_open = false;
    assert!(app.terminal_input_enabled("shell"));
    app.worktree_draft = Some(worktree_ui::WorktreeDraft {
        source: "a".into(),
        start: "HEAD".into(),
        branch: "task".into(),
        dest: "/tmp/task".into(),
        open_terminal: true,
    });
    assert!(!app.terminal_input_enabled("shell"));
    app.worktree_draft = None;
    app.worktree_remove = Some("a".into());
    assert!(!app.terminal_input_enabled("shell"));
    app.worktree_remove = None;
    assert!(app.terminal_input_enabled("shell"));
    app.search_open = true;
    assert!(!app.terminal_input_enabled("shell"));
    app.search_open = false;
    assert!(app.terminal_input_enabled("shell"));
    app.worktree_open = true;
    assert!(!app.terminal_input_enabled("shell"));
    app.worktree_open = false;
    assert!(app.terminal_input_enabled("shell"));
}

#[test]
fn closing_scrollback_search_restores_terminal_input() {
    let (mut app, _, _dir) = fixture();
    app.active_session = Some("live".into());
    app.search_active_scrollback(&egui::Context::default());
    assert!(app.search_open);
    assert_eq!(app.search_session.as_deref(), Some("live"));
    assert!(!app.terminal_input_enabled("live"));
    assert!(app.strip_terminal_input_enabled("live"));

    app.hide_center_overlay();
    assert!(!app.search_open);
    assert!(app.search_session.is_none());
    assert!(app.terminal_input_enabled("live"));
    assert!(app.strip_terminal_input_enabled("live"));

    // The session id is not itself a lock. A close that only cleared
    // `search_open` used to leave every main terminal disabled.
    app.search_session = Some("live".into());
    assert!(app.terminal_input_enabled("live"));
    app.search_open = true;
    assert!(!app.terminal_input_enabled("live"));
    app.close_scrollback_search();
    assert!(app.search_session.is_none());
    assert!(app.terminal_input_enabled("live"));
}

#[test]
fn select_all_yields_to_focused_text_field() {
    let (app, ctx, _dir) = fixture();
    assert!(!app.text_input_focused(&ctx));
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let mut text = String::new();
        let response =
            ui.add(appearance::singleline(&mut text).id(egui::Id::unique("select-all-probe")));
        response.request_focus();
    });
    output.textures_delta.clear();
    assert!(app.text_input_focused(&ctx));
}

#[test]
fn select_all_yields_to_focused_native_editor() {
    let (mut app, ctx, _dir) = fixture();
    assert!(!app.text_input_focused(&ctx));
    let path = PathBuf::from("/a/file.rs");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::new(path.clone(), false),
    );
    let mut doc = terminator_native_edit::doc::Doc::new("source");
    let mut engine = terminator_native_edit::vim::VimEngine::new();
    let mut highlight = terminator_native_edit::highlight::HighlightCache::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        terminator_native_edit::view::show_source(
            ui,
            &mut doc,
            &mut engine,
            &terminator_native_edit::view::SourceOptions::default(),
            &mut highlight,
            terminator_native_edit::highlight::Language::Rust,
            "file.rs#split-pane",
        );
    });
    output.textures_delta.clear();
    assert!(app.text_input_focused(&ctx));
}

#[test]
fn opening_scrollback_search_focuses_query_field() {
    let (mut app, ctx, _dir) = fixture();
    app.active_session = Some("live".into());
    app.open_scrollback_search(&ctx, "live");
    assert!(app.search_open);
    assert_eq!(
        ctx.memory(eframe::egui::Memory::focused),
        Some(egui::Id::unique("scrollback-search"))
    );
}

#[test]
fn raw_input_hook_preserves_renderer_texture_limit() {
    let (mut app, ctx, _dir) = fixture();
    // The hook must not rewrite the renderer limit: immediate
    // (floating) viewports bypass the hook, so a parent-only value
    // would alternate the shared font atlas every frame.
    let mut input = egui::RawInput {
        max_texture_side: Some(16_384),
        ..Default::default()
    };
    eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
    assert_eq!(input.max_texture_side, Some(16_384));
    let mut missing = egui::RawInput::default();
    eframe::App::raw_input_hook(&mut app, &ctx, &mut missing);
    assert_eq!(missing.max_texture_side, None);
}
