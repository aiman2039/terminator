//! `JetBrains` tool window keys: the toggle rule, the side choice, focus
//! handoff, Esc, Alt+F12, Select In, and where the keys are shown.
use super::*;
use crate::app::ide::{StripeTool, ToolPlace, ToolStep};
use crate::palette::PaletteItem;
use crate::shortcuts::Side;

const TOOL_ACTIONS: [Action; 6] = [
    Action::ToolExplorer,
    Action::ToolProblems,
    Action::ToolStructure,
    Action::ToolGit,
    Action::ToolTerminal,
    Action::SelectIn,
];

fn place(left: bool, right: bool, focused: Option<Side>, usual: Side) -> ToolPlace {
    ToolPlace {
        left,
        right,
        focused,
        usual,
    }
}

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// One frame as the app runs it: shortcuts first, then the sidebars.
fn frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 700.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            app.dispatch_shortcuts(ui.ctx(), true);
            app.workspace_sidebars(ui);
        },
    );
    output.textures_delta.clear();
}

fn chord(app: &App, action: Action) -> shortcuts::Chord {
    let value = shortcuts::binding(&app.state.settings.keybindings, action.id());
    shortcuts::parse(&value).unwrap()
}

/// Press the Settings binding of `action`, then the key-release frame.
fn press(app: &mut App, ctx: &egui::Context, action: Action) {
    let chord = chord(app, action);
    frame(
        app,
        ctx,
        vec![egui::Event::Key {
            key: chord.key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: chord.modifiers,
        }],
    );
    frame(app, ctx, Vec::new());
}

/// A stand-in terminal pane that holds keyboard focus.
fn paint_focused_terminal(ctx: &egui::Context) {
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let terminal = ui.allocate_response(egui::vec2(80.0, 40.0), egui::Sense::click());
        shortcuts::mark_terminal(ui.ctx(), terminal.id);
        terminal.request_focus();
    });
    output.textures_delta.clear();
}

fn focused(ctx: &egui::Context) -> Option<egui::Id> {
    ctx.memory(egui::Memory::focused)
}

fn file(name: &str) -> terminator_git::Entry {
    terminator_git::Entry {
        path: Path::new("/a").join(name),
        directory: !name.contains('.'),
        ignored: false,
    }
}

/// An editor on `path` holds the main pane; the Explorer root lists
/// `a.rs` and `b.rs`.
fn editor_on(app: &mut App, path: &str) {
    let mut dock = Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
        path: PathBuf::from(path),
    }]));
    dock.main_surface_mut().set_focused_node(NodeIndex::root());
    app.layouts.insert("a".into(), dock);
    app.active_session = None;
    app.non_terminal_selected = true;
}

fn explorer_fixture() -> (App, egui::Context, tempfile::TempDir) {
    let (mut app, ctx, dir) = fixture();
    let (jobs, _requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.dirs
        .insert(PathBuf::from("/a"), vec![file("a.rs"), file("b.rs")]);
    editor_on(&mut app, "/a/a.rs");
    frame(&mut app, &ctx, Vec::new());
    (app, ctx, dir)
}

#[test]
fn tool_keys_show_then_focus_then_hide_a_tool_window() {
    use Side::{Left, Right};
    // Hidden: show it on its usual side and focus it.
    assert_eq!(
        place(false, false, None, Right).step(),
        ToolStep::Show(Right)
    );
    assert_eq!(
        place(false, false, Some(Right), Left).step(),
        ToolStep::Show(Left)
    );
    // Shown, focus elsewhere (an editor or the other sidebar): focus it.
    assert_eq!(
        place(false, true, None, Right).step(),
        ToolStep::Focus(Right)
    );
    assert_eq!(
        place(false, true, Some(Left), Right).step(),
        ToolStep::Focus(Right)
    );
    // Shown and focused: hide it.
    assert_eq!(
        place(false, true, Some(Right), Right).step(),
        ToolStep::Hide(Right)
    );
}

#[test]
fn a_tool_uses_the_side_where_it_shows_else_its_usual_side() {
    use Side::{Left, Right};
    assert_eq!(place(true, false, None, Right).side(), Left);
    assert_eq!(place(false, false, None, Right).side(), Right);
    assert_eq!(
        place(true, false, None, Right).step(),
        ToolStep::Focus(Left)
    );
    assert_eq!(
        place(true, false, Some(Left), Right).step(),
        ToolStep::Hide(Left)
    );
    // On both sides: the usual one, unless the other one has focus.
    assert_eq!(place(true, true, None, Left).step(), ToolStep::Focus(Left));
    assert_eq!(
        place(true, true, Some(Right), Left).step(),
        ToolStep::Hide(Right)
    );
}

#[test]
fn a_hidden_tool_opens_on_the_side_its_stripe_chose() {
    let (mut app, ctx, _dir) = explorer_fixture();
    // A left stripe put the Explorer in the left sidebar, now closed.
    app.preferences.left_tool = Some(SidebarTool::Explorer);
    app.preferences.left_visible = false;
    app.preferences.tool = SidebarTool::Git;
    assert_eq!(
        app.tool_place(&ctx, SidebarTool::Explorer).usual,
        Side::Left
    );
    assert_eq!(app.tool_place(&ctx, SidebarTool::Git).usual, Side::Right);
    press(&mut app, &ctx, Action::ToolExplorer);
    assert!(app.preferences.left_visible);
    assert_eq!(app.preferences.left_tool, Some(SidebarTool::Explorer));
    assert_eq!(
        app.preferences.tool,
        SidebarTool::Git,
        "the right side stays"
    );
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Left));
    // Problems has no left slot: it opens on the right, in place of Git.
    press(&mut app, &ctx, Action::ToolProblems);
    assert!(app.preferences.visible);
    assert_eq!(app.preferences.tool, SidebarTool::Problems);
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
}

