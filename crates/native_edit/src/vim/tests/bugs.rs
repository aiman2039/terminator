//! One regression per fixed engine bug, named after the vim behavior.
use super::super::super::doc::{Buffer, Doc};
use super::super::mode::{Cursor, Key, ModalEngine};
use super::super::state::VimEngine;
use super::super::vim_tests::tests::keys;

fn engine(text: &str) -> (VimEngine, Doc) {
    (VimEngine::new(), Doc::new(text))
}

fn esc(eng: &mut VimEngine, doc: &mut Doc) {
    eng.press_key(doc, Key::Escape);
}

#[test]
fn dgg_deletes_lines_up_to_the_first() {
    let (mut eng, mut doc) = engine("a\nb\nc\nd\n");
    keys(&mut eng, &mut doc, "jjdgg");
    assert_eq!(doc.text(), "d\n");
}

#[test]
fn ygg_yanks_and_cgg_changes_linewise() {
    let (mut eng, mut doc) = engine("a\nb\nc\n");
    keys(&mut eng, &mut doc, "jyggGp");
    assert_eq!(doc.text(), "a\nb\nc\na\nb\n");
    let (mut eng, mut doc) = engine("a\nb\nc\n");
    keys(&mut eng, &mut doc, "jcgg");
    eng.type_text(&mut doc, "x");
    esc(&mut eng, &mut doc);
    assert_eq!(doc.text(), "x\nc\n");
}

#[test]
fn count_gg_goes_to_that_line() {
    let (mut eng, mut doc) = engine("a\nb\nc\nd\ne\n");
    keys(&mut eng, &mut doc, "4gg");
    assert_eq!(eng.cursor().line, 3);
}

#[test]
fn count_before_and_after_operator_multiply() {
    let (mut eng, mut doc) = engine("a b c d e f g h\n");
    keys(&mut eng, &mut doc, "2d3w");
    assert_eq!(doc.text(), "g h\n");
    // The repeat deletes the same six words, not 23 or 36.
    let (mut eng, mut doc) = engine("1 2 3 4 5 6 7 8 9 10 11 12 13\n");
    keys(&mut eng, &mut doc, "2d3w.");
    assert_eq!(doc.text(), "13\n");
}

#[test]
fn cw_on_last_char_of_word_changes_only_that_char() {
    let (mut eng, mut doc) = engine("foo bar\n");
    keys(&mut eng, &mut doc, "llcw");
    eng.type_text(&mut doc, "X");
    esc(&mut eng, &mut doc);
    assert_eq!(doc.text(), "foX bar\n");
}

#[test]
fn dw_on_last_word_of_line_keeps_the_newline() {
    let (mut eng, mut doc) = engine("foo bar\nbaz\n");
    keys(&mut eng, &mut doc, "wdw");
    assert_eq!(doc.text(), "foo \nbaz\n");
}

#[test]
fn d_dollar_and_c_dollar_on_empty_line_keep_the_newline() {
    let (mut eng, mut doc) = engine("a\n\nb\n");
    keys(&mut eng, &mut doc, "jd$");
    assert_eq!(doc.text(), "a\n\nb\n");
    keys(&mut eng, &mut doc, "c$");
    eng.type_text(&mut doc, "x");
    esc(&mut eng, &mut doc);
    assert_eq!(doc.text(), "a\nx\nb\n");
}

#[test]
fn dj_dk_and_dg_are_linewise() {
    let (mut eng, mut doc) = engine("one\ntwo\nthree\nfour\n");
    keys(&mut eng, &mut doc, "ldj");
    assert_eq!(doc.text(), "three\nfour\n");
    keys(&mut eng, &mut doc, "jdk");
    assert_eq!(doc.text(), "");
    let (mut eng, mut doc) = engine("one\ntwo\nthree\n");
    keys(&mut eng, &mut doc, "jldG");
    assert_eq!(doc.text(), "one\n");
}

#[test]
fn visual_delete_change_and_cc_fill_the_register() {
    let (mut eng, mut doc) = engine("abc def\n");
    keys(&mut eng, &mut doc, "vlld$p");
    assert_eq!(doc.text(), " defabc\n");
    let (mut eng, mut doc) = engine("abc def\n");
    keys(&mut eng, &mut doc, "vllc");
    esc(&mut eng, &mut doc);
    keys(&mut eng, &mut doc, "$p");
    assert_eq!(doc.text(), " defabc\n");
    let (mut eng, mut doc) = engine("one\ntwo\n");
    keys(&mut eng, &mut doc, "cc");
    esc(&mut eng, &mut doc);
    keys(&mut eng, &mut doc, "jp");
    assert_eq!(doc.text(), "\ntwo\none\n");
}

