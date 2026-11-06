//! Edits, registers, macros, ex commands, insert keys and mappings.
use super::super::super::doc::Doc;
use super::super::mode::{Effect, Key, ModalEngine, Mode};
use super::super::state::{VimEngine, parse_vimrc};
use super::super::vim_tests::tests::keys;

fn engine(text: &str) -> (VimEngine, Doc) {
    (VimEngine::new(), Doc::new(text))
}

fn ex(eng: &mut VimEngine, doc: &mut Doc, line: &str) -> Vec<Effect> {
    keys(eng, doc, line);
    eng.press_key(doc, Key::Enter)
}

#[test]
fn replace_join_and_case_commands() {
    let (mut eng, mut doc) = engine("abcd\n  ef\ngh\n");
    keys(&mut eng, &mut doc, "2rx");
    assert_eq!(doc.text(), "xxcd\n  ef\ngh\n");
    keys(&mut eng, &mut doc, "J");
    assert_eq!(doc.text(), "xxcd ef\ngh\n");
    keys(&mut eng, &mut doc, "gJ");
    assert_eq!(doc.text(), "xxcd efgh\n");
    keys(&mut eng, &mut doc, "0~~");
    assert_eq!(doc.text(), "XXcd efgh\n");
    keys(&mut eng, &mut doc, "gUiw");
    assert_eq!(doc.text(), "XXCD efgh\n");
    keys(&mut eng, &mut doc, "guu");
    assert_eq!(doc.text(), "xxcd efgh\n");
    keys(&mut eng, &mut doc, "g~~");
    assert_eq!(doc.text(), "XXCD EFGH\n");
}

#[test]
fn shorthand_deletes_and_changes() {
    let (mut eng, mut doc) = engine("abc def\nxyz\n");
    keys(&mut eng, &mut doc, "wD");
    assert_eq!(doc.text(), "abc \nxyz\n");
    keys(&mut eng, &mut doc, "X");
    assert_eq!(doc.text(), "ab \nxyz\n");
    keys(&mut eng, &mut doc, "0C");
    eng.type_text(&mut doc, "q");
    eng.press_key(&mut doc, Key::Escape);
    assert_eq!(doc.text(), "q\nxyz\n");
    keys(&mut eng, &mut doc, "jS");
    eng.type_text(&mut doc, "z");
    eng.press_key(&mut doc, Key::Escape);
    assert_eq!(doc.text(), "q\nz\n");
}

#[test]
fn replace_mode_overwrites_and_backspace_restores() {
    let (mut eng, mut doc) = engine("abcd\n");
    keys(&mut eng, &mut doc, "lR");
    eng.type_text(&mut doc, "XYZW");
    assert_eq!(doc.text(), "aXYZW\n");
    eng.press_key(&mut doc, Key::Backspace);
    eng.press_key(&mut doc, Key::Backspace);
    assert_eq!(doc.text(), "aXYd\n");
    eng.press_key(&mut doc, Key::Escape);
    assert_eq!(eng.mode(), Mode::Normal);
}

#[test]
fn shift_operators_indent_lines() {
    let (mut eng, mut doc) = engine("a\nb\nc\n");
    keys(&mut eng, &mut doc, ">>");
    assert_eq!(doc.text(), "    a\nb\nc\n");
    keys(&mut eng, &mut doc, "j>j");
    assert_eq!(doc.text(), "    a\n    b\n    c\n");
    keys(&mut eng, &mut doc, "gg2<<");
    assert_eq!(doc.text(), "a\nb\n    c\n");
    keys(&mut eng, &mut doc, "G.");
    assert_eq!(doc.text(), "a\nb\nc\n");
}

#[test]
fn ctrl_a_and_ctrl_x_change_numbers() {
    let (mut eng, mut doc) = engine("x = 9, y = -3, z = 0x0f\n");
    eng.press_key(&mut doc, Key::Ctrl('a'));
    assert_eq!(doc.text(), "x = 10, y = -3, z = 0x0f\n");
    keys(&mut eng, &mut doc, "w");
    keys(&mut eng, &mut doc, "5");
    eng.press_key(&mut doc, Key::Ctrl('a'));
    assert_eq!(doc.text(), "x = 10, y = 2, z = 0x0f\n");
    keys(&mut eng, &mut doc, "f0");
    eng.press_key(&mut doc, Key::Ctrl('a'));
    assert_eq!(doc.text(), "x = 10, y = 2, z = 0x10\n");
    eng.press_key(&mut doc, Key::Ctrl('x'));
    assert_eq!(doc.text(), "x = 10, y = 2, z = 0x0f\n");
}