#[test]
fn cmd_1_focuses_the_explorer_tree_so_arrow_keys_work_at_once() {
    let (mut app, ctx, _dir) = explorer_fixture();
    assert_eq!(shortcuts::focused_sidebar(&ctx), None);
    // Shown but not focused: the key focuses the tree, no click needed.
    press(&mut app, &ctx, Action::ToolExplorer);
    assert!(app.preferences.visible);
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
    frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    assert_eq!(app.explorer_selection, [PathBuf::from("/a/a.rs")]);
    frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    assert_eq!(app.explorer_selection, [PathBuf::from("/a/b.rs")]);
    // Focused: the key hides it and the editor gets the keyboard back.
    assert!(app.native_focus_pending.is_none());
    press(&mut app, &ctx, Action::ToolExplorer);
    assert!(!app.preferences.visible);
    assert_eq!(focused(&ctx), None);
    let pending = app.native_focus_pending.as_ref().expect("editor focus");
    assert_eq!(pending.project, "a");
    // Hidden: the key shows it and focuses it again.
    press(&mut app, &ctx, Action::ToolExplorer);
    assert!(app.preferences.visible);
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
}

#[test]
fn escape_in_the_explorer_ends_a_search_then_returns_to_the_editor() {
    let (mut app, ctx, _dir) = explorer_fixture();
    press(&mut app, &ctx, Action::ToolExplorer);
    frame(&mut app, &ctx, vec![egui::Event::Text("b".into())]);
    assert_eq!(app.explorer_selection, [PathBuf::from("/a/b.rs")]);
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert!(app.explorer_speed.is_empty());
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
    assert!(app.native_focus_pending.is_none());
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert_eq!(focused(&ctx), None);
    assert!(app.native_focus_pending.is_some());
    assert_eq!(app.explorer_selection, [PathBuf::from("/a/b.rs")]);
    assert!(app.preferences.visible, "Esc leaves the tool window open");
}

#[test]
fn escape_after_a_timed_out_explorer_search_returns_to_the_editor_at_once() {
    let (mut app, ctx, _dir) = explorer_fixture();
    press(&mut app, &ctx, Action::ToolExplorer);
    frame(&mut app, &ctx, vec![egui::Event::Text("b".into())]);
    assert_eq!(app.explorer_selection, [PathBuf::from("/a/b.rs")]);
    // The search hint is gone after its idle time.
    app.explorer_speed_at = Instant::now()
        .checked_sub(Duration::from_secs(5))
        .expect("instant in range");
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert_eq!(focused(&ctx), None);
    assert!(app.native_focus_pending.is_some());
    assert!(app.explorer_speed.is_empty());
    assert_eq!(app.explorer_selection, [PathBuf::from("/a/b.rs")]);
}

