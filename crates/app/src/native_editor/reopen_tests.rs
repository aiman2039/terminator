//! A file reopens where it was left: caret, top line and folds.
use super::test_support::{app, pump};
use super::*;
use crate::preferences::EditorState;

/// A block on lines 0..=10 (a fold candidate), then plain lines.
fn source() -> String {
    format!("{{\n{}}}\n{}", "  inner\n".repeat(9), "line\n".repeat(400))
}

fn state(path: &Path) -> EditorState {
    EditorState {
        path: path.to_owned(),
        line: 200,
        column: 2,
        top: 190,
        // Line 3 is inside the block: a stale fold.
        folds: vec![0, 3],
    }
}

/// Create `path`'s buffer the way a view does, and load it.
fn load(app: &mut App, path: &Path) {
    app.native_doc(path, true);
    let mut native = app.native_docs.remove(path).expect("buffer");
    pump(&mut native, app, |native| native.doc.is_some());
    app.native_docs.insert(path.to_owned(), native);
}

/// Draw project "a" (one tab with `path`) until its buffer loaded, then
/// a few more frames with time moving on so scroll animations finish.
fn show(app: &mut App, path: &Path) {
    let ctx = egui::Context::default();
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::NativeEditor {
            path: path.to_owned(),
        }])),
    );
    let start = Instant::now();
    let mut settled = 0usize;
    let mut time = 0.0;
    while settled < 4 {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out");
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 500.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ui| {
                let mut dock = app.layouts.remove("a").unwrap_or_else(Workspace::empty);
                app.paint_dock(ui, "a", &mut dock);
                app.layouts.insert("a".into(), dock);
            },
        );
        output.textures_delta.clear();
        time += 1.0;
        if app
            .native_docs
            .get(path)
            .is_some_and(|doc| doc.doc.is_some())
        {
            settled = settled.saturating_add(1);
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

#[test]
fn a_reopened_file_gets_back_its_caret_top_line_and_folds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, source()).unwrap();
    let mut app = app(dir.path());
    app.preferences.editor_states.remember(state(&path));
    load(&mut app, &path);
    app.restore_native_state(&path);
    let native = &app.native_docs[&path];
    assert_eq!(
        native.engine.cursor(),
        terminator_native_edit::vim::Cursor { line: 200, col: 2 }
    );
    assert_eq!(native.top_line, Some(190));
    assert_eq!(
        native.highlight.folds().folded_starts(),
        [0],
        "a fold that no longer starts a region is ignored"
    );
    assert!(native.restore.is_none(), "restored once");
    let kept = native.editor_state().expect("loaded");
    assert_eq!(
        kept,
        EditorState {
            folds: vec![0],
            ..state(&path)
        }
    );
}

#[test]
fn the_view_scrolls_to_the_remembered_top_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, source()).unwrap();
    let mut app = app(dir.path());
    app.preferences.editor_states.remember(state(&path));
    show(&mut app, &path);
    let native = &app.native_docs[&path];
    assert_eq!(native.engine.cursor().line, 200);
    assert_eq!(native.highlight.folds().folded_starts(), [0]);
    assert_eq!(native.top_line, None, "the first frame took it");
    assert_eq!(native.engine.scroll_line(), 190);
}

#[test]
fn a_target_line_wins_over_the_remembered_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, source()).unwrap();
    let mut app = app(dir.path());
    app.preferences.editor_states.remember(state(&path));
    // Go to definition, a search hit or `file:line` asked for line 301.
    app.native_pending_line.insert(path.clone(), 300);
    show(&mut app, &path);
    let native = &app.native_docs[&path];
    assert_eq!(native.engine.cursor().line, 300);
    assert!(native.restore.is_none());
    assert!(native.highlight.folds().folded_starts().is_empty());
    let top = native.engine.scroll_line();
    assert!(
        top > 250 && top <= 300,
        "the target line is shown: top {top}"
    );
}

