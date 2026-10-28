use super::*;
use crate::doc::Doc;
use crate::vim::{Key, ModalEngine};

fn modeless(text: &str) -> (VimEngine, Doc) {
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    (engine, Doc::new(text))
}

fn at(engine: &mut VimEngine, doc: &mut Doc, line: usize, col: usize) {
    engine.place_cursor(doc, line, col);
}

#[test]
fn shift_right_selects_end_exclusive_text() {
    let (mut engine, mut doc) = modeless("hello");
    engine.ide_move(&mut doc, IdeMotion::Right, true);
    engine.ide_move(&mut doc, IdeMotion::Right, true);
    assert_eq!(engine.copy_text(&doc), "he");
    // Shrinking back to the anchor leaves a bare caret.
    engine.ide_move(&mut doc, IdeMotion::Left, true);
    engine.ide_move(&mut doc, IdeMotion::Left, true);
    assert!(engine.selection().is_none());
}

#[test]
fn plain_arrows_collapse_a_selection_to_its_edge() {
    let (mut engine, mut doc) = modeless("hello");
    at(&mut engine, &mut doc, 0, 1);
    engine.ide_move(&mut doc, IdeMotion::Right, true);
    engine.ide_move(&mut doc, IdeMotion::Right, true);
    engine.ide_move(&mut doc, IdeMotion::Left, false);
    assert!(engine.selection().is_none());
    assert_eq!(engine.cursor(), Cursor { line: 0, col: 1 });
}

#[test]
fn caret_reaches_line_end_and_wraps_across_lines() {
    let (mut engine, mut doc) = modeless("ab\ncd");
    engine.ide_move(&mut doc, IdeMotion::LineEnd, false);
    assert_eq!(engine.cursor(), Cursor { line: 0, col: 2 });
    engine.ide_move(&mut doc, IdeMotion::Right, false);
    assert_eq!(engine.cursor(), Cursor { line: 1, col: 0 });
    engine.ide_move(&mut doc, IdeMotion::Left, false);
    assert_eq!(engine.cursor(), Cursor { line: 0, col: 2 });
    // Typing at the end appends instead of inserting before the last char.
    engine.modeless_key(&mut doc, Key::Char('!'));
    assert_eq!(doc.text(), "ab!\ncd");
}

#[test]
fn word_motions_stop_at_word_edges() {
    let (mut engine, mut doc) = modeless("foo bar.baz");
    engine.ide_move(&mut doc, IdeMotion::WordRight, false);
    assert_eq!(engine.cursor().col, 3);
    engine.ide_move(&mut doc, IdeMotion::WordRight, false);
    assert_eq!(engine.cursor().col, 7);
    engine.ide_move(&mut doc, IdeMotion::WordLeft, false);
    assert_eq!(engine.cursor().col, 4);
}

#[test]
fn smart_home_toggles_between_indent_and_column_zero() {
    let (mut engine, mut doc) = modeless("    x = 1");
    at(&mut engine, &mut doc, 0, 7);
    engine.ide_move(&mut doc, IdeMotion::LineStart, false);
    assert_eq!(engine.cursor().col, 4);
    engine.ide_move(&mut doc, IdeMotion::LineStart, false);
    assert_eq!(engine.cursor().col, 0);
}

#[test]
fn duplicate_line_is_one_undo_step() {
    let (mut engine, mut doc) = modeless("a\nb");
    engine.ide_duplicate(&mut doc);
    assert_eq!(doc.text(), "a\na\nb");
    assert_eq!(engine.cursor().line, 1);
    engine.undo(&mut doc);
    assert_eq!(doc.text(), "a\nb");
}

#[test]
fn duplicate_selection_selects_the_copy() {
    let (mut engine, mut doc) = modeless("ab");
    engine.ide_move(&mut doc, IdeMotion::Right, true);
    engine.ide_duplicate(&mut doc);
    assert_eq!(doc.text(), "aab");
    assert_eq!(engine.copy_text(&doc), "a");
    assert_eq!(engine.cursor().col, 2);
}

