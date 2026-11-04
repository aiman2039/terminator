//! Motions, text objects, search, marks, jumps and scrolling.
use super::super::super::doc::Doc;
use super::super::mode::{Cursor, Key, ModalEngine, Mode};
use super::super::state::{ViewScroll, VimEngine};
use super::super::vim_tests::tests::keys;

fn engine(text: &str) -> (VimEngine, Doc) {
    (VimEngine::new(), Doc::new(text))
}

fn at(line: usize, col: usize) -> Cursor {
    Cursor { line, col }
}

#[test]
fn word_objects_select_words_and_blanks() {
    let (mut eng, mut doc) = engine("foo bar baz\n");
    keys(&mut eng, &mut doc, "wdiw");
    assert_eq!(doc.text(), "foo  baz\n");
    let (mut eng, mut doc) = engine("foo bar baz\n");
    keys(&mut eng, &mut doc, "wdaw");
    assert_eq!(doc.text(), "foo baz\n");
    // `aw` on the last word takes the blank before it.
    let (mut eng, mut doc) = engine("foo bar\n");
    keys(&mut eng, &mut doc, "wdaw");
    assert_eq!(doc.text(), "foo\n");
    let (mut eng, mut doc) = engine("a.b c\n");
    keys(&mut eng, &mut doc, "diW");
    assert_eq!(doc.text(), " c\n");
}

#[test]
fn bracket_and_quote_objects_find_the_enclosing_pair() {
    let (mut eng, mut doc) = engine("f(a, (b), c)\n");
    keys(&mut eng, &mut doc, "fadi(");
    assert_eq!(doc.text(), "f()\n");
    let (mut eng, mut doc) = engine("f(a, (b), c)\n");
    keys(&mut eng, &mut doc, "fbda(");
    assert_eq!(doc.text(), "f(a, , c)\n");
    let (mut eng, mut doc) = engine("x = [1, {k: 2}]\n");
    keys(&mut eng, &mut doc, "fkci{");
    eng.type_text(&mut doc, "z");
    eng.press_key(&mut doc, Key::Escape);
    assert_eq!(doc.text(), "x = [1, {z}]\n");
    let (mut eng, mut doc) = engine("s = \"hi there\" + 'x'\n");
    keys(&mut eng, &mut doc, "di\"");
    assert_eq!(doc.text(), "s = \"\" + 'x'\n");
    keys(&mut eng, &mut doc, "$da'");
    assert_eq!(doc.text(), "s = \"\" +\n");
    let (mut eng, mut doc) = engine("a <b> `q`\n");
    keys(&mut eng, &mut doc, "fbdi<f`di`");
    assert_eq!(doc.text(), "a <> ``\n");
}

#[test]
fn block_object_on_own_lines_is_linewise() {
    let (mut eng, mut doc) = engine("fn x() {\n    a;\n    b;\n}\n");
    keys(&mut eng, &mut doc, "jdi{");
    assert_eq!(doc.text(), "fn x() {\n}\n");
}

#[test]
fn tag_objects_select_inside_and_around_tags() {
    let (mut eng, mut doc) = engine("<a><b>hi</b> x</a>\n");
    keys(&mut eng, &mut doc, "fhdit");
    assert_eq!(doc.text(), "<a><b></b> x</a>\n");
    keys(&mut eng, &mut doc, "dat");
    assert_eq!(doc.text(), "<a> x</a>\n");
    keys(&mut eng, &mut doc, "dit");
    assert_eq!(doc.text(), "<a></a>\n");
}

#[test]
fn paragraph_objects_and_motions_use_blank_lines() {
    let (mut eng, mut doc) = engine("a\nb\n\nc\nd\n");
    keys(&mut eng, &mut doc, "dap");
    assert_eq!(doc.text(), "c\nd\n");
    let (mut eng, mut doc) = engine("a\nb\n\nc\nd\n");
    keys(&mut eng, &mut doc, "yipGp");
    assert_eq!(doc.text(), "a\nb\n\nc\nd\na\nb\n");
    let (mut eng, mut doc) = engine("a\nb\n\nc\n\nd\n");
    keys(&mut eng, &mut doc, "}");
    assert_eq!(eng.cursor(), at(2, 0));
    keys(&mut eng, &mut doc, "2}");
    assert_eq!(eng.cursor(), at(5, 0));
    keys(&mut eng, &mut doc, "{");
    assert_eq!(eng.cursor(), at(4, 0));
    let (mut eng, mut doc) = engine("a\nb\n\nc\n");
    keys(&mut eng, &mut doc, "d}");
    assert_eq!(doc.text(), "\nc\n");
}

