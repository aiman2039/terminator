//! Registers and paste (`p`, `P`, `Y`, `x`, `s`, clipboard), `gf`, Ctrl-^.
use super::mode::{Effect, Key, ModalEngine, Mode};
use super::state::VimEngine;
use super::vim_tests::tests::keys;
use crate::doc::Doc;

#[test]
fn capital_p_pastes_lines_above_and_text_before_the_cursor() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("one\ntwo\n");
    keys(&mut eng, &mut doc, "jYkP");
    assert_eq!(doc.text().as_str(), "two\none\ntwo\n");
    assert_eq!(eng.cursor().line, 0);
    let mut doc = Doc::new("abc");
    keys(&mut eng, &mut doc, "xP");
    assert_eq!(doc.text().as_str(), "abc");
}

#[test]
fn x_and_d_fill_the_register_so_xp_swaps() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("ab cd");
    keys(&mut eng, &mut doc, "xp");
    assert_eq!(doc.text().as_str(), "ba cd");
    keys(&mut eng, &mut doc, "0dw$p");
    assert_eq!(doc.text().as_str(), "cdba ");
}

#[test]
fn s_changes_counted_chars_and_repeats() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("abcdef");
    keys(&mut eng, &mut doc, "2s");
    assert_eq!(eng.mode(), Mode::Insert);
    eng.type_text(&mut doc, "X");
    eng.press_key(&mut doc, Key::Escape);
    assert_eq!(doc.text().as_str(), "Xcdef");
    keys(&mut eng, &mut doc, "0l.");
    assert_eq!(doc.text().as_str(), "XXef");
}

#[test]
fn visual_s_changes_the_selection() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("hello");
    keys(&mut eng, &mut doc, "vls");
    eng.type_text(&mut doc, "J");
    eng.press_key(&mut doc, Key::Escape);
    assert_eq!(doc.text().as_str(), "Jllo");
}

#[test]
fn p_pastes_clipboard_text_copied_elsewhere() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("a\nb\n");
    eng.adopt_clipboard("XY");
    keys(&mut eng, &mut doc, "p");
    assert_eq!(doc.text().as_str(), "aXY\nb\n");
    // Text with a final newline pastes as whole lines.
    eng.adopt_clipboard("line\n");
    keys(&mut eng, &mut doc, "p");
    assert_eq!(doc.text().as_str(), "aXY\nline\nb\n");
}

#[test]
fn own_linewise_yank_keeps_its_shape_after_clipboard_round_trip() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("one\ntwo\n");
    keys(&mut eng, &mut doc, "yy");
    let staged = eng.take_clipboard().expect("staged");
    eng.adopt_clipboard(&staged);
    keys(&mut eng, &mut doc, "p");
    assert_eq!(doc.text().as_str(), "one\none\ntwo\n");
}

#[test]
fn gf_reports_the_file_name_under_the_cursor() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("see src/main.rs:12: here");
    let fx = keys(&mut eng, &mut doc, "wwgf");
    assert_eq!(fx, vec![Effect::GotoFile("src/main.rs:12".into())]);
    let mut doc = Doc::new("   ");
    assert_eq!(keys(&mut eng, &mut doc, "gf"), vec![Effect::Bell]);
}

#[test]
fn ctrl_caret_asks_for_the_alternate_file() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("x");
    assert_eq!(
        eng.press_key(&mut doc, Key::CtrlCaret),
        vec![Effect::AlternateFile]
    );
    assert_eq!(doc.text().as_str(), "x");
}

#[test]
fn delete_and_change_do_not_overwrite_the_system_clipboard() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("ab cd");
    keys(&mut eng, &mut doc, "x");
    assert_eq!(eng.take_clipboard(), None);
    keys(&mut eng, &mut doc, "dw");
    assert_eq!(eng.take_clipboard(), None);
}

#[test]
fn p_keeps_a_newer_delete_until_the_clipboard_changes() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("ab cd");
    keys(&mut eng, &mut doc, "x");
    assert!(eng.take_clipboard_snapshot());
    eng.note_clipboard(Some("old"));
    eng.adopt_clipboard("old");
    keys(&mut eng, &mut doc, "p");
    assert_eq!(doc.text().as_str(), "ba cd");
    // Copied elsewhere after the delete: the clipboard wins.
    eng.adopt_clipboard("new");
    keys(&mut eng, &mut doc, "0P");
    assert_eq!(doc.text().as_str(), "newba cd");
}

#[test]
fn p_after_a_yank_in_the_same_frame_ignores_the_stale_clipboard() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("one\n");
    keys(&mut eng, &mut doc, "yy");
    eng.adopt_clipboard("stale external text");
    keys(&mut eng, &mut doc, "p");
    assert_eq!(doc.text().as_str(), "one\none\n");
}

#[test]
fn s_on_the_last_char_replaces_it() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("abc");
    keys(&mut eng, &mut doc, "$s");
    eng.type_text(&mut doc, "x");
    eng.press_key(&mut doc, Key::Escape);
    assert_eq!(doc.text().as_str(), "abx");
    let mut doc = Doc::new("abcd");
    keys(&mut eng, &mut doc, "0ll3s");
    eng.press_key(&mut doc, Key::Escape);
    assert_eq!(doc.text().as_str(), "ab");
}

#[test]
fn s_under_an_operator_bells_and_changes_nothing() {
    for op in ["y", "d", "c"] {
        let mut eng = VimEngine::new();
        let mut doc = Doc::new("abc");
        let fx = keys(&mut eng, &mut doc, &format!("{op}s"));
        assert_eq!(fx, vec![Effect::Bell], "{op}s");
        assert_eq!(doc.text().as_str(), "abc");
        assert_eq!(eng.mode(), Mode::Normal);
    }
}

#[test]
fn counted_capital_p_pastes_count_times_like_its_repeat() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("x");
    keys(&mut eng, &mut doc, "yl3P");
    assert_eq!(doc.text().as_str(), "xxxx");
    keys(&mut eng, &mut doc, ".");
    assert_eq!(doc.text().as_str(), "xxxxxxx");
}

#[test]
fn multi_line_charwise_paste_lands_on_the_first_pasted_char() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("foo\nbar zz");
    keys(&mut eng, &mut doc, "ldj");
    assert_eq!(doc.text().as_str(), "fr zz");
    keys(&mut eng, &mut doc, "P");
    assert_eq!(doc.text().as_str(), "foo\nbar zz");
    assert_eq!((eng.cursor().line, eng.cursor().col), (0, 1));
}

#[test]
fn gf_on_the_line_number_still_names_the_file() {
    let mut eng = VimEngine::new();
    let mut doc = Doc::new("src/main.rs:42:7");
    keys(&mut eng, &mut doc, "$hh");
    assert_eq!(
        keys(&mut eng, &mut doc, "gf"),
        vec![Effect::GotoFile("src/main.rs:42:7".into())]
    );
}