#[test]
fn move_lines_carries_the_caret() {
    let (mut engine, mut doc) = modeless("one\ntwo\nthree\n");
    at(&mut engine, &mut doc, 1, 1);
    engine.ide_move_lines(&mut doc, true);
    assert_eq!(doc.text(), "two\none\nthree\n");
    assert_eq!(engine.cursor(), Cursor { line: 0, col: 1 });
    engine.ide_move_lines(&mut doc, false);
    engine.ide_move_lines(&mut doc, false);
    assert_eq!(doc.text(), "one\nthree\ntwo\n");
    // The final newline is not a line to swap with.
    engine.ide_move_lines(&mut doc, false);
    assert_eq!(doc.text(), "one\nthree\ntwo\n");
}

#[test]
fn delete_lines_removes_the_newline_too() {
    let (mut engine, mut doc) = modeless("a\nb\nc");
    at(&mut engine, &mut doc, 1, 0);
    engine.ide_delete_lines(&mut doc);
    assert_eq!(doc.text(), "a\nc");
    at(&mut engine, &mut doc, 1, 0);
    engine.ide_delete_lines(&mut doc);
    assert_eq!(doc.text(), "a");
}

#[test]
fn toggle_comment_round_trips_at_common_indent() {
    let (mut engine, mut doc) = modeless("fn a() {\n    x();\n}\n");
    engine.ide_move(&mut doc, IdeMotion::Down, true);
    engine.ide_move(&mut doc, IdeMotion::Down, true);
    engine.ide_toggle_comment(&mut doc, "//");
    assert_eq!(doc.text(), "// fn a() {\n//     x();\n}\n");
    engine.ide_toggle_comment(&mut doc, "//");
    assert_eq!(doc.text(), "fn a() {\n    x();\n}\n");
}

#[test]
fn toggle_comment_without_selection_moves_down() {
    let (mut engine, mut doc) = modeless("a\nb\n");
    engine.ide_toggle_comment(&mut doc, "#");
    assert_eq!(doc.text(), "# a\nb\n");
    assert_eq!(engine.cursor().line, 1);
}

#[test]
fn enter_keeps_indent_and_splits_braces() {
    let (mut engine, mut doc) = modeless("    if x {}");
    at(&mut engine, &mut doc, 0, 10);
    engine.modeless_key(&mut doc, Key::Enter);
    assert_eq!(doc.text(), "    if x {\n        \n    }");
    assert_eq!(engine.cursor(), Cursor { line: 1, col: 8 });
    engine.modeless_key(&mut doc, Key::Char('y'));
    engine.modeless_key(&mut doc, Key::Enter);
    assert_eq!(doc.text(), "    if x {\n        y\n        \n    }");
}

#[test]
fn brackets_pair_and_type_over_their_closer() {
    let (mut engine, mut doc) = modeless("");
    engine.modeless_key(&mut doc, Key::Char('f'));
    engine.type_text(&mut doc, "(");
    assert_eq!(doc.text(), "f()");
    assert_eq!(engine.cursor().col, 2);
    engine.type_text(&mut doc, ")");
    assert_eq!(doc.text(), "f()");
    assert_eq!(engine.cursor().col, 3);
}

#[test]
fn opener_before_a_word_does_not_pair() {
    let (mut engine, mut doc) = modeless("x");
    engine.modeless_key(&mut doc, Key::Char('('));
    assert_eq!(doc.text(), "(x");
}

#[test]
fn backspace_removes_an_empty_pair_and_whole_indent_levels() {
    let (mut engine, mut doc) = modeless("");
    engine.modeless_key(&mut doc, Key::Char('['));
    engine.modeless_key(&mut doc, Key::Backspace);
    assert_eq!(doc.text(), "");
    let (mut engine, mut doc) = modeless("        x");
    at(&mut engine, &mut doc, 0, 8);
    engine.modeless_key(&mut doc, Key::Backspace);
    assert_eq!(doc.text(), "    x");
}

#[test]
fn typing_replaces_the_selection() {
    let (mut engine, mut doc) = modeless("hello world");
    engine.ide_move(&mut doc, IdeMotion::WordRight, true);
    engine.modeless_key(&mut doc, Key::Char('J'));
    assert_eq!(doc.text(), "J world");
    engine.ide_move(&mut doc, IdeMotion::LineEnd, true);
    engine.modeless_key(&mut doc, Key::Backspace);
    assert_eq!(doc.text(), "J");
}

