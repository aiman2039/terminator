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