fn git_fixture() -> (App, egui::Context, tempfile::TempDir) {
    let (mut app, ctx, dir) = explorer_fixture();
    let root = PathBuf::from("/a");
    app.context = Some(services::ContextData {
        cwd: root.clone(),
        root: Some(root.clone()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: ["a.rs", "b.rs"]
            .map(|name| terminator_git::Change {
                path: root.join(name),
                status: " M".into(),
            })
            .to_vec(),
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    });
    (app, ctx, dir)
}

#[test]
fn cmd_9_focuses_the_first_change_and_arrow_keys_walk_the_git_panel() {
    let (mut app, ctx, _dir) = git_fixture();
    press(&mut app, &ctx, Action::ToolGit);
    assert_eq!(app.preferences.tool, SidebarTool::Git);
    let rect = |id| ctx.read_response(id).expect("painted").rect;
    let first = rect(focused(&ctx).expect("a focused change"));
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
    // A full-width 20 px change row, not a checkbox or the sidebar itself.
    assert_eq!(first.height(), 20.0);
    assert!(first.width() > 100.0, "{first:?}");
    frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    let second = rect(focused(&ctx).expect("the next change"));
    assert!(
        second.top() > first.top() && second.top() < first.bottom() + first.height(),
        "one row down: {first:?} then {second:?}"
    );
    assert_eq!(second.width(), first.width());
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
    // Esc drops the row's focus; the editor gets the keyboard back.
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert_eq!(focused(&ctx), None);
    assert!(app.native_focus_pending.is_some());
    assert!(app.preferences.visible);
}

/// Escape key presses left for the panes after `run`.
fn escapes_left(ui: &egui::Ui) -> usize {
    ui.input(|input| {
        input
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        ..
                    }
                )
            })
            .count()
    })
}

#[test]
fn escape_that_leaves_a_tool_window_never_reaches_a_terminal() {
    let (mut app, ctx, _dir) = git_fixture();
    app.state.sessions = vec![session_fixture("shell", SessionKind::Shell)];
    press(&mut app, &ctx, Action::ToolGit);
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
    app.active_session = Some("shell".into());
    let mut left = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 700.0),
            )),
            events: vec![key(egui::Key::Escape)],
            ..Default::default()
        },
        |ui| {
            app.dispatch_shortcuts(ui.ctx(), true);
            // The IDE strip can paint before the sidebars: nothing is left
            // for a terminal that takes the free focus now.
            left = Some(escapes_left(ui));
            app.workspace_sidebars(ui);
        },
    );
    output.textures_delta.clear();
    assert_eq!(left, Some(0), "the Esc is used up");
    assert_eq!(focused(&ctx), None);
    assert!(app.preferences.visible, "Esc leaves the tool window open");
}

/// One frame with a floating window whose text field `field` lies over
/// the right sidebar, as the Compare picker can. Returns the Esc presses
/// left after the shortcuts ran.
fn frame_with_window(
    app: &mut App,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    field: egui::Id,
) -> usize {
    let mut left = 0;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 700.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            app.dispatch_shortcuts(ui.ctx(), true);
            left = escapes_left(ui);
            app.workspace_sidebars(ui);
            egui::Window::new("Compare with")
                .fixed_pos(egui::pos2(1000.0, 100.0))
                .show(ui.ctx(), |ui| {
                    let mut text = String::new();
                    ui.add(egui::TextEdit::singleline(&mut text).id(field));
                });
        },
    );
    output.textures_delta.clear();
    left
}

#[test]
fn a_floating_window_over_a_tool_window_keeps_its_focus_and_esc() {
    let (mut app, ctx, _dir) = git_fixture();
    press(&mut app, &ctx, Action::ToolGit);
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
    let field = egui::Id::unique("floating-picker-field");
    // A new window sizes itself invisibly first; then its field takes focus.
    frame_with_window(&mut app, &ctx, Vec::new(), field);
    ctx.memory_mut(|memory| memory.request_focus(field));
    frame_with_window(&mut app, &ctx, Vec::new(), field);
    frame_with_window(&mut app, &ctx, Vec::new(), field);
    assert_eq!(focused(&ctx), Some(field));
    assert_eq!(
        shortcuts::focused_sidebar(&ctx),
        None,
        "a window, not the tool"
    );
    let esc = vec![key(egui::Key::Escape)];
    assert_eq!(frame_with_window(&mut app, &ctx, esc, field), 1, "its Esc");
    // The tool key focuses the tool window under it; it does not hide it.
    ctx.memory_mut(|memory| memory.request_focus(field));
    frame_with_window(&mut app, &ctx, Vec::new(), field);
    frame_with_window(&mut app, &ctx, Vec::new(), field);
    press(&mut app, &ctx, Action::ToolGit);
    assert!(app.preferences.visible);
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
}

#[test]
fn a_shared_chord_in_a_focused_tool_window_is_not_the_terminals() {
    let (mut app, ctx, _dir) = git_fixture();
    app.state.sessions = vec![session_fixture("shell", SessionKind::Shell)];
    press(&mut app, &ctx, Action::ToolGit);
    app.active_session = Some("shell".into());
    assert!(!app.terminal_has_keyboard(&ctx), "a Git row has the keys");
    press(&mut app, &ctx, Action::ReplaceInFiles);
    assert!(app.rename_session.is_none(), "no terminal rename");
    assert!(
        app.find_in_files
            .as_ref()
            .is_some_and(|find| find.shown && find.replace.on)
    );
}