#[test]
fn counts_on_undo_redo_dot_and_inserts() {
    let (mut eng, mut doc) = engine("abcdef\n");
    keys(&mut eng, &mut doc, "xxx2u");
    assert_eq!(doc.text(), "bcdef\n");
    eng.press_key(&mut doc, Key::Ctrl('r'));
    assert_eq!(doc.text(), "cdef\n");
    keys(&mut eng, &mut doc, "2.");
    assert_eq!(doc.text(), "ef\n");
    let (mut eng, mut doc) = engine("x\n");
    keys(&mut eng, &mut doc, "3i");
    eng.type_text(&mut doc, "ab");
    eng.press_key(&mut doc, Key::Escape);
    assert_eq!(doc.text(), "abababx\n");
    let (mut eng, mut doc) = engine("x\n");
    keys(&mut eng, &mut doc, "2o");
    eng.type_text(&mut doc, "y");
    eng.press_key(&mut doc, Key::Escape);
    assert_eq!(doc.text(), "x\ny\ny\n");
}

#[test]
fn dot_repeats_text_object_changes() {
    let (mut eng, mut doc) = engine("foo bar baz\n");
    keys(&mut eng, &mut doc, "ciw");
    eng.type_text(&mut doc, "X");
    eng.press_key(&mut doc, Key::Escape);
    keys(&mut eng, &mut doc, "w.");
    assert_eq!(doc.text(), "X X baz\n");
}

#[test]
fn named_yank_black_hole_and_append_registers() {
    let (mut eng, mut doc) = engine("one\ntwo\n");
    keys(&mut eng, &mut doc, "\"ayyj\"Ayy\"_ddk\"ap");
    assert_eq!(doc.text(), "one\none\ntwo\n");
    let (mut eng, mut doc) = engine("ab\n");
    keys(&mut eng, &mut doc, "yl\"_xP");
    assert_eq!(doc.text(), "ab\n");
    let (mut eng, mut doc) = engine("ab\n");
    keys(&mut eng, &mut doc, "ylx\"0P");
    assert_eq!(doc.text(), "ab\n");
}

#[test]
fn macros_record_replay_and_repeat() {
    let (mut eng, mut doc) = engine("a\nb\nc\nd\n");
    keys(&mut eng, &mut doc, "qaA");
    eng.type_text(&mut doc, "!");
    eng.press_key(&mut doc, Key::Escape);
    keys(&mut eng, &mut doc, "jq");
    keys(&mut eng, &mut doc, "@a");
    keys(&mut eng, &mut doc, "@@");
    assert_eq!(doc.text(), "a!\nb!\nc!\nd\n");
    assert_eq!(eng.recording_macro(), None);
}

#[test]
fn ex_ranges_and_line_commands() {
    let (mut eng, mut doc) = engine("a1\na2\na3\na4\n");
    ex(&mut eng, &mut doc, ":2,3s/a/b/");
    assert_eq!(doc.text(), "a1\nb2\nb3\na4\n");
    ex(&mut eng, &mut doc, ":%s/\\(\\w\\)\\(\\d\\)/\\2\\1/g");
    assert_eq!(doc.text(), "1a\n2b\n3b\n4a\n");
    ex(&mut eng, &mut doc, ":s/A/x/i");
    assert_eq!(doc.text(), "1x\n2b\n3b\n4a\n");
    ex(&mut eng, &mut doc, ":1d");
    assert_eq!(doc.text(), "2b\n3b\n4a\n");
    ex(&mut eng, &mut doc, ":1m$");
    assert_eq!(doc.text(), "3b\n4a\n2b\n");
    ex(&mut eng, &mut doc, ":3t0");
    assert_eq!(doc.text(), "2b\n3b\n4a\n2b\n");
    ex(&mut eng, &mut doc, ":1,2j");
    assert_eq!(doc.text(), "2b 3b\n4a\n2b\n");
    assert!(ex(&mut eng, &mut doc, ":noh").is_empty());
    assert_eq!(ex(&mut eng, &mut doc, ":%s/a/b/c"), vec![Effect::Bell]);
    let fx = ex(&mut eng, &mut doc, ":%s/b/z/gn");
    assert!(fx.is_empty());
    assert_eq!(doc.text(), "2b 3b\n4a\n2b\n");
}

#[test]
fn global_command_deletes_and_moves_matching_lines() {
    let (mut eng, mut doc) = engine("x1\ny\nx2\nz\n");
    ex(&mut eng, &mut doc, ":g/x/d");
    assert_eq!(doc.text(), "y\nz\n");
    let (mut eng, mut doc) = engine("a\nb\nc\n");
    ex(&mut eng, &mut doc, ":g/^/m0");
    assert_eq!(doc.text(), "c\nb\na\n");
    let (mut eng, mut doc) = engine("x1\ny\nx2\n");
    ex(&mut eng, &mut doc, ":v/x/s/y/Y/");
    assert_eq!(doc.text(), "x1\nY\nx2\n");
}