#[test]
fn modeless_undo_works_without_escape() {
    let (mut engine, mut doc) = modeless("x");
    at(&mut engine, &mut doc, 0, 1);
    engine.modeless_key(&mut doc, Key::Char('y'));
    engine.type_text(&mut doc, "z");
    engine.undo(&mut doc);
    assert_eq!(doc.text(), "x");
}

#[test]
fn tab_inserts_spaces_to_the_next_stop() {
    let (mut engine, mut doc) = modeless("ab");
    at(&mut engine, &mut doc, 0, 1);
    engine.modeless_key(&mut doc, Key::Tab);
    assert_eq!(doc.text(), "a   b");
    let (mut engine, mut doc) = modeless("\tx\n");
    at(&mut engine, &mut doc, 1, 0);
    engine.modeless_key(&mut doc, Key::Tab);
    assert_eq!(doc.text(), "\tx\n\t");
}

#[test]
fn join_lines_glues_with_one_space() {
    let (mut engine, mut doc) = modeless("let x =\n    1;\nfoo(\n  a)");
    engine.ide_join_lines(&mut doc);
    assert_eq!(doc.text(), "let x = 1;\nfoo(\n  a)");
    at(&mut engine, &mut doc, 1, 0);
    engine.ide_join_lines(&mut doc);
    assert_eq!(doc.text(), "let x = 1;\nfoo(a)");
}

#[test]
fn delete_word_back_stops_at_the_word_start() {
    let (mut engine, mut doc) = modeless("let value");
    engine.ide_move(&mut doc, IdeMotion::LineEnd, false);
    engine.ide_delete_word_back(&mut doc);
    assert_eq!(doc.text(), "let ");
}

#[test]
fn matching_bracket_and_caret_word() {
    let (mut engine, mut doc) = modeless("f(a[0], {b})\n");
    at(&mut engine, &mut doc, 0, 1);
    assert_eq!(
        engine.matching_bracket(&doc),
        Some((Cursor { line: 0, col: 1 }, Cursor { line: 0, col: 11 }))
    );
    at(&mut engine, &mut doc, 0, 10);
    assert_eq!(
        engine.matching_bracket(&doc),
        Some((Cursor { line: 0, col: 8 }, Cursor { line: 0, col: 10 }))
    );
    at(&mut engine, &mut doc, 0, 2);
    assert_eq!(engine.word_at_caret(&doc).as_deref(), Some("a"));
}

#[test]
fn tab_indents_only_lines_the_selection_covers() {
    let (mut engine, mut doc) = modeless("a\nb\nc");
    engine.ide_move(&mut doc, IdeMotion::Down, true);
    engine.modeless_key(&mut doc, Key::Tab);
    assert_eq!(doc.text(), "    a\nb\nc");
}

#[test]
fn extend_selection_grows_word_contents_brackets_line_and_shrinks_back() {
    let (mut engine, mut doc) = modeless("    call(one, two_x);\nnext\n");
    at(&mut engine, &mut doc, 0, 15);
    let mut grown = Vec::new();
    for _ in 0..6 {
        engine.ide_extend_selection(&mut doc);
        grown.push(engine.copy_text(&doc));
    }
    // The call expression (callee included) comes before the statement.
    assert_eq!(
        grown,
        [
            "two_x",
            "one, two_x",
            "(one, two_x)",
            "call(one, two_x)",
            "call(one, two_x);",
            "    call(one, two_x);",
        ]
    );
    engine.ide_shrink_selection(&mut doc);
    assert_eq!(engine.copy_text(&doc), "call(one, two_x);");
    for _ in 0..4 {
        engine.ide_shrink_selection(&mut doc);
    }
    assert_eq!(engine.copy_text(&doc), "two_x");
}

#[test]
fn vim_mode_keeps_inclusive_selections() {
    let mut engine = VimEngine::new();
    let mut doc = Doc::new("hello");
    engine.ide_move(&mut doc, IdeMotion::Right, true);
    assert_eq!(engine.copy_text(&doc), "he");
}
