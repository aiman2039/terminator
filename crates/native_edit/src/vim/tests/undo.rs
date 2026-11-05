//! Undo and redo put the caret on the change and scroll to it.
use super::super::super::doc::{Buffer, Doc};
use super::super::IdeMotion;
use super::super::mode::{Cursor, Key, ModalEngine, Mode};
use super::super::state::VimEngine;
use super::super::vim_tests::tests::keys;

fn modeless(text: &str) -> (VimEngine, Doc) {
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    (engine, Doc::new(text))
}

/// One modeless key the way the view sends it: through the caret loop,
/// which notes the caret and selection for undo.
fn ide_key(engine: &mut VimEngine, doc: &mut Doc, key: Key) {
    engine.for_each_caret(doc, |engine, doc| engine.modeless_key(doc, key));
}

#[test]
fn vim_undo_puts_the_cursor_at_the_start_of_the_change() {
    let (mut eng, mut doc) = (VimEngine::new(), Doc::new("a\nb\nc\nd\n"));
    keys(&mut eng, &mut doc, "jjddgg");
    assert_eq!(doc.text(), "a\nb\nd\n");
    let _ = eng.take_scroll_request();
    keys(&mut eng, &mut doc, "u");
    assert_eq!(doc.text(), "a\nb\nc\nd\n");
    assert_eq!(eng.cursor(), Cursor { line: 2, col: 0 });
    assert!(eng.take_scroll_request(), "the view scrolls to the change");
    // Appended text: the start is past the line end, so Normal clamps.
    let (mut eng, mut doc) = (VimEngine::new(), Doc::new("ab\n"));
    keys(&mut eng, &mut doc, "A");
    eng.type_text(&mut doc, "cd");
    eng.press_key(&mut doc, Key::Escape);
    keys(&mut eng, &mut doc, "0u");
    assert_eq!(doc.text(), "ab\n");
    assert_eq!(eng.cursor(), Cursor { line: 0, col: 1 });
}

#[test]
fn vim_undo_and_redo_of_a_change_land_on_its_start() {
    let (mut eng, mut doc) = (VimEngine::new(), Doc::new("foo bar baz\nnext\n"));
    keys(&mut eng, &mut doc, "wcw");
    eng.type_text(&mut doc, "XY");
    eng.press_key(&mut doc, Key::Escape);
    keys(&mut eng, &mut doc, "j$u");
    assert_eq!(doc.text(), "foo bar baz\nnext\n");
    assert_eq!(eng.cursor(), Cursor { line: 0, col: 4 });
    assert_eq!(eng.mode(), Mode::Normal);
    keys(&mut eng, &mut doc, "j");
    eng.press_key(&mut doc, Key::Ctrl('r'));
    assert_eq!(doc.text(), "foo XY baz\nnext\n");
    assert_eq!(eng.cursor(), Cursor { line: 0, col: 4 });
}

#[test]
fn ide_undo_in_vim_visual_drops_the_selection_at_the_change() {
    let (mut eng, mut doc) = (VimEngine::new(), Doc::new("one\ntwo\n"));
    keys(&mut eng, &mut doc, "x");
    keys(&mut eng, &mut doc, "jvl");
    eng.undo(&mut doc);
    assert_eq!(doc.text(), "one\ntwo\n");
    assert_eq!(eng.mode(), Mode::Normal);
    assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
}

#[test]
fn undo_inside_an_insert_session_stays_in_insert_at_the_change() {
    let (mut eng, mut doc) = (VimEngine::new(), Doc::new("x\n"));
    keys(&mut eng, &mut doc, "A");
    eng.type_text(&mut doc, "abc");
    eng.undo(&mut doc);
    assert_eq!(doc.text(), "x\n");
    assert_eq!(eng.mode(), Mode::Insert);
    assert_eq!(eng.cursor(), Cursor { line: 0, col: 1 });
}

#[test]
fn backspace_after_undo_in_replace_mode_puts_nothing_back() {
    let (mut eng, mut doc) = (VimEngine::new(), Doc::new("abcd\n"));
    keys(&mut eng, &mut doc, "llR");
    eng.type_text(&mut doc, "XY");
    assert_eq!(doc.text(), "abXY\n");
    eng.undo(&mut doc);
    assert_eq!(doc.text(), "abcd\n");
    assert_eq!(eng.cursor(), Cursor { line: 0, col: 2 });
    eng.press_key(&mut doc, Key::Backspace);
    assert_eq!(doc.text(), "abcd\n", "no overwritten char comes back");
}

