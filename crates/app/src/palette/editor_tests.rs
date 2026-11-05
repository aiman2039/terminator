//! Native editor commands in Find Action: listed only with a native editor
//! on screen, shown with their keys, and run in the editor focused last.
use super::*;
use crate::native_editor::NativeDoc;
use egui_dock::{DockState, NodeIndex, NodePath, SurfaceIndex, TabIndex};
use terminator_native_edit::view::queued_command;

/// An app with project `p` at `dir` selected.
fn app_with_project(dir: &Path) -> (App, egui::Context) {
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.join("state")));
    app.state.projects.push(Project {
        id: "p".into(),
        name: "p".into(),
        path: dir.into(),
        layout: serde_json::Value::Null,
    });
    app.selected = Some("p".into());
    (app, ctx)
}

/// A loaded native editor for `path`, alone in the selected project's
/// workspace, focused in view `view`.
fn open_editor(app: &mut App, path: &Path, view: &str) {
    let mut doc = NativeDoc::loaded(path.to_owned(), "fn main() {}\n");
    doc.focus_for_test(view, Instant::now());
    app.native_docs.insert(path.to_owned(), doc);
    app.layouts.insert(
        "p".into(),
        Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
            path: path.to_owned(),
        }])),
    );
}

fn editor_items(app: &App) -> Vec<EditorCommand> {
    app.filtered_palette()
        .into_iter()
        .filter_map(|item| match item {
            PaletteItem::Editor(command) => Some(command),
            _ => None,
        })
        .collect()
}

#[test]
fn editor_commands_need_a_native_editor_on_screen() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, ctx) = app_with_project(dir.path());
    app.palette.filters = FILTER_ACTIONS;
    assert!(
        editor_items(&app).is_empty(),
        "no editor, no editor commands"
    );
    app.run_palette(&ctx, PaletteItem::Editor(EditorCommand::DuplicateLine));
    assert!(
        app.info
            .as_deref()
            .is_some_and(|info| info.contains("open a file"))
    );
    // Running an item closes the palette; reopen it on Actions.
    app.palette.filters = FILTER_ACTIONS;
    // An editor in another workspace tab is not on screen.
    let path = dir.path().join("main.rs");
    open_editor(&mut app, &path, "main-view");
    let workspace = app.layouts.get_mut("p").unwrap();
    workspace.add("other".into(), Tab::Terminal("shell".into()));
    workspace.active = "other".into();
    assert!(editor_items(&app).is_empty());
    let first = app.layouts["p"].tabs[0].id.clone();
    app.layouts.get_mut("p").unwrap().active = first;
    let listed = editor_items(&app);
    assert!(listed.contains(&EditorCommand::DuplicateLine));
    // Rust has a language server.
    assert!(listed.contains(&EditorCommand::GotoDeclaration));
    // Editor commands never repeat an app action.
    for command in &listed {
        assert!(
            !Action::ALL
                .iter()
                .any(|action| action.label().eq_ignore_ascii_case(command.name())),
            "{command:?} repeats an action"
        );
    }
}

#[test]
fn editor_command_rows_show_their_keys_and_match_other_names() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _ctx) = app_with_project(dir.path());
    app.state.settings.native_vim = false;
    open_editor(&mut app, &dir.path().join("notes.txt"), "notes-view");
    let duplicate = PaletteItem::Editor(EditorCommand::DuplicateLine);
    assert_eq!(duplicate.label(), "Editor  Duplicate Line or Selection");
    let keys = if cfg!(target_os = "macos") {
        "⌘D"
    } else {
        "Ctrl+D"
    };
    assert_eq!(duplicate.detail(&app), keys);
    assert_eq!(
        PaletteItem::Editor(EditorCommand::TypeHierarchy).detail(&app),
        ""
    );
    app.palette.filters = FILTER_ACTIONS;
    let listed = editor_items(&app);
    // Plain text: no language server and no line comments; vim is off,
    // so multiple carets apply.
    assert!(!listed.contains(&EditorCommand::GotoDeclaration));
    assert!(!listed.contains(&EditorCommand::CommentLine));
    assert!(listed.contains(&EditorCommand::AddNextOccurrence));
    app.palette.query = "collapse".into();
    assert!(editor_items(&app).contains(&EditorCommand::Fold));
}