#[test]
fn find_motions_repeat_and_reverse() {
    let (mut eng, mut doc) = engine("a,b,c,d\n");
    keys(&mut eng, &mut doc, "f,");
    assert_eq!(eng.cursor().col, 1);
    keys(&mut eng, &mut doc, ";");
    assert_eq!(eng.cursor().col, 3);
    keys(&mut eng, &mut doc, ",");
    assert_eq!(eng.cursor().col, 1);
    keys(&mut eng, &mut doc, "$F,");
    assert_eq!(eng.cursor().col, 5);
    keys(&mut eng, &mut doc, "0t,;");
    assert_eq!(eng.cursor().col, 2);
    keys(&mut eng, &mut doc, "0dt,");
    assert_eq!(doc.text(), ",b,c,d\n");
    keys(&mut eng, &mut doc, "d2f,");
    assert_eq!(doc.text(), "d\n");
}

#[test]
fn percent_jumps_between_matching_brackets() {
    let (mut eng, mut doc) = engine("if (a[1]) {\n  x\n}\n");
    keys(&mut eng, &mut doc, "%");
    assert_eq!(eng.cursor(), at(0, 8));
    keys(&mut eng, &mut doc, "%");
    assert_eq!(eng.cursor(), at(0, 3));
    keys(&mut eng, &mut doc, "$%");
    assert_eq!(eng.cursor(), at(2, 0));
    keys(&mut eng, &mut doc, "d%");
    assert_eq!(doc.text(), "if (a[1]) \n");
}

#[test]
fn big_word_motions_and_empty_lines_as_words() {
    let (mut eng, mut doc) = engine("a.b c.d\n\nx\n");
    keys(&mut eng, &mut doc, "W");
    assert_eq!(eng.cursor(), at(0, 4));
    keys(&mut eng, &mut doc, "w");
    assert_eq!(eng.cursor(), at(0, 5));
    keys(&mut eng, &mut doc, "ww");
    assert_eq!(eng.cursor(), at(1, 0));
    keys(&mut eng, &mut doc, "w");
    assert_eq!(eng.cursor(), at(2, 0));
    keys(&mut eng, &mut doc, "b");
    assert_eq!(eng.cursor(), at(1, 0));
    keys(&mut eng, &mut doc, "B");
    assert_eq!(eng.cursor(), at(0, 4));
    keys(&mut eng, &mut doc, "E");
    assert_eq!(eng.cursor(), at(0, 6));
    keys(&mut eng, &mut doc, "0ge");
    assert_eq!(eng.cursor(), at(0, 0));
    keys(&mut eng, &mut doc, "wge");
    assert_eq!(eng.cursor(), at(0, 0));
}

#[test]
fn screen_motions_and_scrolling_use_the_reported_viewport() {
    let text: String = (0..100).fold(String::new(), |mut acc, i| {
        acc.push_str(&format!("l{i}\n"));
        acc
    });
    let (mut eng, mut doc) = engine(&text);
    eng.set_viewport(10, 29);
    keys(&mut eng, &mut doc, "H");
    assert_eq!(eng.cursor().line, 10);
    keys(&mut eng, &mut doc, "L");
    assert_eq!(eng.cursor().line, 29);
    keys(&mut eng, &mut doc, "M");
    assert_eq!(eng.cursor().line, 19);
    keys(&mut eng, &mut doc, "3H");
    assert_eq!(eng.cursor().line, 12);
    eng.press_key(&mut doc, Key::Ctrl('d'));
    assert_eq!(eng.cursor().line, 22);
    assert_eq!(eng.take_view_scroll(), Some(ViewScroll::Lines(10)));
    eng.press_key(&mut doc, Key::Ctrl('u'));
    assert_eq!(eng.cursor().line, 12);
    keys(&mut eng, &mut doc, "zz");
    assert_eq!(eng.take_view_scroll(), Some(ViewScroll::Center));
    keys(&mut eng, &mut doc, "zt");
    assert_eq!(eng.take_view_scroll(), Some(ViewScroll::Top));
    keys(&mut eng, &mut doc, "zb");
    assert_eq!(eng.take_view_scroll(), Some(ViewScroll::Bottom));
    eng.press_key(&mut doc, Key::Ctrl('f'));
    assert_eq!(eng.cursor().line, 28);
}