#[test]
fn visual_block_delete_fills_the_register() {
    let (mut eng, mut doc) = engine("abcd\nefgh\n");
    eng.press_key(&mut doc, Key::CtrlV);
    keys(&mut eng, &mut doc, "ljd$p");
    assert_eq!(doc.text(), "cdab\nghef\n");
}

#[test]
fn yank_keeps_the_cursor_at_the_start_of_the_text() {
    let (mut eng, mut doc) = engine("foo bar baz\n");
    keys(&mut eng, &mut doc, "yw");
    assert_eq!(eng.cursor().col, 0);
    keys(&mut eng, &mut doc, "ye");
    assert_eq!(eng.cursor().col, 0);
    keys(&mut eng, &mut doc, "y$");
    assert_eq!(eng.cursor().col, 0);
    // A backward selection yanks from its start.
    keys(&mut eng, &mut doc, "$vby");
    assert_eq!(eng.cursor().col, 8);
}

#[test]
fn charwise_p_on_empty_line_pastes_on_that_line() {
    let (mut eng, mut doc) = engine("xy\n\nz\n");
    keys(&mut eng, &mut doc, "ylj");
    keys(&mut eng, &mut doc, "p");
    assert_eq!(doc.text(), "xy\nx\nz\n");
}

#[test]
fn blockwise_p_pastes_after_and_capital_p_at_the_cursor() {
    let (mut eng, mut doc) = engine("ab\ncd\n");
    eng.press_key(&mut doc, Key::CtrlV);
    keys(&mut eng, &mut doc, "jy");
    keys(&mut eng, &mut doc, "P");
    assert_eq!(doc.text(), "aab\nccd\n");
    keys(&mut eng, &mut doc, "u0p");
    assert_eq!(doc.text(), "aab\nccd\n");
    keys(&mut eng, &mut doc, "u$p");
    assert_eq!(doc.text(), "aba\ncdc\n");
}

#[test]
fn block_insert_skips_rows_shorter_than_the_block() {
    let (mut eng, mut doc) = engine("abcd\nx\nabcd\n");
    keys(&mut eng, &mut doc, "ll");
    eng.press_key(&mut doc, Key::CtrlV);
    keys(&mut eng, &mut doc, "jjI");
    eng.type_text(&mut doc, "-");
    esc(&mut eng, &mut doc);
    assert_eq!(doc.text(), "ab-cd\nx\nab-cd\n");
}

#[test]
fn slash_search_starts_after_the_cursor() {
    let (mut eng, mut doc) = engine("foo foo\n");
    keys(&mut eng, &mut doc, "/foo");
    eng.press_key(&mut doc, Key::Enter);
    assert_eq!(eng.cursor(), Cursor { line: 0, col: 4 });
}

#[test]
fn substitute_backslash_ampersand_is_a_literal_ampersand() {
    let (mut eng, mut doc) = engine("a b\n");
    keys(&mut eng, &mut doc, ":s/a/\\&/");
    eng.press_key(&mut doc, Key::Enter);
    assert_eq!(doc.text(), "& b\n");
    keys(&mut eng, &mut doc, ":s/b/[&]/");
    eng.press_key(&mut doc, Key::Enter);
    assert_eq!(doc.text(), "& [b]\n");
}

#[test]
fn escape_from_insert_steps_the_cursor_left() {
    let (mut eng, mut doc) = engine("ab\n");
    keys(&mut eng, &mut doc, "A");
    eng.type_text(&mut doc, "cd");
    esc(&mut eng, &mut doc);
    assert_eq!(eng.cursor(), Cursor { line: 0, col: 3 });
    keys(&mut eng, &mut doc, "0i");
    esc(&mut eng, &mut doc);
    assert_eq!(eng.cursor().col, 0);
    keys(&mut eng, &mut doc, "lli");
    esc(&mut eng, &mut doc);
    assert_eq!(eng.cursor().col, 1);
}

#[test]
fn capital_g_lands_on_the_last_real_line() {
    let (mut eng, mut doc) = engine("a\n  b\n");
    keys(&mut eng, &mut doc, "G");
    assert_eq!(eng.cursor(), Cursor { line: 1, col: 2 });
}

#[test]
fn counted_dd_and_yy_at_eof_skip_the_phantom_line() {
    let (mut eng, mut doc) = engine("a\nb\nc\n");
    keys(&mut eng, &mut doc, "j5yyP");
    assert_eq!(doc.text(), "a\nb\nc\nb\nc\n");
    let (mut eng, mut doc) = engine("a\nb\nc\n");
    keys(&mut eng, &mut doc, "j5ddp");
    assert_eq!(doc.text(), "a\nb\nc\n");
}

