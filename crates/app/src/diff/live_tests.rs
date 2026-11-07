use super::live::{EditorMarks, LiveEditor, MarkColors, build_live, editor_marks};
use super::*;
use eframe::egui::Color32;
use terminator_native_edit::doc::Buffer;

const COLORS: MarkColors = MarkColors {
    added: Color32::GREEN,
    modified: Color32::BLUE,
    row: 25,
    word: 70,
};

fn marks(left: &str, right: &str) -> EditorMarks {
    let path = Path::new("file.rs");
    let base = build_with_context(BuildWithContextInput {
        path,
        left,
        right: left,
        staged: false,
        context: 3,
    });
    editor_marks(&build_live(&base, right, path), COLORS)
}

#[test]
fn deleted_lines_become_filler_rows_before_the_next_buffer_line() {
    let marks = marks("a\nb\nc\n", "a\nc\n");
    assert_eq!(*marks.spacers, [(1, 1)]);
    assert!(
        marks.line_fills.is_empty(),
        "a deletion tints no buffer line"
    );
}

#[test]
fn trailing_deletions_fill_after_the_last_buffer_line() {
    assert_eq!(*marks("a\nb\nc\n", "a\n").spacers, [(1, 2)]);
}

#[test]
fn a_longer_replaced_block_pads_the_buffer_side() {
    let marks = marks("a\nb\nc\nd\n", "a\nX\nd\n");
    // Left rows: a, b|X, c|filler, d — one filler before `d` (line 2).
    assert_eq!(*marks.spacers, [(2, 1)]);
    let fill = marks.line_fills.first().expect("replaced line tint");
    assert_eq!(fill.line, 1);
    assert_eq!(
        fill.color,
        Color32::from_rgba_unmultiplied(0, 0, 255, COLORS.row)
    );
}

#[test]
fn inserted_lines_tint_added_and_changed_words_get_ranges() {
    let added = marks("a\n", "a\nnew\n");
    assert!(added.spacers.is_empty());
    assert_eq!(added.line_fills.len(), 1);
    assert_eq!(
        added.line_fills[0].color,
        Color32::from_rgba_unmultiplied(0, 255, 0, COLORS.row)
    );
    let words = marks("let a = 1;\n", "let a = 2;\n");
    assert!(
        words
            .range_fills
            .iter()
            .any(|fill| fill.line == 0 && fill.start <= 8 && 8 < fill.end),
        "the changed digit is marked: {:?}",
        words.range_fills
    );
}

#[test]
fn identical_sides_pair_every_line_without_fillers() {
    let path = Path::new("file.rs");
    let base = build_with_context(BuildWithContextInput {
        path,
        left: "a\nb\n",
        right: "a\nb\n",
        staged: false,
        context: 3,
    });
    let live = build_live(&base, "a\nb\n", path);
    assert_eq!(live.split.len(), 2);
    assert!(editor_marks(&live, COLORS).spacers.is_empty());
}

#[test]
fn no_highlight_mode_keeps_fillers_but_drops_tints() {
    let path = Path::new("file.rs");
    let mut base = build_with_context(BuildWithContextInput {
        path,
        left: "a\nb\n",
        right: "a\nb\n",
        staged: false,
        context: 3,
    });
    base.options.highlight = Highlight::None;
    let marks = editor_marks(&build_live(&base, "x\n", path), COLORS);
    assert!(marks.line_fills.is_empty() && marks.range_fills.is_empty());
    assert_eq!(*marks.spacers, [(1, 1)]);
}

#[test]
fn editor_text_round_trips_line_endings() {
    for text in ["a\r\nb\r\n", "a\r\nb\n", "a\nb", ""] {
        assert_eq!(LiveEditor::new(text).text(), text);
    }
    let crlf = LiveEditor::new("a\r\nb\r\n");
    assert_eq!(crlf.doc.line_text(0), "a", "CRLF files edit as LF");
}

#[test]
fn editing_the_buffer_marks_unsaved_and_discard_restores() {
    let document = std::sync::Arc::new(build_with_context(BuildWithContextInput {
        path: Path::new("file.rs"),
        left: "a\n",
        right: "a\nb\n",
        staged: false,
        context: 3,
    }));
    let mut state = review::State {
        buffer: Some(review::Edit::new(std::sync::Arc::clone(&document))),
        ..Default::default()
    };
    assert!(!state.unsaved_edits(), "a fresh buffer is clean");
    let buffer = state.buffer.as_mut().expect("buffer");
    let at = buffer.editor.doc.char_at_line_col(1, 1);
    buffer.editor.doc.insert(at, "!");
    buffer.sync_text(1.0);
    assert_eq!(buffer.text, "a\nb!\n");
    assert!(state.unsaved_edits());
    // Discard drops the buffer; the next seed starts from the document.
    state.buffer = None;
    assert!(!state.unsaved_edits());
    assert_eq!(review::Edit::new(document).text, "a\nb\n");
}

