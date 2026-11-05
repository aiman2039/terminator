//! The Markdown toolbar, the status-cell menus, and the pane tab file menu.
//! They read fixture targets, so they need the `test-support` feature.
#![cfg(feature = "test-support")]
use super::*;
use crate::diagnostics::current_rect;

/// An app with an isolated data directory; project "a" at `/a` is
/// selected. The spell checker is off, so no server process starts.
fn fixture() -> (App, egui::Context, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temporary directory");
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().join("state")));
    app.preferences_writable = false;
    let mut state = terminator_core::State {
        projects: vec![terminator_core::Project {
            id: "a".into(),
            name: "a".into(),
            path: PathBuf::from("/a"),
            layout: serde_json::Value::Null,
        }],
        selected_project: Some("a".into()),
        ..Default::default()
    };
    state.settings.spell_check = false;
    app.apply_state(state);
    (app, ctx, dir)
}

/// One 800 × 500 frame at `time` seconds, drawn by `paint`.
fn frame(
    ctx: &egui::Context,
    (time, events): (f64, Vec<egui::Event>),
    paint: impl FnMut(&mut egui::Ui),
) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 500.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        },
        paint,
    );
    output.textures_delta.clear();
}

/// One frame of the native view showing `path`.
fn view(app: &mut App, ctx: &egui::Context, path: &Path, input: (f64, Vec<egui::Event>)) {
    frame(ctx, input, |ui| {
        app.native_editor_view(NativeEditorViewInput {
            ui,
            path,
            project: Some("a"),
            tab: None,
            node: None,
        });
    });
}

