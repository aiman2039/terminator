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

#[test]
fn char_at_line_col_clamps_to_line_end_without_the_newline() {
    let doc = Doc::new("ab\ncdé\nlast");
    assert_eq!(doc.char_at_line_col(0, 99), 2);
    assert_eq!(doc.char_at_line_col(1, 99), 6);
    assert_eq!(doc.char_at_line_col(2, 99), doc.len_chars());
    let crlf = Doc::new("ab\r\ncd");
    // `\r\n` is one line break: the caret stops before the `\r`.
    assert_eq!(crlf.char_at_line_col(0, 99), 2);
}

#[test]
fn typing_at_the_end_of_a_crlf_line_keeps_one_line_break() {
    let mut doc = Doc::new("one\r\ntwo\nthree\rfour");
    assert_eq!(doc.line_text(0), "one");
    assert_eq!(doc.line_text(2), "three");
    let end = doc.char_at_line_col(0, usize::MAX);
    doc.insert(end, "!");
    assert_eq!(doc.text(), "one!\r\ntwo\nthree\rfour");
    let end = doc.char_at_line_col(2, usize::MAX);
    doc.insert(end, "?");
    assert_eq!(doc.text(), "one!\r\ntwo\nthree?\rfour");
}

#[test]
fn undo_and_redo_report_where_the_text_changed() {
    let mut doc = Doc::new("hello");
    doc.insert(5, " world");
    let undone = doc.undo_change().expect("one step");
    assert_eq!(undone.range, 5..5, "the removed text leaves an empty spot");
    let redone = doc.redo_change().expect("one step");
    assert_eq!(redone.range, 5..11, "the text put back");
    doc.delete(0..2);
    assert_eq!(doc.undo_change().expect("delete").range, 0..2);
    assert_eq!(doc.text(), "hello world");
    assert_eq!(doc.undo_change().expect("insert").range, 5..5);
    assert_eq!(doc.undo_change(), None);
    assert_eq!(Doc::new("x").redo_change(), None);
}

#[test]
fn group_undo_reports_one_span_over_every_edit() {
    let mut doc = Doc::new("mid");
    doc.begin_undo_group();
    doc.insert(0, "A");
    doc.insert(4, "Z");
    doc.end_undo_group();
    assert_eq!(doc.text(), "AmidZ");
    assert_eq!(doc.undo_change().expect("group").range, 0..3);
    assert_eq!(doc.text(), "mid");
    assert_eq!(doc.redo_change().expect("group").range, 0..5);
    // A change that replaced text spans the text put back.
    let mut doc = Doc::new("foo bar");
    doc.begin_undo_group();
    doc.delete(4..7);
    doc.insert(4, "X");
    doc.insert(5, "Y");
    doc.end_undo_group();
    assert_eq!(doc.undo_change().expect("change").range, 4..7);
    assert_eq!(doc.text(), "foo bar");
}

#[test]
fn undo_returns_the_selection_noted_before_the_edit() {
    let mut doc = Doc::new("hello world");
    doc.note_selection(Some((6, 11)));
    doc.delete(6..11);
    let undone = doc.undo_change().expect("delete");
    assert_eq!(undone.selection, Some((6, 11)));
    // Redo leaves the text the note belongs to; a second undo has it.
    assert_eq!(doc.redo_change().expect("redo").selection, None);
    assert_eq!(doc.undo_change().expect("again").selection, Some((6, 11)));
}

#[test]
fn only_the_next_new_step_keeps_a_note() {
    let mut doc = Doc::new("abc");
    doc.note_selection(Some((1, 3)));
    doc.delete(1..3);
    doc.end_run();
    doc.insert(1, "x");
    assert_eq!(doc.undo_change().expect("insert").selection, None);
    assert_eq!(doc.undo_change().expect("delete").selection, Some((1, 3)));
    // A typing run keeps the note from its start; later notes merge away.
    let mut doc = Doc::new("");
    doc.note_selection(Some((0, 0)));
    doc.insert(0, "a");
    doc.note_selection(Some((1, 1)));
    doc.insert(1, "b");
    let run = doc.undo_change().expect("run");
    assert_eq!((run.range, run.selection), (0..0, Some((0, 0))));
    // A unit keeps the note from before it opened.
    let mut doc = Doc::new("ab");
    doc.note_selection(Some((2, 0)));
    doc.begin_undo_group();
    doc.delete(0..2);
    doc.note_selection(Some((0, 0)));
    doc.insert(0, "c");
    doc.end_undo_group();
    assert_eq!(doc.undo_change().expect("unit").selection, Some((2, 0)));
    // A cleared note is gone.
    doc.note_selection(Some((0, 1)));
    doc.note_selection(None);
    doc.insert(0, "z");
    assert_eq!(doc.undo_change().expect("edit").selection, None);
}

#[test]
fn content_hash_streams_chunks_like_hashing_the_text() {
    let text = "x".repeat(10_000) + "\nyé\r\n";
    let doc = Doc::new(text.clone());
    assert_eq!(doc.content_hash(), super::hash_text(&text));
    assert_ne!(doc.content_hash(), super::hash_text("other"));
}

#[test]
fn the_journal_lists_the_edits_after_a_revision_until_it_forgets_them() {
    let mut doc = Doc::new("abc");
    let start = doc.revision();
    doc.insert(1, "XY");
    doc.delete(0..1);
    assert_eq!(
        doc.changes_since(start),
        Some(vec![Change::insert(1, 2), Change::delete(0, 1)])
    );
    let now = doc.revision();
    assert_eq!(doc.changes_since(now), Some(Vec::new()));
    // Undo and redo are edits too.
    doc.undo();
    assert_eq!(doc.changes_since(now), Some(vec![Change::insert(0, 1)]));
    let undone = doc.revision();
    doc.redo();
    assert_eq!(doc.changes_since(undone), Some(vec![Change::delete(0, 1)]));
    // Another buffer's revision, or one too old, is not known.
    assert_eq!(doc.changes_since(Doc::new("x").revision()), None);
    for _ in 0..1100 {
        doc.insert(0, "x");
    }
    assert_eq!(doc.changes_since(now), None);
}
