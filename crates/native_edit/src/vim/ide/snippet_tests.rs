//! Snippet sessions: stop order, copies, ranges that follow typing, and
//! the ways a session ends.
use super::*;
use crate::doc::Doc;
use crate::lsp::{SnippetVars, parse_snippet};
use crate::vim::{Key, ModalEngine};

fn modeless(text: &str) -> (VimEngine, Doc) {
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    (engine, Doc::new(text))
}

/// A vim engine in Insert mode.
fn vim_insert(text: &str) -> (VimEngine, Doc) {
    let mut engine = VimEngine::new();
    let mut doc = Doc::new(text);
    engine.press_key(&mut doc, Key::Char('i'));
    (engine, doc)
}

/// Insert `snippet` at char `at` and start its session, as an accepted
/// completion does.
fn insert(engine: &mut VimEngine, doc: &mut Doc, at: usize, snippet: &str) {
    let snippet = parse_snippet(snippet, &SnippetVars::default());
    doc.insert(at, &snippet.text);
    engine.start_snippet(doc, at, &snippet);
}

/// Type `text` at every caret, as the modeless view does.
fn type_modeless(engine: &mut VimEngine, doc: &mut Doc, text: &str) {
    for c in text.chars() {
        engine.for_each_caret(doc, |engine, doc| engine.modeless_key(doc, Key::Char(c)));
    }
}

fn caret(engine: &VimEngine) -> (usize, usize) {
    (engine.cursor().line, engine.cursor().col)
}

#[test]
fn the_first_stop_is_selected_and_tab_walks_the_stops_in_index_order() {
    let (mut engine, mut doc) = modeless("let v = ;\n");
    insert(&mut engine, &mut doc, 8, "foo(${2:b}, ${1:a})$0");
    assert_eq!(doc.text(), "let v = foo(b, a);\n");
    assert!(engine.snippet_active());
    assert_eq!(engine.copy_text(&doc), "a", "`$1` comes first");
    assert!(engine.snippet_jump(&mut doc, true));
    assert_eq!(engine.copy_text(&doc), "b");
    assert!(engine.snippet_jump(&mut doc, false), "Shift+Tab goes back");
    assert_eq!(engine.copy_text(&doc), "a");
    assert!(
        engine.snippet_jump(&mut doc, false),
        "before the first stop"
    );
    assert_eq!(engine.copy_text(&doc), "a", "nothing moves");
    engine.snippet_jump(&mut doc, true);
    engine.snippet_jump(&mut doc, true);
    assert_eq!(caret(&engine), (0, 17), "`$0` is last");
    assert!(engine.selection().is_none());
    assert!(!engine.snippet_active());
    assert!(!engine.snippet_jump(&mut doc, true), "Tab is Tab again");
}

#[test]
fn typing_in_a_stop_shifts_the_stops_after_it() {
    let (mut engine, mut doc) = modeless("let v = ;\n");
    insert(&mut engine, &mut doc, 8, "foo(${1:a}, ${2:b})$0");
    type_modeless(&mut engine, &mut doc, "xyz");
    assert_eq!(doc.text(), "let v = foo(xyz, b);\n");
    // Backspace inside the stop keeps it, shorter.
    engine.modeless_key(&mut doc, Key::Backspace);
    type_modeless(&mut engine, &mut doc, "zz");
    assert_eq!(doc.text(), "let v = foo(xyzz, b);\n");
    engine.snippet_upkeep(&doc);
    assert!(engine.snippet_jump(&mut doc, true));
    assert_eq!(engine.copy_text(&doc), "b");
    type_modeless(&mut engine, &mut doc, "q(");
    assert_eq!(doc.text(), "let v = foo(xyzz, q());\n");
    engine.snippet_jump(&mut doc, true);
    assert_eq!(caret(&engine), (0, 22), "after the `)` of the snippet");
}

