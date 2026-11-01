use super::*;

#[test]
fn insert_and_delete_round_trip() {
    let mut doc = Doc::new("hello");
    doc.insert(5, " world");
    assert_eq!(doc.text().as_str(), "hello world");
    doc.delete(5..11);
    assert_eq!(doc.text().as_str(), "hello");
}

#[test]
fn unicode_indices_are_char_based() {
    let mut doc = Doc::new("日本語");
    doc.insert(3, "テスト");
    assert_eq!(doc.text().as_str(), "日本語テスト");
    assert_eq!(doc.line_col_at(4), (0, 4));
    assert_eq!(doc.char_at_line_col(0, 4), 4);
}

#[test]
fn lines_split_on_newlines() {
    let doc = Doc::new("a\nbb\n");
    assert_eq!(doc.line_count(), 3);
    assert_eq!(doc.line_text(1), "bb");
    assert_eq!(doc.char_at_line_col(1, 1), 3);
    assert_eq!(doc.line_col_at(3), (1, 1));
}

#[test]
fn typing_run_coalesces_into_one_undo() {
    let mut doc = Doc::new(String::new());
    doc.insert(0, "a");
    doc.insert(1, "b");
    doc.insert(2, "c");
    assert!(doc.undo());
    assert_eq!(doc.text().as_str(), "");
}

#[test]
fn undo_group_is_one_step_and_redoes_forward() {
    let mut doc = Doc::new(String::new());
    doc.begin_undo_group();
    doc.insert(0, "a");
    doc.insert(1, "b");
    doc.delete(0..1);
    doc.end_undo_group();
    assert_eq!(doc.text().as_str(), "b");
    assert!(doc.undo());
    assert_eq!(doc.text().as_str(), "");
    assert!(doc.redo());
    assert_eq!(doc.text().as_str(), "b");
}

#[test]
fn empty_group_records_nothing() {
    let mut doc = Doc::new("x");
    doc.begin_undo_group();
    doc.end_undo_group();
    assert!(!doc.undo());
    assert_eq!(doc.text().as_str(), "x");
}

#[test]
fn cursor_move_ends_the_run() {
    let mut doc = Doc::new(String::new());
    doc.insert(0, "a");
    doc.end_run();
    doc.insert(1, "b");
    assert!(doc.undo());
    assert_eq!(doc.text().as_str(), "a");
    assert!(doc.undo());
    assert_eq!(doc.text().as_str(), "");
}

#[test]
fn undo_redo_delete_restores_text() {
    let mut doc = Doc::new("hello");
    doc.delete(1..4);
    assert_eq!(doc.text().as_str(), "ho");
    assert!(doc.undo());
    assert_eq!(doc.text().as_str(), "hello");
    assert!(doc.redo());
    assert_eq!(doc.text().as_str(), "ho");
}

#[test]
fn reloaded_document_never_reuses_a_revision() {
    // A clean reload replaces the buffer; caches keyed on revision
    // must see the new text even when neither buffer was edited.
    let before = Doc::new("old");
    let reloaded = Doc::new("new");
    assert_ne!(before.revision(), reloaded.revision());
}

#[test]
fn revision_bumps_on_every_mutation() {
    let mut doc = Doc::new("hello");
    let initial = doc.revision();
    doc.insert(5, "!");
    let after_insert = doc.revision();
    assert!(after_insert > initial);
    doc.delete(0..1);
    assert!(doc.revision() > after_insert);
    let before_undo = doc.revision();
    assert!(doc.undo());
    assert!(doc.revision() > before_undo);
}

#[test]
fn trailing_newline_keeps_empty_last_line() {
    // Rope line tables must agree with the old backend: `a\nbb\n` is
    // three lines, and out-of-range rows read empty.
    let doc = Doc::new("a\nbb\n");
    assert_eq!(doc.line_count(), 3);
    assert_eq!(doc.line_text(2).as_str(), "");
    assert_eq!(doc.line_text(9).as_str(), "");
    assert_eq!(doc.char_at_line_col(9, 9), doc.len_chars());
}

#[test]
fn out_of_range_edits_clamp() {
    let mut doc = Doc::new("hi");
    doc.insert(99, "!");
    assert_eq!(doc.text().as_str(), "hi!");
    doc.delete(5..50);
    assert_eq!(doc.text().as_str(), "hi!");
    assert!(!Doc::new("x").undo());
}

#[test]
fn undo_inside_open_group_keeps_earlier_history() {
    // `ihello<Esc>`, `oabc`, undo, Esc: the next undo still removes hello.
    let mut doc = Doc::new(String::new());
    doc.begin_undo_group();
    doc.insert(0, "hello");
    doc.end_undo_group();
    doc.begin_undo_group();
    doc.insert(5, "\nabc");
    assert!(doc.undo());
    assert_eq!(doc.text().as_str(), "hello");
    doc.end_undo_group();
    assert!(doc.undo());
    assert_eq!(doc.text().as_str(), "");
    assert!(doc.redo());
    assert_eq!(doc.text().as_str(), "hello");
}

#[test]
fn nested_group_end_keeps_outer_group_open() {
    let mut doc = Doc::new(String::new());
    doc.begin_undo_group();
    doc.insert(0, "a");
    doc.begin_undo_group();
    doc.insert(1, "}");
    doc.end_undo_group();
    doc.insert(2, "b");
    doc.end_undo_group();
    assert!(doc.undo());
    assert_eq!(doc.text().as_str(), "");
}

#[test]
fn only_lsp_line_breaks_split_lines() {
    // Form feed and U+2028 are text, not line breaks, as in LSP.
    let doc = Doc::new("a\u{c}b\u{2028}c\u{85}d\ne\r\nf\rg");
    assert_eq!(doc.line_count(), 4);
    assert_eq!(doc.line_text(0), "a\u{c}b\u{2028}c\u{85}d");
}