/// Pointer move, press and release at `at`, one frame each.
fn click_events(at: egui::Pos2) -> [Vec<egui::Event>; 3] {
    let button = |pressed| egui::Event::PointerButton {
        pos: at,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    [
        vec![egui::Event::PointerMoved(at)],
        vec![button(true)],
        vec![button(false)],
    ]
}

const MARKDOWN: &str = "# Title\n\nSome prose that runs along the first lines.\n";

/// A loaded Markdown buffer in Edit mode, also written to disk.
fn markdown_doc(dir: &Path) -> (PathBuf, NativeDoc) {
    let path = dir.join("notes.md");
    std::fs::write(&path, MARKDOWN).expect("write the file");
    let mut doc = NativeDoc::loaded(path.clone(), MARKDOWN);
    doc.markdown_mode = markdown::Mode::Edit;
    (path, doc)
}

#[test]
fn markdown_toolbar_floats_over_the_top_right_corner_without_a_path_bar() {
    let (mut app, ctx, dir) = fixture();
    let (path, doc) = markdown_doc(dir.path());
    app.native_docs.insert(path.clone(), doc);
    view(&mut app, &ctx, &path, (0.0, Vec::new()));
    // Answer the language-server trust question, so its bar goes away.
    if let Some(root) = app.lsp.trust_request_for(&path).map(Path::to_path_buf) {
        app.lsp.set_trust(&root, false);
    }
    // The first frames size the floating toolbar.
    for step in 1..4 {
        view(&mut app, &ctx, &path, (f64::from(step) * 0.1, Vec::new()));
    }
    let file = current_rect(&ctx, "native-file").expect("source view");
    let toolbar = current_rect(&ctx, "native-markdown-toolbar").expect("toolbar");
    assert!(file.top() < 4.0, "no path bar above the text: {file:?}");
    assert!(
        toolbar.top() >= file.top() && toolbar.top() - file.top() < 12.0,
        "toolbar {toolbar:?} sits on the text's top edge {file:?}"
    );
    assert!(
        toolbar.right() < 800.0 && 800.0 - toolbar.right() < 30.0,
        "toolbar {toolbar:?} sits at the right edge"
    );
    let split = current_rect(&ctx, "native-markdown-mode:Split").expect("Split");
    for label in ["Edit", "Preview", "Split"] {
        let mode = current_rect(&ctx, &format!("native-markdown-mode:{label}")).expect(label);
        assert!(
            toolbar.contains_rect(mode),
            "{label} {mode:?} in {toolbar:?}"
        );
        assert!(mode.right() <= split.right(), "the modes end the row");
    }
    // The floating buttons take clicks over the text under them.
    let mut time = 0.3;
    for events in click_events(split.center()) {
        time += 0.1;
        view(&mut app, &ctx, &path, (time, events));
    }
    let doc = app.native_docs.get(&path).expect("buffer");
    assert_eq!(doc.markdown_mode, markdown::Mode::Split);
    assert_eq!(doc.doc.as_ref().map(Doc::text).as_deref(), Some(MARKDOWN));
    let caret = doc.engine.cursor();
    assert_eq!(
        (caret.line, caret.col),
        (0, 0),
        "the text under it kept its caret"
    );
}

#[test]
fn markdown_toolbar_fades_while_typing_and_returns_after_a_pause() {
    let (mut app, ctx, dir) = fixture();
    let (path, doc) = markdown_doc(dir.path());
    app.native_docs.insert(path.clone(), doc);
    let shown = |app: &mut App, time: f64| {
        view(app, &ctx, &path, (time, Vec::new()));
        current_rect(&ctx, "native-markdown-toolbar").is_some()
    };
    for step in 0..3 {
        assert!(shown(&mut app, f64::from(step) * 0.1));
    }
    let typed = |app: &mut App| {
        let doc = app.native_docs.get_mut(&path).expect("buffer");
        doc.doc.as_mut().expect("loaded").insert(0, "x");
    };
    typed(&mut app);
    let mut time = 0.3;
    let edited = time;
    while shown(&mut app, time) {
        time += 0.1;
        assert!(
            time < edited + 1.0,
            "the toolbar fades while the user types"
        );
    }
    // More typing keeps it away; a pause brings it back.
    typed(&mut app);
    time += 0.1;
    let edited = time;
    while !shown(&mut app, time) {
        time += 0.1;
        assert!(time < edited + 3.0, "the toolbar returns after a pause");
    }
    assert!(
        time - edited >= toolbar::TYPING_PAUSE - 0.05,
        "{time} {edited}"
    );
}

#[test]
fn line_break_cell_opens_a_menu_that_converts_the_buffer() {
    let (mut app, ctx, dir) = fixture();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "one\ntwo\n").expect("write the file");
    app.native_docs
        .insert(path.clone(), NativeDoc::loaded(path.clone(), "one\ntwo\n"));
    let mut time = 0.0;
    let mut step = |app: &mut App, events: Vec<egui::Event>| {
        time += 0.1;
        view(app, &ctx, &path, (time, events));
    };
    step(&mut app, Vec::new());
    let cell = "native-status-cell:Line breaks (saved with the next save)";
    let at = current_rect(&ctx, cell).expect("line-break cell").center();
    for events in click_events(at) {
        step(&mut app, events);
    }
    // The opening frame sizes the menu; read the item after it settles.
    step(&mut app, Vec::new());
    let item = current_rect(&ctx, "CRLF line breaks").expect("menu item");
    for events in click_events(item.center()) {
        step(&mut app, events);
    }
    let doc = app.native_docs.get(&path).expect("buffer");
    assert_eq!(doc.line_ending, LineEnding::Crlf);
    assert!(doc.dirty(), "the next save writes the new line breaks");
}

#[test]
fn pane_tab_menu_toggles_the_bookmark_and_lists_project_recent_files() {
    let (mut app, ctx, _dir) = fixture();
    let path = PathBuf::from("/a/src/main.rs");
    let (lib, notes) = (PathBuf::from("/a/src/lib.rs"), PathBuf::from("/a/notes.md"));
    app.preferences.recent_native_files = vec![
        path.clone(),
        lib.clone(),
        PathBuf::from("/b/other.rs"),
        notes.clone(),
    ];
    app.preferences.pinned_native_files.insert(notes.clone());
    assert_eq!(app.native_recent_files(&path), [notes, lib]);
    let mut time = 0.0;
    let mut menu = |app: &mut App, events: Vec<egui::Event>| {
        time += 0.1;
        frame(&ctx, (time, events), |ui| app.native_file_menu(ui, &path));
    };
    menu(&mut app, Vec::new());
    assert!(current_rect(&ctx, "Recent files").is_some());
    assert!(
        current_rect(&ctx, "Local History").is_none(),
        "the daemon lacks Local History"
    );
    let bookmark = current_rect(&ctx, "Bookmark file").expect("bookmark item");
    for events in click_events(bookmark.center()) {
        menu(&mut app, events);
    }
    assert!(app.preferences.pinned_native_files.contains(&path));
    for events in click_events(bookmark.center()) {
        menu(&mut app, events);
    }
    assert!(!app.preferences.pinned_native_files.contains(&path));
}