#[test]
fn a_large_first_live_diff_builds_off_the_gui_thread() {
    let left = "line\n".repeat(8192);
    let right = format!("{left}tail\n");
    let document = std::sync::Arc::new(build_with_context(BuildWithContextInput {
        path: Path::new("file.rs"),
        left: &left,
        right: &right,
        staged: false,
        context: 3,
    }));
    let (pool, _owner) = pool();
    let mut buffer = review::Edit::new(document);
    let (_, waiting) = buffer.live_document(Path::new("file.rs"), 0.0, &pool);
    assert!(waiting, "the first large build waits for its worker");
    assert!(buffer.live_loading());
    let started = std::time::Instant::now();
    while buffer.live_loading() && started.elapsed() < std::time::Duration::from_secs(30) {
        std::thread::sleep(std::time::Duration::from_millis(10));
        let _ = buffer.live_document(Path::new("file.rs"), 0.0, &pool);
    }
    assert!(!buffer.live_loading(), "the worker build is shown");
}

#[test]
fn a_live_build_that_ends_without_a_result_is_built_again_after_a_pause() {
    let left = "line\n".repeat(8192);
    let right = format!("{left}tail\n");
    let document = std::sync::Arc::new(build_with_context(BuildWithContextInput {
        path: Path::new("file.rs"),
        left: &left,
        right: &right,
        staged: false,
        context: 3,
    }));
    let (pool, _owner) = pool();
    let mut buffer = review::Edit::new(document);
    let _ = buffer.live_document(Path::new("file.rs"), 0.0, &pool);
    assert!(buffer.live.building());
    // The pool job timed out or panicked: its sender is gone.
    buffer.live.drop_build_result();
    let (_, waiting) = buffer.live_document(Path::new("file.rs"), 0.5, &pool);
    assert!(
        waiting && !buffer.live.building(),
        "no spinner on a dead build"
    );
    let _ = buffer.live_document(Path::new("file.rs"), 1.0, &pool);
    assert!(!buffer.live.building(), "the retry waits for its pause");
    settle(&mut buffer, &pool, 2.0);
    assert!(!buffer.live_loading(), "the retried build is shown");
}

fn pool() -> (crate::gui_services::Submit, crate::gui_services::Owner) {
    let dir = std::env::temp_dir().join(terminator_core::id());
    let (updates, _) = std::sync::mpsc::channel();
    let (services, owner) = crate::gui_services::Services::new(
        terminator_core::Paths::at(dir),
        eframe::egui::Context::default(),
        updates,
    )
    .unwrap();
    (services.submit(), owner)
}

/// Edit the buffer, then poll the live diff (one frame per call) until
/// it describes the current revision.
fn settle(buffer: &mut review::Edit, pool: &crate::gui_services::Submit, now: f64) -> u64 {
    let started = std::time::Instant::now();
    loop {
        let _ = buffer.live_document(Path::new("file.rs"), now, pool);
        let revision = buffer.editor.doc.revision();
        if buffer
            .live
            .shown
            .as_ref()
            .is_some_and(|(at, _)| *at == revision)
            || started.elapsed() > std::time::Duration::from_secs(30)
        {
            return revision;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn typing_rebuilds_the_live_diff_off_the_gui_thread_and_drops_stale_builds() {
    let document = std::sync::Arc::new(build_with_context(BuildWithContextInput {
        path: Path::new("file.rs"),
        left: "a\n",
        right: "a\nb\n",
        staged: false,
        context: 3,
    }));
    let (pool, _owner) = pool();
    let mut buffer = review::Edit::new(document);
    // The first small build is shown at once.
    let _ = buffer.live_document(Path::new("file.rs"), 0.0, &pool);
    assert!(!buffer.live_loading());
    let first = buffer.live.shown.as_ref().map(|(at, _)| *at);
    // Two quick edits: the frame after an edit keeps the old document
    // while the rebuild runs on the pool.
    for (step, text) in ["x", "y"].into_iter().enumerate() {
        let at = buffer.editor.doc.char_at_line_col(1, 1);
        buffer.editor.doc.insert(at, text);
        buffer.sync_text(1.0 + step as f64);
        let (_, waiting) = buffer.live_document(Path::new("file.rs"), 1.0 + step as f64, &pool);
        assert!(waiting, "an edit re-diffs on the pool, not this frame");
        assert_eq!(buffer.live.shown.as_ref().map(|(at, _)| *at), first);
    }
    let revision = settle(&mut buffer, &pool, 10.0);
    let (at, live) = buffer.live.shown.as_ref().expect("live diff");
    assert_eq!(*at, revision, "only the newest revision is shown");
    assert_eq!(live.right_text, "a\nbyx\n");
}