#[test]
fn a_recent_file_picked_in_the_pane_menu_opens_where_it_was_left() {
    let dir = tempfile::tempdir().unwrap();
    let from = dir.path().join("from.txt");
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, source()).unwrap();
    let mut app = app(dir.path());
    app.preferences.editor_states.remember(state(&path));
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::NativeEditor {
            path: from.clone(),
        }])),
    );
    // What the menu queues: the file, with no target position.
    let at = native_lsp::Position {
        line: 0,
        character: 0,
    };
    app.pending_goto.push(lsp_manager::GotoJump {
        from,
        targets: vec![native_lsp::GotoTarget {
            uri: native_lsp::file_uri(&path).expect("uri").to_string(),
            range: native_lsp::Range { start: at, end: at },
        }],
        at_position: false,
    });
    app.drain_pending_native_close();
    assert!(app.native_pending_line.is_empty(), "no target line");
    assert!(app.native_pending_col.is_empty());
    load(&mut app, &path);
    app.restore_native_state(&path);
    assert_eq!(app.native_docs[&path].engine.cursor().line, 200);
}

#[test]
fn dropping_a_buffer_remembers_it_but_an_unloaded_one_keeps_the_old_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, source()).unwrap();
    let waiting = dir.path().join("waiting.txt");
    let mut app = app(dir.path());
    app.preferences.editor_states.remember(state(&waiting));
    load(&mut app, &path);
    if let Some(native) = app.native_docs.get_mut(&path)
        && let Some(buffer) = native.doc.as_mut()
    {
        let _ = native.engine.place_cursor(buffer, 42, 1);
        native.engine.set_scroll_line(30);
    }
    // A buffer that never loaded: its remembered state must survive.
    app.native_doc(&waiting, true);
    // No tab shows either file any more.
    app.prune_native_docs();
    assert!(app.native_docs.is_empty());
    let kept = app.preferences.editor_states.get(&path).expect("kept");
    assert_eq!((kept.line, kept.column, kept.top), (42, 1, 30));
    assert_eq!(
        app.preferences.editor_states.get(&waiting),
        Some(&state(&waiting))
    );
    assert_eq!(app.preferences.editor_states.paths()[0], path.as_path());
}

#[test]
fn exit_preferences_hold_every_open_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, source()).unwrap();
    let mut app = app(dir.path());
    load(&mut app, &path);
    if let Some(native) = app.native_docs.get_mut(&path)
        && let Some(buffer) = native.doc.as_mut()
    {
        let _ = native.engine.place_cursor(buffer, 7, 0);
    }
    let saved = app.preferences_for_exit();
    assert_eq!(
        saved.editor_states.get(&path).map(|kept| kept.line),
        Some(7)
    );
    assert!(
        app.preferences.editor_states.get(&path).is_none(),
        "the live preferences change only when a buffer closes"
    );
}

#[test]
fn a_new_view_starts_with_the_caret_folds_and_top_line_of_the_old_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, source()).unwrap();
    let mut app = app(dir.path());
    load(&mut app, &path);
    let native = app.native_docs.get_mut(&path).expect("buffer");
    native.enter_view("old");
    if let Some(buffer) = native.doc.as_mut() {
        let _ = native.engine.place_cursor(buffer, 50, 0);
        native
            .highlight
            .restore_folds(&*buffer, Language::Plain, &[0]);
    }
    native.engine.set_scroll_line(45);
    // A split, or the file opened again while its buffer stayed.
    native.enter_view("new");
    assert_eq!(native.engine.cursor().line, 50);
    assert_eq!(native.highlight.folds().folded_starts(), [0]);
    assert_eq!(native.top_line, Some(45));
    // A jump that still waits to scroll goes to the next new view.
    native.top_line = None;
    native.engine.request_scroll();
    native.enter_view("third");
    assert!(native.engine.take_scroll_request());
    assert_eq!(native.top_line, None, "the jump decides the scroll");
}