/// Paint the selected project's dock once, as the center pane does.
fn paint_dock(app: &mut App, ctx: &egui::Context) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 480.0),
            )),
            ..Default::default()
        },
        |ui| {
            let mut dock = app.layouts.remove("p").unwrap_or_else(Workspace::empty);
            app.paint_dock(ui, "p", &mut dock);
            app.layouts.insert("p".into(), dock);
        },
    );
    output.textures_delta.clear();
}

#[test]
fn an_editor_command_goes_to_the_view_on_screen() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, ctx) = app_with_project(dir.path());
    app.lsp.set_spell_enabled(false);
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "one\n").unwrap();
    app.native_docs
        .insert(path.clone(), NativeDoc::loaded(path.clone(), "one\n"));
    app.layouts.insert(
        "p".into(),
        Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
            path: path.clone(),
        }])),
    );
    for _ in 0..3 {
        paint_dock(&mut app, &ctx);
    }
    // The view focused last is gone (a split moved its pane).
    if let Some(doc) = app.native_docs.get_mut(&path) {
        doc.focus_for_test("gone", Instant::now());
    }
    let target = app.editor_command_target().expect("an editor on screen");
    assert_ne!(target.view, "gone");
    app.open_command_palette();
    app.run_palette(&ctx, PaletteItem::Editor(EditorCommand::DuplicateLine));
    paint_dock(&mut app, &ctx);
    assert_eq!(
        app.native_docs[&path].buffer().unwrap().text(),
        "one\none\n"
    );
}

#[test]
fn editor_command_runs_in_the_last_focused_editor() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, ctx) = app_with_project(dir.path());
    app.lsp.set_spell_enabled(false);
    let (left, right) = (dir.path().join("left.txt"), dir.path().join("right.txt"));
    for (path, text) in [(&left, "one\n"), (&right, "two\n")] {
        std::fs::write(path, text).unwrap();
        app.native_docs
            .insert(path.clone(), NativeDoc::loaded(path.clone(), text));
    }
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
        path: left.clone(),
    }]));
    let nodes = workspace.main_surface_mut().split_right(
        NodeIndex::root(),
        0.5,
        vec![Tab::NativeEditor {
            path: right.clone(),
        }],
    );
    let group = workspace.tabs[0].id.clone();
    app.layouts.insert("p".into(), workspace);
    let focus = |app: &mut App, node: NodeIndex| {
        let pane = NodePath {
            surface: SurfaceIndex::main(),
            node,
        };
        app.native_focus_pending = Some(PaneSource {
            project: "p".into(),
            group: group.clone(),
            path: (pane, TabIndex(0)).into(),
        });
        paint_dock(app, &ctx);
        paint_dock(app, &ctx);
    };
    // The left editor takes free focus first; then the right one.
    paint_dock(&mut app, &ctx);
    focus(&mut app, nodes[1]);
    // Find Action holds focus now; its command goes to the right editor.
    app.open_command_palette();
    app.run_palette(&ctx, PaletteItem::Editor(EditorCommand::DuplicateLine));
    let target = app.editor_command_target().expect("an editor on screen");
    assert_eq!(target.path, right);
    assert_eq!(
        queued_command(&ctx, &target.view),
        Some(EditorCommand::DuplicateLine)
    );
    paint_dock(&mut app, &ctx);
    assert_eq!(queued_command(&ctx, &target.view), None);
    // Focus moves back left, while the dock still marks the right pane.
    focus(&mut app, nodes[0]);
    app.open_command_palette();
    app.run_palette(&ctx, PaletteItem::Editor(EditorCommand::DuplicateLine));
    paint_dock(&mut app, &ctx);
    let text = |path: &Path| app.native_docs[path].buffer().unwrap().text();
    assert_eq!(text(&left), "one\none\n");
    assert_eq!(text(&right), "two\ntwo\n");
}