/// The Git panel in history mode with a log longer than the sidebar.
fn git_log_fixture() -> (App, egui::Context, tempfile::TempDir) {
    let (mut app, ctx, dir) = git_fixture();
    app.git.list_root = Some(PathBuf::from("/a"));
    app.git.history = true;
    app.git.log = (0..200)
        .map(|i| (format!("{i:07x}"), format!("Commit {i}")))
        .collect();
    (app, ctx, dir)
}

#[test]
fn arrow_keys_keep_the_focused_tool_window_row_on_screen() {
    let (mut app, ctx, _dir) = git_log_fixture();
    press(&mut app, &ctx, Action::ToolGit);
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
    for _ in 0..60 {
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    }
    // Let the scroll animation (at most 0.3 s) end.
    for _ in 0..30 {
        frame(&mut app, &ctx, Vec::new());
    }
    let row = ctx
        .read_response(focused(&ctx).expect("a focused commit"))
        .expect("painted")
        .rect;
    assert!(
        row.bottom() <= 700.0,
        "the list scrolled to the row: {row:?}"
    );
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
    // Focused: the key hides the tool window.
    press(&mut app, &ctx, Action::ToolGit);
    assert!(!app.preferences.visible);
}

#[test]
fn a_tool_window_key_focuses_a_row_on_screen_in_a_scrolled_list() {
    let (mut app, ctx, _dir) = git_log_fixture();
    press(&mut app, &ctx, Action::ToolGit);
    let first = focused(&ctx).expect("the first commit");
    for _ in 0..60 {
        frame(&mut app, &ctx, vec![key(egui::Key::ArrowDown)]);
    }
    frame(&mut app, &ctx, vec![key(egui::Key::Escape)]);
    assert_eq!(focused(&ctx), None);
    // Shown but not focused: the key focuses a row the user can see, not
    // the first commit that scrolled away.
    press(&mut app, &ctx, Action::ToolGit);
    let row = focused(&ctx).expect("a focused commit");
    assert_ne!(row, first, "the scroll position stays");
    let rect = ctx.read_response(row).expect("painted").rect;
    assert!(rect.top() >= 0.0 && rect.bottom() <= 700.0, "{rect:?}");
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
}

#[test]
fn a_tool_window_with_no_rows_still_takes_focus_so_its_key_hides_it() {
    let (mut app, ctx, _dir) = explorer_fixture();
    press(&mut app, &ctx, Action::ToolProblems);
    assert_eq!(app.preferences.tool, SidebarTool::Problems);
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
    frame(&mut app, &ctx, Vec::new());
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
    press(&mut app, &ctx, Action::ToolProblems);
    assert!(!app.preferences.visible);
}

#[test]
fn alt_f1_selects_the_current_file_in_the_explorer() {
    let (mut app, ctx, _dir) = explorer_fixture();
    app.dirs
        .insert(PathBuf::from("/a"), vec![file("src"), file("a.rs")]);
    app.dirs
        .insert(PathBuf::from("/a/src"), vec![file("src/main.rs")]);
    editor_on(&mut app, "/a/src/main.rs");
    app.preferences.visible = false;
    press(&mut app, &ctx, Action::SelectIn);
    assert!(app.preferences.visible);
    assert_eq!(app.preferences.tool, SidebarTool::Explorer);
    assert!(app.expanded_dirs.contains(Path::new("/a/src")));
    assert_eq!(app.explorer_selection, [PathBuf::from("/a/src/main.rs")]);
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
    assert_eq!(app.info, None);
    // No current file: the Explorer still takes focus and says why.
    app.layouts.insert("a".into(), Workspace::empty());
    press(&mut app, &ctx, Action::SelectIn);
    assert_eq!(
        app.info.as_deref(),
        Some("Focus a local file to reveal it in Explorer")
    );
    assert_eq!(shortcuts::focused_sidebar(&ctx), Some(Side::Right));
}