#[test]
fn edits_elsewhere_move_the_stops_with_their_text() {
    let (mut engine, mut doc) = modeless("fn main() {\n    \n}\n");
    insert(&mut engine, &mut doc, 16, "foo(${1:a}, ${2:b})");
    // A late auto-import lands above, and the carets follow it.
    let carets = engine.caret_offsets(&doc);
    doc.insert(0, "use x::foo;\n");
    engine.follow_carets(&doc, carets);
    assert_eq!(engine.copy_text(&doc), "a", "the stop stays selected");
    assert_eq!(caret(&engine), (2, 9));
    engine.snippet_jump(&mut doc, true);
    assert_eq!(engine.copy_text(&doc), "b");
    assert_eq!(caret(&engine), (2, 12));
    // Without `$0` the snippet end is last.
    engine.snippet_jump(&mut doc, true);
    assert_eq!(caret(&engine), (2, 13));
}

#[test]
fn copies_of_a_stop_get_a_caret_each_and_change_together() {
    let (mut engine, mut doc) = modeless("\n");
    insert(&mut engine, &mut doc, 0, "let ${1:x} = 1; $1 += ${2:2};");
    assert_eq!(engine.extra_carets().len(), 1);
    type_modeless(&mut engine, &mut doc, "count");
    assert_eq!(doc.text(), "let count = 1; count += 2;\n");
    engine.snippet_jump(&mut doc, true);
    assert!(engine.extra_carets().is_empty(), "`$2` has one copy");
    assert_eq!(engine.copy_text(&doc), "2");
    engine.snippet_jump(&mut doc, false);
    assert_eq!(engine.extra_carets().len(), 1, "back on both copies");
    engine.end_snippet();
    assert!(engine.extra_carets().is_empty(), "Escape drops the copies");
}

#[test]
fn vim_insert_mode_types_and_deletes_at_every_copy() {
    let (mut engine, mut doc) = vim_insert("\n");
    insert(&mut engine, &mut doc, 0, "${1:x} = $1;");
    assert_eq!(engine.extra_carets().len(), 1);
    assert!(engine.snippet_take_selection(&mut doc));
    assert_eq!(doc.text(), " = ;\n", "both copies give way");
    engine.for_each_caret(&mut doc, |engine, doc| engine.type_text(doc, "nm"));
    engine.for_each_caret(&mut doc, |engine, doc| {
        engine.press_key(doc, Key::Backspace)
    });
    assert_eq!(doc.text(), "n = n;\n");
    engine.snippet_jump(&mut doc, true);
    assert_eq!((caret(&engine), engine.extra_carets().len()), ((0, 6), 0));
}

#[test]
fn a_stop_nested_in_a_replaced_placeholder_is_skipped() {
    let (mut engine, mut doc) = modeless("\n");
    insert(&mut engine, &mut doc, 0, "${1:Vec<${2:T}>}::new()$0");
    assert_eq!(engine.copy_text(&doc), "Vec<T>");
    type_modeless(&mut engine, &mut doc, "S");
    engine.snippet_jump(&mut doc, true);
    assert_eq!(doc.text(), "S::new()\n");
    assert_eq!(caret(&engine), (0, 8), "straight to `$0`");
    assert!(!engine.snippet_active());
}

#[test]
fn vim_insert_mode_keeps_the_caret_after_the_stop_until_typing_replaces_it() {
    let (mut engine, mut doc) = vim_insert("x = ;\n");
    let _ = engine.place_cursor(&mut doc, 0, 4);
    insert(&mut engine, &mut doc, 4, "max(${1:a}, ${2:b})$0");
    assert_eq!(engine.mode(), Mode::Insert);
    assert_eq!(caret(&engine), (0, 9), "after `a`");
    engine.snippet_upkeep(&doc);
    let marks = engine.snippet_marks(&doc);
    assert!(marks.iter().any(|mark| mark.active && mark.selected));
    assert!(engine.snippet_take_selection(&mut doc));
    engine.type_text(&mut doc, "1");
    assert_eq!(doc.text(), "x = max(1, b);\n");
    assert!(engine.snippet_jump(&mut doc, true));
    assert_eq!(caret(&engine), (0, 12), "after `b`");
    // Backspace removes the whole stop; typing after it does not.
    assert!(engine.snippet_take_selection(&mut doc));
    assert_eq!(doc.text(), "x = max(1, );\n");
    assert!(!engine.snippet_take_selection(&mut doc));
    engine.type_text(&mut doc, "2");
    engine.snippet_jump(&mut doc, true);
    assert_eq!(
        (doc.text().as_str(), caret(&engine)),
        ("x = max(1, 2);\n", (0, 13))
    );
    assert_eq!(engine.mode(), Mode::Insert);
}

