use super::*;
use crate::doc::Doc;
use crate::vim::Key;

fn modeless(doc: &mut Doc, line: usize, col: usize) -> VimEngine {
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    let _ = engine.place_cursor(doc, line, col);
    engine
}

fn type_char(engine: &mut VimEngine, doc: &mut Doc, c: char) {
    engine.for_each_caret(doc, |engine, doc| engine.modeless_key(doc, Key::Char(c)));
}

#[test]
fn column_select_makes_one_caret_per_line_clamped_to_each_line() {
    let mut doc = Doc::new("abcdef\nab\nabcdef\n");
    let mut engine = modeless(&mut doc, 0, 0);
    engine.column_select(&doc, Cursor { line: 0, col: 1 }, Cursor { line: 2, col: 4 });
    let mut spans: Vec<_> = engine
        .carets()
        .iter()
        .map(|caret| {
            (
                caret.anchor.line,
                caret.anchor.col,
                caret.cursor.col,
                caret.mode,
            )
        })
        .collect();
    spans.sort_unstable_by_key(|span| span.0);
    assert_eq!(
        spans,
        [
            (0, 1, 4, Mode::Visual),
            (1, 1, 2, Mode::Visual),
            (2, 1, 4, Mode::Visual)
        ]
    );
    assert_eq!(engine.cursor, Cursor { line: 2, col: 4 });
}

#[test]
fn typing_after_column_select_edits_every_line_in_one_undo() {
    let mut doc = Doc::new("abcd\nabcd\nabcd\n");
    let mut engine = modeless(&mut doc, 0, 0);
    engine.column_select(&doc, Cursor { line: 0, col: 1 }, Cursor { line: 2, col: 3 });
    type_char(&mut engine, &mut doc, 'X');
    assert_eq!(doc.text(), "aXd\naXd\naXd\n");
    doc.end_run();
    assert!(doc.undo());
    assert_eq!(doc.text(), "abcd\nabcd\nabcd\n");
}

#[test]
fn zero_width_column_select_places_plain_carets() {
    let mut doc = Doc::new("ab\nab\n");
    let mut engine = modeless(&mut doc, 0, 0);
    engine.column_select(&doc, Cursor { line: 1, col: 1 }, Cursor { line: 0, col: 1 });
    assert_eq!(engine.carets().len(), 2);
    assert!(
        engine
            .carets()
            .iter()
            .all(|caret| caret.mode == Mode::Normal)
    );
    type_char(&mut engine, &mut doc, '-');
    assert_eq!(doc.text(), "a-b\na-b\n");
}

#[test]
fn typing_at_every_caret_is_one_undo_step() {
    let mut doc = Doc::new("ab\nab\nab\n");
    let mut engine = modeless(&mut doc, 0, 1);
    engine.toggle_caret_at(&doc, 1, 1);
    engine.toggle_caret_at(&doc, 2, 1);
    assert_eq!(engine.carets().len(), 3);
    type_char(&mut engine, &mut doc, 'X');
    assert_eq!(doc.text(), "aXb\naXb\naXb\n");
    let mut at: Vec<(usize, usize)> = engine
        .carets()
        .iter()
        .map(|caret| (caret.cursor.line, caret.cursor.col))
        .collect();
    at.sort_unstable();
    assert_eq!(at, [(0, 2), (1, 2), (2, 2)]);
    doc.end_run();
    assert!(doc.undo());
    assert_eq!(doc.text(), "ab\nab\nab\n");
    // Alt+Shift+click on an existing caret removes it.
    engine.toggle_caret_at(&doc, 1, 2);
    assert_eq!(engine.carets().len(), 2);
}

#[test]
fn carets_on_one_line_shift_and_merge() {
    let mut doc = Doc::new("abc\n");
    let mut engine = modeless(&mut doc, 0, 1);
    engine.toggle_caret_at(&doc, 0, 2);
    engine.toggle_caret_at(&doc, 0, 3);
    // Each Backspace removes the char before every caret.
    engine.for_each_caret(&mut doc, |engine, doc| {
        engine.modeless_key(doc, Key::Backspace)
    });
    assert_eq!(doc.text(), "\n");
    // All three landed on column 0: one caret is left.
    assert_eq!(engine.carets().len(), 1);
}

#[test]
fn next_occurrence_selects_matches_and_typing_replaces_them_all() {
    let mut doc = Doc::new("foo bar foo\nfoo\n");
    let mut engine = modeless(&mut doc, 0, 1);
    // First press selects the word at the caret.
    assert!(engine.add_next_occurrence(&doc));
    assert_eq!(engine.carets().len(), 1);
    assert_eq!(engine.caret_texts(&doc), ["foo"]);
    assert!(engine.add_next_occurrence(&doc));
    assert_eq!(engine.carets().len(), 2);
    // Unselect drops the newest; Select All takes every match.
    engine.remove_last_occurrence();
    assert_eq!(engine.carets().len(), 1);
    engine.add_all_occurrences(&doc);
    assert_eq!(engine.carets().len(), 3);
    assert!(!engine.add_next_occurrence(&doc), "no fourth match");
    assert_eq!(engine.caret_texts(&doc), ["foo", "foo", "foo"]);
    type_char(&mut engine, &mut doc, 'x');
    assert_eq!(doc.text(), "x bar x\nx\n");
    assert_eq!(engine.carets().len(), 3);
    engine.clear_extra_carets();
    assert_eq!(engine.carets().len(), 1);
}
