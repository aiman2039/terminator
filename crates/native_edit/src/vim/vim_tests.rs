use super::super::doc::Buffer;
use super::mode::{Cursor, Effect, Key, ModalEngine, Mode};
use super::state::{VimEngine, parse_vimrc};
#[cfg(test)]
mod tests {
    use super::*;

    use crate::doc::Doc;

    fn engine(text: &str) -> (VimEngine, Doc) {
        (VimEngine::new(), Doc::new(text))
    }

    fn keys<B: Buffer>(eng: &mut VimEngine, doc: &mut B, seq: &str) -> Vec<Effect> {
        let mut out = Vec::new();
        for c in seq.chars() {
            out.extend(eng.press_key(doc, Key::Char(c)));
        }
        out
    }

    #[test]
    fn hjkl_moves_with_counts_and_clamps() {
        // "ab\ncdef\n" is two vim lines; the trailing empty row is the file's
        // final newline, not an addressable line.
        let (mut eng, mut doc) = engine("ab\ncdef\n");
        keys(&mut eng, &mut doc, "l");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 1 });
        keys(&mut eng, &mut doc, "2j");
        assert_eq!(eng.cursor().line, 1);
        keys(&mut eng, &mut doc, "10k");
        assert_eq!(eng.cursor().line, 0);
    }

    #[test]
    fn word_motions_cross_words() {
        let (mut eng, mut doc) = engine("foo bar  baz");
        keys(&mut eng, &mut doc, "w");
        assert_eq!(eng.cursor().col, 4);
        keys(&mut eng, &mut doc, "e");
        assert_eq!(eng.cursor().col, 6);
        keys(&mut eng, &mut doc, "b");
        assert_eq!(eng.cursor().col, 4);
    }

    #[test]
    fn delete_word_removes_through_blank() {
        let (mut eng, mut doc) = engine("foo bar");
        keys(&mut eng, &mut doc, "dw");
        assert_eq!(doc.text().as_str(), "bar");
    }

    #[test]
    fn change_word_enters_insert() {
        let (mut eng, mut doc) = engine("foo bar");
        keys(&mut eng, &mut doc, "cw");
        assert_eq!(eng.mode(), Mode::Insert);
        assert_eq!(doc.text().as_str(), " bar");
        eng.type_text(&mut doc, "baz");
        assert_eq!(doc.text().as_str(), "baz bar");
    }

    #[test]
    fn line_operators_delete_yank_paste() {
        let (mut eng, mut doc) = engine("one\ntwo\nthree\n");
        keys(&mut eng, &mut doc, "j");
        keys(&mut eng, &mut doc, "dd");
        assert_eq!(doc.text().as_str(), "one\nthree\n");
        keys(&mut eng, &mut doc, "p");
        assert_eq!(doc.text().as_str(), "one\nthree\ntwo\n");
    }

    #[test]
    fn undo_restores_deleted_line() {
        let (mut eng, mut doc) = engine("one\ntwo\n");
        keys(&mut eng, &mut doc, "dd");
        assert_eq!(doc.text().as_str(), "two\n");
        keys(&mut eng, &mut doc, "u");
        assert_eq!(doc.text().as_str(), "one\ntwo\n");
    }

    #[test]
    fn open_line_below_and_escape() {
        let (mut eng, mut doc) = engine("a\nb\n");
        keys(&mut eng, &mut doc, "o");
        assert_eq!(eng.mode(), Mode::Insert);
        eng.type_text(&mut doc, "x");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "a\nx\nb\n");
        assert_eq!(eng.mode(), Mode::Normal);
    }

    #[test]
    fn visual_yank_and_paste() {
        let (mut eng, mut doc) = engine("hello");
        keys(&mut eng, &mut doc, "vll");
        assert!(eng.selection().is_some());
        keys(&mut eng, &mut doc, "y");
        keys(&mut eng, &mut doc, "$p");
        assert_eq!(doc.text().as_str(), "hellohel");
    }

    #[test]
    fn ex_save_quit_and_unknown() {
        let (mut eng, mut doc) = engine("x");
        keys(&mut eng, &mut doc, ":qw");
        let fx = eng.press_key(&mut doc, Key::Enter);
        assert_eq!(fx, vec![Effect::WriteQuit { force: false }]);
        keys(&mut eng, &mut doc, ":nope");
        let fx = eng.press_key(&mut doc, Key::Enter);
        assert_eq!(fx, vec![Effect::Bell]);
        assert_eq!(eng.mode(), Mode::Normal);
    }

    #[test]
    fn visual_block_delete_rectangle() {
        let (mut eng, mut doc) = engine("abcd\nefgh\nijkl\n");
        eng.press_key(&mut doc, Key::CtrlV);
        assert_eq!(eng.mode(), Mode::VisualBlock);
        keys(&mut eng, &mut doc, "ljj");
        keys(&mut eng, &mut doc, "d");
        assert_eq!(eng.mode(), Mode::Normal);
        assert_eq!(doc.text().as_str(), "cd\ngh\nkl\n");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn visual_block_delete_touches_short_rows() {
        // Verified against real Neovim: the rectangle covers column 0 of
        // the short row, so its character goes too.
        let (mut eng, mut doc) = engine("abcd\nx\nijkl\n");
        eng.press_key(&mut doc, Key::CtrlV);
        keys(&mut eng, &mut doc, "lljj");
        keys(&mut eng, &mut doc, "d");
        assert_eq!(doc.text().as_str(), "d\n\nl\n");
    }

    #[test]
    fn visual_block_ctrl_v_toggles_back_to_normal() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        eng.press_key(&mut doc, Key::CtrlV);
        assert_eq!(eng.mode(), Mode::VisualBlock);
        eng.press_key(&mut doc, Key::CtrlV);
        assert_eq!(eng.mode(), Mode::Normal);
        assert!(eng.selection().is_none());
    }

    #[test]
    fn visual_block_yank_pastes_rectangle() {
        let (mut eng, mut doc) = engine("abcd\nefgh\n");
        eng.press_key(&mut doc, Key::CtrlV);
        keys(&mut eng, &mut doc, "lj");
        keys(&mut eng, &mut doc, "y");
        keys(&mut eng, &mut doc, "ll");
        keys(&mut eng, &mut doc, "p");
        assert_eq!(doc.text().as_str(), "ababcd\nefefgh\n");
    }

    #[test]
    fn visual_block_insert_replays_on_other_rows() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        eng.press_key(&mut doc, Key::CtrlV);
        keys(&mut eng, &mut doc, "jI");
        eng.type_text(&mut doc, "X");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(eng.mode(), Mode::Normal);
        assert_eq!(doc.text().as_str(), "Xab\nXcd\n");
    }

    #[test]
    fn visual_block_append_uses_right_edge() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        eng.press_key(&mut doc, Key::CtrlV);
        keys(&mut eng, &mut doc, "ljA");
        eng.type_text(&mut doc, "Y");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "abY\ncdY\n");
    }

    #[test]
    fn visual_block_insert_is_one_undo() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        eng.press_key(&mut doc, Key::CtrlV);
        keys(&mut eng, &mut doc, "jI");
        eng.type_text(&mut doc, "X");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "Xab\nXcd\n");
        eng.press_key(&mut doc, Key::Char('u'));
        assert_eq!(doc.text().as_str(), "ab\ncd\n");
    }

    #[test]
    fn visual_o_swaps_selection_ends() {
        let (mut eng, mut doc) = engine("abc");
        keys(&mut eng, &mut doc, "vl");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 1 });
        keys(&mut eng, &mut doc, "o");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
        assert_eq!(eng.copy_text(&doc), "ab");
    }

    #[test]
    fn dot_repeats_insert_at_new_spot() {
        let (mut eng, mut doc) = engine("hi");
        keys(&mut eng, &mut doc, "i");
        eng.type_text(&mut doc, "ab");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "abhi");
        keys(&mut eng, &mut doc, "$.");
        assert_eq!(doc.text().as_str(), "abhabi");
        assert_eq!(eng.mode(), Mode::Normal);
    }

    #[test]
    fn dot_without_change_bells() {
        let (mut eng, mut doc) = engine("hi");
        assert_eq!(keys(&mut eng, &mut doc, "."), vec![Effect::Bell]);
        assert_eq!(doc.text().as_str(), "hi");
    }

    #[test]
    fn dot_repeats_delete_word() {
        let (mut eng, mut doc) = engine("foo bar\nbaz qux\n");
        keys(&mut eng, &mut doc, "dw");
        assert_eq!(doc.text().as_str(), "bar\nbaz qux\n");
        keys(&mut eng, &mut doc, "j.");
        assert_eq!(doc.text().as_str(), "bar\nqux\n");
    }

    #[test]
    fn dot_repeats_delete_line() {
        let (mut eng, mut doc) = engine("one\ntwo\nthree\n");
        keys(&mut eng, &mut doc, "jdd");
        assert_eq!(doc.text().as_str(), "one\nthree\n");
        keys(&mut eng, &mut doc, ".");
        assert_eq!(doc.text().as_str(), "one\n");
    }

    #[test]
    fn dot_repeats_change_with_typed_text() {
        let (mut eng, mut doc) = engine("foo bar");
        keys(&mut eng, &mut doc, "cw");
        eng.type_text(&mut doc, "baz");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "baz bar");
        keys(&mut eng, &mut doc, "w.");
        assert_eq!(doc.text().as_str(), "baz baz");
    }

    #[test]
    fn dot_repeats_block_insert() {
        let (mut eng, mut doc) = engine("ab\ncd\nef\n");
        eng.press_key(&mut doc, Key::CtrlV);
        keys(&mut eng, &mut doc, "jI");
        eng.type_text(&mut doc, "X");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "Xab\nXcd\nef\n");
        keys(&mut eng, &mut doc, "j.");
        assert_eq!(doc.text().as_str(), "Xab\nXXcd\nXef\n");
    }

    #[test]
    fn yank_does_not_disturb_dot() {
        let (mut eng, mut doc) = engine("foo bar baz");
        keys(&mut eng, &mut doc, "dw");
        assert_eq!(doc.text().as_str(), "bar baz");
        keys(&mut eng, &mut doc, "yy");
        keys(&mut eng, &mut doc, "w.");
        assert_eq!(doc.text().as_str(), "bar ");
    }

    #[test]
    fn change_word_undoes_as_one_step() {
        let (mut eng, mut doc) = engine("foo bar");
        keys(&mut eng, &mut doc, "cw");
        eng.type_text(&mut doc, "baz");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "baz bar");
        keys(&mut eng, &mut doc, "u");
        assert_eq!(doc.text().as_str(), "foo bar");
        eng.redo(&mut doc);
        assert_eq!(doc.text().as_str(), "baz bar");
    }

    #[test]
    fn open_line_typing_undoes_as_one_step() {
        let (mut eng, mut doc) = engine("a\n");
        keys(&mut eng, &mut doc, "o");
        eng.type_text(&mut doc, "x");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "a\nx\n");
        keys(&mut eng, &mut doc, "u");
        assert_eq!(doc.text().as_str(), "a\n");
    }

    #[test]
    fn substitute_first_match_on_line() {
        let (mut eng, mut doc) = engine("foo foo\n");
        keys(&mut eng, &mut doc, ":s/foo/bar");
        assert_eq!(eng.press_key(&mut doc, Key::Enter), Vec::new());
        assert_eq!(doc.text().as_str(), "bar foo\n");
        assert_eq!(eng.mode(), Mode::Normal);
    }

    #[test]
    fn substitute_global_and_percent_range() {
        let (mut eng, mut doc) = engine("foo foo\nfoo\n");
        keys(&mut eng, &mut doc, ":%s/foo/bar/g");
        assert_eq!(eng.press_key(&mut doc, Key::Enter), Vec::new());
        assert_eq!(doc.text().as_str(), "bar bar\nbar\n");
        // One change, one undo step.
        keys(&mut eng, &mut doc, "u");
        assert_eq!(doc.text().as_str(), "foo foo\nfoo\n");
    }

    #[test]
    fn substitute_ampersand_splices_match() {
        let (mut eng, mut doc) = engine("foo\n");
        keys(&mut eng, &mut doc, ":s/foo/<&>/");
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(doc.text().as_str(), "<foo>\n");
    }

    #[test]
    fn substitute_escaped_delimiter_and_alt_delimiter() {
        let (mut eng, mut doc) = engine("a/b\n");
        keys(&mut eng, &mut doc, ":s/\\//X/");
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(doc.text().as_str(), "aXb\n");
        let (mut eng, mut doc) = engine("a\n");
        keys(&mut eng, &mut doc, ":s#a#b#");
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(doc.text().as_str(), "b\n");
    }

    #[test]
    fn substitute_empty_pattern_reuses_last_search() {
        let (mut eng, mut doc) = engine("foo foo\n");
        keys(&mut eng, &mut doc, "/foo");
        eng.press_key(&mut doc, Key::Enter);
        keys(&mut eng, &mut doc, ":s//bar/");
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(doc.text().as_str(), "bar foo\n");
    }

    #[test]
    fn substitute_no_match_and_bad_flags_bell() {
        let (mut eng, mut doc) = engine("foo\n");
        keys(&mut eng, &mut doc, ":s/zzz/q");
        assert_eq!(eng.press_key(&mut doc, Key::Enter), vec![Effect::Bell]);
        assert_eq!(doc.text().as_str(), "foo\n");
        keys(&mut eng, &mut doc, ":s/foo/bar/z");
        assert_eq!(eng.press_key(&mut doc, Key::Enter), vec![Effect::Bell]);
        assert_eq!(doc.text().as_str(), "foo\n");
    }

    #[test]
    fn ex_split_vsplit_tabnew_effects() {
        fn ex(command: &str) -> Vec<Effect> {
            let (mut eng, mut doc) = engine("x");
            keys(&mut eng, &mut doc, &format!(":{command}"));
            eng.press_key(&mut doc, Key::Enter)
        }
        assert_eq!(ex("split"), vec![Effect::Split]);
        assert_eq!(ex("sp"), vec![Effect::Split]);
        assert_eq!(ex("vsplit"), vec![Effect::Vsplit]);
        assert_eq!(ex("vs"), vec![Effect::Vsplit]);
        assert_eq!(ex("tabnew"), vec![Effect::TabNew]);
    }

    #[test]
    fn yank_stages_clipboard_and_take_clears() {
        let (mut eng, mut doc) = engine("foo\nbar\n");
        assert_eq!(eng.take_clipboard(), None);
        keys(&mut eng, &mut doc, "yy");
        assert_eq!(eng.take_clipboard().as_deref(), Some("foo"));
        assert_eq!(eng.take_clipboard(), None);
    }

    #[test]
    fn delete_lines_stage_clipboard_too() {
        // `clipboard=unnamed`: register writes sync, deletes included.
        let (mut eng, mut doc) = engine("foo\nbar\n");
        keys(&mut eng, &mut doc, "dd");
        assert_eq!(eng.take_clipboard().as_deref(), Some("foo"));
    }

    #[test]
    fn normal_backspace_moves_and_delete_deletes() {
        let (mut eng, mut doc) = engine("ab");
        keys(&mut eng, &mut doc, "l");
        eng.press_key(&mut doc, Key::Backspace);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
        eng.press_key(&mut doc, Key::Delete);
        assert_eq!(doc.text().as_str(), "b");
    }

    #[test]
    fn ex_write_quit_matrix() {
        fn ex(command: &str) -> Vec<Effect> {
            let (mut eng, mut doc) = engine("x");
            keys(&mut eng, &mut doc, &format!(":{command}"));
            eng.press_key(&mut doc, Key::Enter)
        }
        assert_eq!(ex("w"), vec![Effect::Save]);
        assert_eq!(ex("w!"), vec![Effect::SaveForce]);
        assert_eq!(ex("q"), vec![Effect::Quit]);
        assert_eq!(ex("q!"), vec![Effect::QuitForce]);
        assert_eq!(ex("wq"), vec![Effect::WriteQuit { force: false }]);
        assert_eq!(ex("qw"), vec![Effect::WriteQuit { force: false }]);
        assert_eq!(ex("x"), vec![Effect::WriteQuit { force: false }]);
        assert_eq!(ex("wq!"), vec![Effect::WriteQuit { force: true }]);
        assert_eq!(ex("x!"), vec![Effect::WriteQuit { force: true }]);
        assert_eq!(ex("qa"), vec![Effect::QuitAll { force: false }]);
        assert_eq!(ex("qa!"), vec![Effect::QuitAll { force: true }]);
        assert_eq!(ex("wqa"), vec![Effect::Bell]);
    }

    #[test]
    fn goto_lines_with_g() {
        let (mut eng, mut doc) = engine("a\nb\nc\n");
        keys(&mut eng, &mut doc, "G");
        assert_eq!(eng.cursor().line, 2);
        keys(&mut eng, &mut doc, "gg");
        assert_eq!(eng.cursor().line, 0);
        keys(&mut eng, &mut doc, "2G");
        assert_eq!(eng.cursor().line, 1);
    }

    #[test]
    fn space_moves_like_l() {
        let (mut eng, mut doc) = engine("ab");
        keys(&mut eng, &mut doc, " ");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 1 });
    }

    #[test]
    fn dollar_reaches_end_of_line() {
        let (mut eng, mut doc) = engine("ab\ncdef\n");
        keys(&mut eng, &mut doc, "$");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 1 });
    }

    #[test]
    fn insert_typing_and_dollar_append() {
        let (mut eng, mut doc) = engine("hi");
        keys(&mut eng, &mut doc, "A");
        eng.type_text(&mut doc, "!");
        assert_eq!(doc.text().as_str(), "hi!");
        eng.press_key(&mut doc, Key::Escape);
        keys(&mut eng, &mut doc, "x");
        assert_eq!(doc.text().as_str(), "hi");
    }

    #[test]
    fn select_all_covers_whole_buffer() {
        let (mut eng, doc) = engine("a\nb\n");
        eng.select_all(&doc);
        assert_eq!(eng.mode(), Mode::Visual);
        assert_eq!(
            eng.selection(),
            Some((Cursor { line: 0, col: 0 }, Cursor { line: 2, col: 0 }))
        );
    }

    #[test]
    fn copy_without_selection_takes_cursor_line() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        eng.place_cursor(&mut doc, 1, 0);
        assert_eq!(eng.copy_text(&doc), "cd");
        // Pure: mode, cursor, and undo are untouched.
        assert_eq!(eng.mode(), Mode::Normal);
        assert_eq!(eng.cursor(), Cursor { line: 1, col: 0 });
    }

    #[test]
    fn copy_selection_is_charwise() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        keys(&mut eng, &mut doc, "vl");
        assert_eq!(eng.copy_text(&doc), "ab");
    }

    #[test]
    fn cut_line_removes_newline_and_parks_cursor() {
        let (mut eng, mut doc) = engine("a\nb\n");
        let removed = eng.cut(&mut doc);
        assert_eq!(removed, "a\n");
        assert_eq!(doc.text().as_str(), "b\n");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn cut_selection_collapses_to_normal() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        keys(&mut eng, &mut doc, "vl");
        let removed = eng.cut(&mut doc);
        assert_eq!(removed, "ab");
        assert_eq!(doc.text().as_str(), "\ncd\n");
        assert_eq!(eng.mode(), Mode::Normal);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn paste_text_inserts_at_cursor_in_normal() {
        // Insert-at-cursor (not vim `p` after-cursor) so a copied line
        // pastes back as a line instead of splitting one.
        let (mut eng, mut doc) = engine("ac");
        eng.paste_text(&mut doc, "b");
        assert_eq!(doc.text().as_str(), "bac");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 1 });
        assert_eq!(eng.mode(), Mode::Normal);
    }

    #[test]
    fn slash_search_jumps_and_n_cycles_with_wrap() {
        let (mut eng, mut doc) = engine("foo\nbar foo\n");
        keys(&mut eng, &mut doc, "/foo");
        assert_eq!(eng.mode(), Mode::Command);
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(eng.mode(), Mode::Normal);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
        assert!(eng.take_scroll_request());
        keys(&mut eng, &mut doc, "n");
        assert_eq!(eng.cursor(), Cursor { line: 1, col: 4 });
        keys(&mut eng, &mut doc, "n");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
        keys(&mut eng, &mut doc, "N");
        assert_eq!(eng.cursor(), Cursor { line: 1, col: 4 });
    }

    #[test]
    fn search_without_match_bells_and_stays_put() {
        let (mut eng, mut doc) = engine("foo\n");
        keys(&mut eng, &mut doc, "/zzz");
        let fx = eng.press_key(&mut doc, Key::Enter);
        assert_eq!(fx, vec![Effect::Bell]);
        assert_eq!(eng.mode(), Mode::Normal);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn search_without_history_bells() {
        let (mut eng, mut doc) = engine("foo\n");
        assert_eq!(keys(&mut eng, &mut doc, "n"), vec![Effect::Bell]);
        keys(&mut eng, &mut doc, "/");
        let fx = eng.press_key(&mut doc, Key::Enter);
        assert_eq!(fx, vec![Effect::Bell]);
    }

    #[test]
    fn empty_search_pattern_repeats_last() {
        let (mut eng, mut doc) = engine("foo foo\n");
        keys(&mut eng, &mut doc, "/foo");
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
        keys(&mut eng, &mut doc, "n");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 4 });
        keys(&mut eng, &mut doc, "/");
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 4 });
    }

    #[test]
    fn ide_undo_redo_round_trip() {
        let (mut eng, mut doc) = engine("");
        keys(&mut eng, &mut doc, "i");
        eng.type_text(&mut doc, "x");
        eng.press_key(&mut doc, Key::Escape);
        eng.undo(&mut doc);
        assert_eq!(doc.text().as_str(), "");
        eng.redo(&mut doc);
        assert_eq!(doc.text().as_str(), "x");
    }

    #[test]
    fn vimrc_set_applies_ignorecase_and_strips_trailing_comments() {
        assert!(parse_vimrc("set ignorecase\n").ignorecase);
        assert!(parse_vimrc("set ignorecase \" trailing comment\n").ignorecase);
        assert!(parse_vimrc("set ignorecase\t\" tab comment\n").ignorecase);
        assert!(!parse_vimrc("\" full-line comment\nset noignorecase\n").ignorecase);
        assert!(!parse_vimrc("set ignorecase\"glued quote is not a comment\n").ignorecase);
    }

    #[test]
    fn backspace_after_a_multiline_paste_deletes_across_the_newlines() {
        let (mut eng, mut doc) = engine("");
        eng.press_key(&mut doc, Key::Char('i'));
        eng.paste_text(&mut doc, "hello\nworld");
        assert_eq!(doc.text().as_str(), "hello\nworld");
        for _ in 0..11 {
            eng.press_key(&mut doc, Key::Backspace);
        }
        assert_eq!(doc.text().as_str(), "");
    }

    #[test]
    fn enter_then_typing_continues_on_the_next_line() {
        let (mut eng, mut doc) = engine("");
        eng.press_key(&mut doc, Key::Char('i'));
        eng.type_text(&mut doc, "ab");
        eng.press_key(&mut doc, Key::Enter);
        eng.type_text(&mut doc, "c");
        assert_eq!(doc.text().as_str(), "ab\nc");
        assert_eq!(eng.cursor(), Cursor { line: 1, col: 1 });
    }

    #[test]
    fn esc_from_just_past_a_newline_lands_on_the_previous_line_end() {
        let (mut eng, mut doc) = engine("");
        eng.press_key(&mut doc, Key::Char('i'));
        eng.paste_text(&mut doc, "hello\nworld");
        for _ in 0..5 {
            eng.press_key(&mut doc, Key::Backspace);
        }
        assert_eq!(doc.text().as_str(), "hello\n");
        assert_eq!(eng.cursor(), Cursor { line: 1, col: 0 });
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 4 });
    }

    #[test]
    fn paste_ending_with_a_newline_leaves_the_cursor_on_the_last_line() {
        let (mut eng, mut doc) = engine("");
        eng.paste_text(&mut doc, "hello\nworld\n");
        assert_eq!(doc.text().as_str(), "hello\nworld\n");
        assert_eq!(eng.cursor(), Cursor { line: 1, col: 4 });
    }

    #[test]
    fn capital_k_requests_hover_without_moving() {
        let (mut eng, mut doc) = engine("fn main() {}");
        let before = eng.cursor();
        let out = keys(&mut eng, &mut doc, "K");
        assert_eq!(out, vec![Effect::Hover]);
        assert_eq!(eng.cursor(), before);
        assert_eq!(doc.text().as_str(), "fn main() {}");
    }

    #[test]
    fn gd_requests_goto_and_leaves_dd_alone() {
        let (mut eng, mut doc) = engine("one\ntwo\n");
        let out = keys(&mut eng, &mut doc, "gd");
        assert_eq!(out, vec![Effect::GotoDefinition]);
        assert_eq!(doc.text().as_str(), "one\ntwo\n");
        // Bare `dd` still deletes a line, and `gg` still goes to the top.
        let out = keys(&mut eng, &mut doc, "dd");
        assert!(!out.contains(&Effect::GotoDefinition));
        assert_eq!(doc.text().as_str(), "two\n");
        let (mut eng, mut doc) = engine("one\ntwo\n");
        keys(&mut eng, &mut doc, "j");
        keys(&mut eng, &mut doc, "gg");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn g_then_other_key_bells_and_cancels() {
        let (mut eng, mut doc) = engine("one\ntwo\n");
        let out = keys(&mut eng, &mut doc, "gx");
        assert_eq!(out, vec![Effect::Bell]);
        // The cancelled prefix leaves no residue: `d` deletes again.
        let (mut eng, mut doc) = engine("one\ntwo\n");
        keys(&mut eng, &mut doc, "g");
        keys(&mut eng, &mut doc, "x");
        let out = keys(&mut eng, &mut doc, "dd");
        assert!(!out.contains(&Effect::GotoDefinition));
        assert_eq!(doc.text().as_str(), "two\n");
    }
}