#[test]
fn visual_colon_runs_on_the_selection_and_edit_opens_files() {
    let (mut eng, mut doc) = engine("a\na\na\n");
    keys(&mut eng, &mut doc, "jVj:");
    assert_eq!(eng.mode(), Mode::Command);
    keys(&mut eng, &mut doc, "s/a/b/");
    eng.press_key(&mut doc, Key::Enter);
    assert_eq!(doc.text(), "a\nb\nb\n");
    let fx = ex(&mut eng, &mut doc, ":e src/main.rs");
    assert_eq!(fx, vec![Effect::EditFile("src/main.rs".into())]);
}

#[test]
fn edit_hash_is_the_alternate_file_and_bare_edit_reloads() {
    let (mut eng, mut doc) = engine("a\n");
    assert_eq!(ex(&mut eng, &mut doc, ":e#"), vec![Effect::AlternateFile]);
    assert_eq!(ex(&mut eng, &mut doc, ":e #"), vec![Effect::AlternateFile]);
    assert_eq!(
        ex(&mut eng, &mut doc, ":e"),
        vec![Effect::Reload { force: false }]
    );
    assert_eq!(
        ex(&mut eng, &mut doc, ":e!"),
        vec![Effect::Reload { force: true }]
    );
    assert_eq!(
        ex(&mut eng, &mut doc, ":e! new.rs"),
        vec![Effect::EditFile("new.rs".into())]
    );
}

#[test]
fn visual_edits_case_join_replace_put_and_gv() {
    let (mut eng, mut doc) = engine("abc\ndef\n");
    keys(&mut eng, &mut doc, "vlU");
    assert_eq!(doc.text(), "ABc\ndef\n");
    keys(&mut eng, &mut doc, "gvu");
    assert_eq!(doc.text(), "abc\ndef\n");
    keys(&mut eng, &mut doc, "vl~");
    assert_eq!(doc.text(), "ABc\ndef\n");
    keys(&mut eng, &mut doc, "vlrx");
    assert_eq!(doc.text(), "xxc\ndef\n");
    keys(&mut eng, &mut doc, "VjJ");
    assert_eq!(doc.text(), "xxc def\n");
    keys(&mut eng, &mut doc, "0yiw$vp");
    assert_eq!(doc.text(), "xxc dexxc\n");
}

#[test]
fn insert_ctrl_w_ctrl_u_and_ctrl_o() {
    let (mut eng, mut doc) = engine("\n");
    keys(&mut eng, &mut doc, "i");
    eng.type_text(&mut doc, "  foo bar");
    eng.press_key(&mut doc, Key::Ctrl('w'));
    assert_eq!(doc.text(), "  foo \n");
    eng.press_key(&mut doc, Key::Ctrl('u'));
    assert_eq!(doc.text(), "  \n");
    eng.type_text(&mut doc, "ab");
    eng.press_key(&mut doc, Key::Ctrl('o'));
    assert_eq!(eng.mode(), Mode::Normal);
    keys(&mut eng, &mut doc, "0");
    assert_eq!(eng.mode(), Mode::Insert);
    eng.type_text(&mut doc, "X");
    assert_eq!(doc.text(), "X  ab\n");
}

#[test]
fn vimrc_mappings_expand_in_normal_and_insert() {
    let rc = "let mapleader = \",\"\nnnoremap ,d dd\ninoremap jk <Esc>\nnmap Q ,d\nnnoremap Y y$\n";
    let (mut eng, mut doc) = engine("one\ntwo\nthree\n");
    eng.apply_vimrc(parse_vimrc(rc));
    keys(&mut eng, &mut doc, ",d");
    assert_eq!(doc.text(), "two\nthree\n");
    keys(&mut eng, &mut doc, "Q");
    assert_eq!(doc.text(), "three\n");
    keys(&mut eng, &mut doc, "A");
    eng.type_text(&mut doc, "!");
    eng.type_text(&mut doc, "j");
    eng.type_text(&mut doc, "k");
    assert_eq!(eng.mode(), Mode::Normal);
    assert_eq!(doc.text(), "three!\n");
    // A held `j` that turns out not to be `jk` types normally.
    keys(&mut eng, &mut doc, "a");
    eng.type_text(&mut doc, "j");
    eng.type_text(&mut doc, "x");
    eng.press_key(&mut doc, Key::Escape);
    assert_eq!(doc.text(), "three!jx\n");
}