#[test]
fn modeless_undo_selects_the_deleted_text_again() {
    let (mut engine, mut doc) = modeless("hello world\n");
    let _ = engine.place_cursor(&mut doc, 0, 6);
    engine.ide_move(&mut doc, IdeMotion::LineEnd, true);
    ide_key(&mut engine, &mut doc, Key::Backspace);
    assert_eq!(doc.text(), "hello \n");
    engine.ide_move(&mut doc, IdeMotion::DocStart, false);
    engine.undo(&mut doc);
    assert_eq!(doc.text(), "hello world\n");
    assert_eq!(engine.mode(), Mode::Visual);
    assert_eq!(engine.copy_text(&doc), "world");
    assert_eq!(engine.cursor(), Cursor { line: 0, col: 11 });
    // Cut notes its selection too.
    assert_eq!(engine.cut(&mut doc), "world");
    engine.ide_move(&mut doc, IdeMotion::DocStart, false);
    engine.undo(&mut doc);
    assert_eq!(engine.copy_text(&doc), "world");
}

#[test]
fn modeless_undo_puts_the_caret_back_where_the_edit_began() {
    let (mut engine, mut doc) = modeless("one\ntwo\n");
    let _ = engine.place_cursor(&mut doc, 1, 3);
    for _ in 0..3 {
        ide_key(&mut engine, &mut doc, Key::Backspace);
    }
    assert_eq!(doc.text(), "one\n\n");
    engine.ide_move(&mut doc, IdeMotion::DocStart, false);
    engine.undo(&mut doc);
    assert_eq!(doc.text(), "one\ntwo\n");
    assert_eq!(engine.cursor(), Cursor { line: 1, col: 3 });
    assert!(engine.selection().is_none());
    // Redo has no note to restore: the caret goes to the change start.
    engine.redo(&mut doc);
    assert_eq!(doc.text(), "one\n\n");
    assert_eq!(engine.cursor(), Cursor { line: 1, col: 0 });
}

#[test]
fn modeless_undo_without_a_note_lands_on_the_change_start() {
    // A direct engine edit (no view caret loop) records no note.
    let (mut engine, mut doc) = modeless("ab\ncd\n");
    let _ = engine.place_cursor(&mut doc, 1, 1);
    engine.paste_text(&mut doc, "XYZ");
    engine.ide_move(&mut doc, IdeMotion::DocStart, false);
    engine.undo(&mut doc);
    assert_eq!(doc.text(), "ab\ncd\n");
    assert_eq!(engine.cursor(), Cursor { line: 1, col: 1 });
}

#[test]
fn undo_with_many_carets_leaves_one_caret_at_the_first_change() {
    let (mut engine, mut doc) = modeless("ab\nab\nab\n");
    let _ = engine.place_cursor(&mut doc, 2, 1);
    engine.toggle_caret_at(&doc, 0, 1);
    engine.toggle_caret_at(&doc, 1, 1);
    ide_key(&mut engine, &mut doc, Key::Char('X'));
    assert_eq!(doc.text(), "aXb\naXb\naXb\n");
    engine.ide_move(&mut doc, IdeMotion::DocEnd, false);
    engine.undo(&mut doc);
    assert_eq!(doc.text(), "ab\nab\nab\n");
    assert_eq!(engine.carets().len(), 1);
    assert_eq!(engine.cursor(), Cursor { line: 0, col: 1 });
}

#[test]
fn undo_scrolls_only_when_the_change_is_off_screen() {
    let (mut eng, mut doc) = (VimEngine::new(), Doc::new("a\nb\nc\nd\ne\n"));
    keys(&mut eng, &mut doc, "x");
    eng.set_viewport(0, 1);
    let _ = eng.take_scroll_request();
    eng.undo(&mut doc);
    assert!(!eng.take_scroll_request(), "line 0 is on screen");
    keys(&mut eng, &mut doc, "Gx");
    keys(&mut eng, &mut doc, "gg");
    let _ = eng.take_scroll_request();
    eng.undo(&mut doc);
    assert_eq!(eng.cursor(), Cursor { line: 4, col: 0 });
    assert!(eng.take_scroll_request(), "line 4 is below the screen");
    // Nothing left to undo: no move, no scroll.
    eng.undo(&mut doc);
    eng.undo(&mut doc);
    assert!(!eng.take_scroll_request());
    assert!(!doc.undo());
}