#[test]
fn x_and_l_step_over_whole_graphemes() {
    // `e` + combining acute, then an emoji with a skin-tone modifier.
    let (mut eng, mut doc) = engine("e\u{301}\u{1F44D}\u{1F3FD}z\n");
    keys(&mut eng, &mut doc, "l");
    assert_eq!(eng.cursor().col, 2);
    keys(&mut eng, &mut doc, "x");
    assert_eq!(doc.text(), "e\u{301}z\n");
    keys(&mut eng, &mut doc, "0x");
    assert_eq!(doc.text(), "z\n");
}

#[test]
fn visual_block_columns_follow_display_width() {
    // A wide char takes two cells: the block over cells 0..2 takes it whole
    // on the first row and two narrow chars on the second.
    let (mut eng, mut doc) = engine("\u{4E2D}x\nabx\n");
    eng.press_key(&mut doc, Key::CtrlV);
    keys(&mut eng, &mut doc, "jld");
    assert_eq!(doc.text(), "x\nx\n");
}

#[test]
fn visual_block_dot_repeat_keeps_display_columns_after_tabs() {
    let (mut eng, mut doc) = engine("\tfoo\n\tbar\n");
    keys(&mut eng, &mut doc, "l");
    eng.press_key(&mut doc, Key::CtrlV);
    keys(&mut eng, &mut doc, "ld");
    assert_eq!(doc.text(), "\to\n\tbar\n");
    keys(&mut eng, &mut doc, "j.");
    assert_eq!(doc.text(), "\to\n\tr\n");
}

#[test]
fn visual_block_change_dot_repeat_keeps_display_columns_after_tabs() {
    let (mut eng, mut doc) = engine("\tfoo\n\tbar\n");
    keys(&mut eng, &mut doc, "l");
    eng.press_key(&mut doc, Key::CtrlV);
    keys(&mut eng, &mut doc, "lc");
    eng.type_text(&mut doc, "X");
    esc(&mut eng, &mut doc);
    assert_eq!(doc.text(), "\tXo\n\tbar\n");
    keys(&mut eng, &mut doc, "j0.");
    assert_eq!(doc.text(), "\tXo\n\tXr\n");
}

#[test]
fn dd_then_p_on_a_blank_line_pastes_the_blank_line() {
    let (mut eng, mut doc) = engine("a\n\nb\n");
    keys(&mut eng, &mut doc, "jdd");
    assert_eq!(doc.text(), "a\nb\n");
    // The host notes the clipboard after a register-only write.
    assert!(eng.take_clipboard_snapshot());
    eng.note_clipboard(Some("older"));
    eng.adopt_clipboard("older");
    keys(&mut eng, &mut doc, "p");
    assert_eq!(doc.text(), "a\nb\n\n");
    let (mut eng, mut doc) = engine("a\n\nb\n");
    keys(&mut eng, &mut doc, "jyyp");
    assert_eq!(doc.text(), "a\n\n\nb\n");
}

#[test]
fn charwise_visual_takes_the_whole_last_grapheme() {
    let (mut eng, mut doc) = engine("e\u{301}z\n");
    keys(&mut eng, &mut doc, "vd");
    assert_eq!(doc.text(), "z\n");
    let (mut eng, mut doc) = engine("ab\n");
    keys(&mut eng, &mut doc, "lvd");
    assert_eq!(doc.text(), "a\n");
}

#[test]
fn crlf_breaks_in_mixed_files_keep_copied_text_and_word_motions_exact() {
    let (mut eng, mut doc) = engine("ab\r\ncd\n");
    // `vl` on `cd` (rope offsets 4..6): the copy is the selected text.
    keys(&mut eng, &mut doc, "jvl");
    assert_eq!(eng.copy_text(&doc), "cd");
    esc(&mut eng, &mut doc);
    // `~` changes the char under the cursor, not its neighbor.
    keys(&mut eng, &mut doc, "0~");
    assert_eq!(doc.text(), "ab\r\nCd\n");
    // `w` leaves a CRLF line.
    keys(&mut eng, &mut doc, "gg");
    keys(&mut eng, &mut doc, "w");
    assert_eq!(eng.cursor(), Cursor { line: 1, col: 0 });
    // An empty CRLF line is still a word stop.
    let (mut eng, mut doc) = engine("a\r\n\r\nb\n");
    keys(&mut eng, &mut doc, "w");
    assert_eq!(eng.cursor(), Cursor { line: 1, col: 0 });
}

#[test]
fn word_motions_work_on_large_buffers_without_reading_everything() {
    let text = "word ".repeat(20_000);
    let (mut eng, mut doc) = engine(&text);
    keys(&mut eng, &mut doc, "3w");
    assert_eq!(eng.cursor().col, 15);
    keys(&mut eng, &mut doc, "be");
    assert_eq!(eng.cursor().col, 13);
    assert_eq!(doc.len_chars(), text.chars().count());
}