#[test]
fn jump_list_walks_back_and_forth() {
    let (mut eng, mut doc) = engine("a\nb\nc\nd\ne\n");
    keys(&mut eng, &mut doc, "G");
    keys(&mut eng, &mut doc, "2gg");
    eng.press_key(&mut doc, Key::Ctrl('o'));
    assert_eq!(eng.cursor().line, 4);
    eng.press_key(&mut doc, Key::Ctrl('o'));
    assert_eq!(eng.cursor().line, 0);
    eng.press_key(&mut doc, Key::Tab);
    assert_eq!(eng.cursor().line, 4);
    eng.press_key(&mut doc, Key::Ctrl('i'));
    assert_eq!(eng.cursor().line, 1);
}

#[test]
fn marks_jump_by_line_and_exactly() {
    let (mut eng, mut doc) = engine("ab\n  cd\nef\n");
    keys(&mut eng, &mut doc, "jllmaG'a");
    assert_eq!(eng.cursor(), at(1, 2));
    keys(&mut eng, &mut doc, "l0`a");
    assert_eq!(eng.cursor(), at(1, 2));
    keys(&mut eng, &mut doc, "gg''");
    assert_eq!(eng.cursor().line, 1);
    keys(&mut eng, &mut doc, "ggd'a");
    assert_eq!(doc.text(), "ef\n");
}

#[test]
fn regex_search_star_hash_and_question_mark() {
    let (mut eng, mut doc) = engine("foo1 bar foo22\nFoo3\n");
    keys(&mut eng, &mut doc, "/foo\\d\\+");
    eng.press_key(&mut doc, Key::Enter);
    assert_eq!(eng.cursor(), at(0, 9));
    keys(&mut eng, &mut doc, "?bar");
    eng.press_key(&mut doc, Key::Enter);
    assert_eq!(eng.cursor(), at(0, 5));
    // `n` keeps the backward direction of `?`.
    keys(&mut eng, &mut doc, "0n");
    assert_eq!(eng.cursor(), at(0, 5));
    keys(&mut eng, &mut doc, "gg*");
    assert_eq!(eng.cursor(), at(0, 0));
    let (mut eng, mut doc) = engine("ab x ab abc\n");
    keys(&mut eng, &mut doc, "*");
    assert_eq!(eng.cursor().col, 5);
    keys(&mut eng, &mut doc, "#");
    assert_eq!(eng.cursor().col, 0);
    keys(&mut eng, &mut doc, "g*g*");
    assert_eq!(eng.cursor().col, 8);
}

#[test]
fn ignorecase_and_smartcase_shape_searches() {
    let (mut eng, mut doc) = engine("x Foo foo\n");
    eng.apply_vimrc(super::super::state::parse_vimrc(
        "set ignorecase smartcase\n",
    ));
    keys(&mut eng, &mut doc, "/foo");
    eng.press_key(&mut doc, Key::Enter);
    assert_eq!(eng.cursor().col, 2);
    keys(&mut eng, &mut doc, "0/Foo");
    eng.press_key(&mut doc, Key::Enter);
    assert_eq!(eng.cursor().col, 2);
    keys(&mut eng, &mut doc, "/foo\\C");
    eng.press_key(&mut doc, Key::Enter);
    assert_eq!(eng.cursor().col, 6);
}

#[test]
fn visual_mode_takes_objects_and_jumps() {
    let (mut eng, mut doc) = engine("one two\nthree\n");
    keys(&mut eng, &mut doc, "wviwy");
    keys(&mut eng, &mut doc, "$p");
    assert_eq!(doc.text(), "one twotwo\nthree\n");
    let (mut eng, mut doc) = engine("a\nb\nc\n");
    keys(&mut eng, &mut doc, "jvGd");
    assert_eq!(doc.text(), "a\n\n");
    let (mut eng, mut doc) = engine("a\nb\nc\n");
    keys(&mut eng, &mut doc, "Gvggd");
    assert_eq!(eng.mode(), Mode::Normal);
    assert_eq!(doc.text(), "\n");
}