#[test]
fn a_late_import_keeps_the_vim_stop_ready_to_be_typed_over() {
    let (mut engine, mut doc) = vim_insert("fn main() {\n    \n}\n");
    let _ = engine.place_cursor(&mut doc, 1, 4);
    insert(&mut engine, &mut doc, 16, "read(${1:path})");
    // The auto-import of the accepted item lands above, after the insert.
    let carets = engine.caret_offsets(&doc);
    doc.insert(0, "use x::read;\n");
    engine.follow_carets(&doc, carets);
    assert!(engine.snippet_take_selection(&mut doc), "still selected");
    engine.type_text(&mut doc, "\"a\"");
    assert_eq!(
        doc.text(),
        "use x::read;\nfn main() {\n    read(\"a\")\n}\n"
    );
}

#[test]
fn moving_the_caret_cancels_the_vim_selection_of_a_stop() {
    let (mut engine, mut doc) = vim_insert("\n");
    insert(&mut engine, &mut doc, 0, "f(${1:arg})");
    engine.press_key(&mut doc, Key::Left);
    engine.snippet_upkeep(&doc);
    assert!(engine.snippet_marks(&doc).iter().all(|mark| !mark.selected));
    assert!(!engine.snippet_take_selection(&mut doc));
    engine.type_text(&mut doc, "_");
    assert_eq!(doc.text(), "f(ar_g)\n");
}

#[test]
fn escape_enter_undo_and_leaving_the_snippet_end_the_session() {
    let started = |snippet: &str| {
        let (mut engine, mut doc) = modeless("x\n");
        insert(&mut engine, &mut doc, 0, snippet);
        (engine, doc)
    };
    // Enter finishes at `$0`.
    let (mut engine, mut doc) = started("if ${1:c} {\n    $0\n}");
    assert!(engine.finish_snippet(&mut doc));
    assert_eq!((caret(&engine), engine.snippet_active()), ((1, 4), false));
    // A click outside the snippet.
    let (mut engine, mut doc) = started("f(${1:a})");
    let _ = engine.place_cursor(&mut doc, 0, 6);
    engine.snippet_upkeep(&doc);
    assert!(!engine.snippet_active());
    // Undo.
    let (mut engine, mut doc) = started("f(${1:a})");
    engine.undo(&mut doc);
    assert!(!engine.snippet_active());
    // Deleting the whole snippet.
    let (mut engine, mut doc) = started("f(${1:a})");
    engine.select_all(&doc);
    engine.ide_delete_selection(&mut doc);
    engine.snippet_upkeep(&doc);
    assert!(!engine.snippet_active());
    // vim: leaving Insert mode.
    let (mut engine, mut doc) = vim_insert("\n");
    insert(&mut engine, &mut doc, 0, "f(${1:a}, $2)");
    engine.press_key(&mut doc, Key::Escape);
    engine.snippet_upkeep(&doc);
    assert!(!engine.snippet_active());
}

#[test]
fn snippets_without_numbered_stops_start_no_session() {
    let (mut engine, mut doc) = modeless("\n");
    insert(&mut engine, &mut doc, 0, "foo($0)");
    assert!(!engine.snippet_active());
    assert_eq!(caret(&engine), (0, 4));
    // vim outside Insert mode lands on the first stop, with no session.
    let mut engine = VimEngine::new();
    let mut doc = Doc::new("\n");
    insert(&mut engine, &mut doc, 0, "foo(${1:a})");
    assert!(!engine.snippet_active());
    assert_eq!(caret(&engine), (0, 4));
}