fn creates(requests: &mpsc::Receiver<Job>) -> Vec<After> {
    requests
        .try_iter()
        .filter_map(|job| match job {
            Job::Control(request, after) if matches!(*request, Request::Create { .. }) => {
                Some(after)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn alt_f12_shows_the_ide_strip_and_hides_it_from_a_strip_shell() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.preferences.ide_mode = true;
    app.preferences.ide_terminal_collapsed = true;
    app.state.sessions = vec![session_fixture("strip", SessionKind::Shell)];
    app.preferences.ide_strip_docks.0.insert(
        "a".into(),
        DockState::new(vec![Tab::Terminal("strip".into())]),
    );
    press(&mut app, &ctx, Action::ToolTerminal);
    assert!(!app.preferences.ide_terminal_collapsed);
    assert_eq!(app.active_session.as_deref(), Some("strip"));
    // The strip shell has the keyboard: the key hides the strip.
    paint_focused_terminal(&ctx);
    press(&mut app, &ctx, Action::ToolTerminal);
    assert!(app.preferences.ide_terminal_collapsed);
    assert_eq!(app.active_session, None);
    // An empty strip opens a shell there.
    app.preferences.ide_strip_docks.0.clear();
    let _ = creates(&requests);
    press(&mut app, &ctx, Action::ToolTerminal);
    assert!(!app.preferences.ide_terminal_collapsed);
    assert!(matches!(creates(&requests)[..], [After::Strip]));
}

#[test]
fn alt_f12_in_regular_mode_focuses_the_last_shell_or_opens_one() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.state.sessions = vec![
        session_fixture("old", SessionKind::Shell),
        session_fixture("last", SessionKind::Shell),
    ];
    editor_on(&mut app, "/a/a.rs");
    app.layouts
        .get_mut("a")
        .unwrap()
        .add(id(), Tab::Terminal("old".into()));
    app.layouts
        .get_mut("a")
        .unwrap()
        .add(id(), Tab::Terminal("last".into()));
    app.terminal_context.insert("a".into(), "last".into());
    app.active_session = None;
    press(&mut app, &ctx, Action::ToolTerminal);
    assert_eq!(app.active_session.as_deref(), Some("last"));
    // A shell that already has the keyboard keeps it.
    paint_focused_terminal(&ctx);
    press(&mut app, &ctx, Action::ToolTerminal);
    assert_eq!(app.active_session.as_deref(), Some("last"));
    assert!(creates(&requests).is_empty());
    // No live shell: open one in a new tab.
    app.state.sessions.clear();
    app.active_session = None;
    press(&mut app, &ctx, Action::ToolTerminal);
    assert!(matches!(creates(&requests)[..], [After::Workspace(..)]));
}

#[test]
fn a_focused_terminal_keeps_linux_alt_digits_but_not_alt_f12() {
    let (mut app, ctx, _dir) = explorer_fixture();
    paint_focused_terminal(&ctx);
    let explorer = chord(&app, Action::ToolExplorer);
    let mut left = 0;
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: explorer.key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: explorer.modifiers,
            }],
            ..Default::default()
        },
        |ui| {
            app.dispatch_shortcuts(ui.ctx(), true);
            left = ui.input(|input| input.events.len());
        },
    );
    output.textures_delta.clear();
    // Linux shells use Alt+1 (Meta-1); macOS Cmd+1 is the app's.
    assert_eq!(left, usize::from(cfg!(not(target_os = "macos"))));
    // Alt+F12 works from a terminal everywhere, so it can leave it.
    assert!(!shortcuts::terminal_keeps(&shortcuts::binding(
        &app.state.settings.keybindings,
        Action::ToolTerminal.id()
    )));
}

#[test]
fn stripe_header_and_palette_show_the_tool_window_keys() {
    let (app, _ctx, _dir) = fixture();
    let keys = |action: Action| app.shortcut_label(action);
    let explorer = if cfg!(target_os = "macos") {
        "Explorer (⌘1)"
    } else {
        "Explorer (Alt+1)"
    };
    assert_eq!(app.tool_tip("Explorer", SidebarTool::Explorer), explorer);
    assert_eq!(
        app.stripe_tip(StripeTool::Tool(SidebarTool::Git), "Git"),
        format!("Git / Perforce ({})", keys(Action::ToolGit))
    );
    assert_eq!(
        app.stripe_tip(StripeTool::Tool(SidebarTool::Structure), "Structure"),
        format!("Structure ({})", keys(Action::ToolStructure))
    );
    // Tools without a key keep their plain name.
    assert_eq!(
        app.stripe_tip(StripeTool::Tool(SidebarTool::Agents), "Agents"),
        "Agents"
    );
    assert_eq!(app.stripe_tip(StripeTool::Projects, "Projects"), "Projects");
    assert_eq!(app.tool_keys(SidebarTool::History), "");
    let items = app.palette_items();
    for action in TOOL_ACTIONS {
        assert!(!keys(action).is_empty(), "{}", action.id());
        assert!(
            items
                .iter()
                .any(|item| matches!(item, PaletteItem::Action(listed) if *listed == action)),
            "{}",
            action.id()
        );
    }
}
