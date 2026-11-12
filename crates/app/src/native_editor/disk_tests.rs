use super::disk::{apply_edits, text_edits};
use super::file_io::{SaveError, TextEncoding};
use super::test_support::{app, open, pump, save};
use super::*;
use std::sync::Arc;
use std::time::SystemTime;
use terminator_native_edit::indent::IndentStyle;

#[test]
fn a_buffer_grown_past_one_mib_still_saves() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.txt");
    std::fs::write(&path, "start\n").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    let big = "x".repeat(usize::try_from(MAX_NATIVE_BYTES).unwrap() + 10);
    native.doc.as_mut().unwrap().insert(0, &big);
    save(&mut native, &app);
    assert_eq!(native.save_error, None);
    assert!(!native.dirty());
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        big.len() as u64 + 6
    );
}

#[test]
fn typing_at_the_end_of_a_crlf_line_in_a_mixed_file_saves_one_break() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.txt");
    std::fs::write(&path, "a\r\nb\nc\r\n").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    assert_eq!(native.line_ending, LineEnding::Mixed);
    let buffer = native.doc.as_mut().unwrap();
    let end = buffer.char_at_line_col(0, usize::MAX);
    buffer.insert(end, "!");
    save(&mut native, &app);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "a!\r\nb\nc\r\n");
}

#[test]
fn a_clean_buffer_follows_the_disk_as_one_undoable_edit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("follow.txt");
    std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    std::fs::write(&path, "one\n2\nthree\nfour\n").unwrap();
    native.request_reload(false);
    pump(&mut native, &app, |native| {
        !native.disk.reload_wanted && !native.loading
    });
    let buffer = native.doc.as_mut().unwrap();
    assert_eq!(buffer.text(), "one\n2\nthree\nfour\n");
    assert!(!native.dirty(), "the reloaded text is the saved text");
    let buffer = native.doc.as_mut().unwrap();
    assert!(buffer.undo(), "undo history survives the reload");
    assert_eq!(buffer.text(), "one\ntwo\nthree\n");
}

#[test]
fn a_missing_file_is_retried_until_it_appears() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("later.txt");
    let app = app(dir.path());
    let mut native = NativeDoc::new(path.clone(), false);
    pump(&mut native, &app, |native| native.load_error.is_some());
    assert!(native.disk.retry.is_some(), "a retry is scheduled");
    std::fs::write(&path, "here\n").unwrap();
    native.disk.retry = native.disk.retry.map(|(_, n)| (Instant::now(), n));
    pump(&mut native, &app, |native| native.doc.is_some());
    assert_eq!(native.load_error, None);
    assert_eq!(native.doc.as_ref().unwrap().text(), "here\n");
}

#[test]
fn edit_of_a_new_file_opens_empty_and_the_first_save_creates_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("new.txt");
    let app = app(dir.path());
    let mut native = NativeDoc::new(path.clone(), false);
    native.new_file = true;
    pump(&mut native, &app, |native| native.doc.is_some());
    assert_eq!(native.load_error, None);
    assert_eq!(native.doc.as_ref().unwrap().text(), "");
    native.doc.as_mut().unwrap().insert(0, "made\n");
    save(&mut native, &app);
    assert_eq!(native.save_error, None);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "made\n");
}

#[test]
fn a_reload_while_the_file_is_gone_keeps_the_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gone.txt");
    std::fs::write(&path, "kept\n").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    std::fs::remove_file(&path).unwrap();
    native.request_reload(false);
    pump(&mut native, &app, |native| native.disk.retry.is_some());
    assert_eq!(native.doc.as_ref().unwrap().text(), "kept\n");
    assert_eq!(
        native.load_error, None,
        "no 'Could not open' over the buffer"
    );
    // An atomic save by another tool puts it back: the retry picks it up.
    std::fs::write(&path, "back\n").unwrap();
    native.disk.retry = native.disk.retry.map(|(_, n)| (Instant::now(), n));
    pump(&mut native, &app, |native| {
        !native.disk.reload_wanted && !native.loading
    });
    assert_eq!(native.doc.as_ref().unwrap().text(), "back\n");
}

#[test]
fn save_refuses_changed_bytes_even_when_the_mtime_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("same-mtime.txt");
    std::fs::write(&path, "mine\n").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::write(&path, "theirs\n").unwrap();
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(mtime).unwrap();
    native.doc.as_mut().unwrap().insert(0, "edit ");
    save(&mut native, &app);
    assert!(
        native
            .save_error
            .as_ref()
            .is_some_and(|error| *error == SaveError::ChangedOnDisk)
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs\n");
}

#[test]
fn save_accepts_a_touched_file_whose_bytes_are_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("touched.txt");
    std::fs::write(&path, "same\n").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    let file = std::fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_hours(24))
        .unwrap();
    native.doc.as_mut().unwrap().insert(0, "new ");
    save(&mut native, &app);
    assert_eq!(native.save_error, None);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new same\n");
}

#[cfg(unix)]
#[test]
fn saving_a_hard_linked_file_keeps_both_names_on_one_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    let other = dir.path().join("b.txt");
    std::fs::write(&path, "old\n").unwrap();
    std::fs::hard_link(&path, &other).unwrap();
    super::file_io::NativeFileIo::write(&path, b"new\n").unwrap();
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "new\n");
}

#[cfg(unix)]
#[test]
fn an_in_place_save_shrinks_the_file_and_keeps_no_safety_copy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    let other = dir.path().join("b.txt");
    std::fs::write(&path, "a much longer old text\n").unwrap();
    std::fs::hard_link(&path, &other).unwrap();
    super::file_io::NativeFileIo::write(&path, b"short\n").unwrap();
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "short\n");
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

#[test]
fn a_disk_change_is_not_a_conflict_while_a_save_runs() {
    let mut native = NativeDoc::loaded(PathBuf::from("/tmp/saving.txt"), "x");
    let later = Some(SystemTime::now());
    assert!(native.disk_conflict(later));
    native.saving = true;
    assert!(!native.disk_conflict(later));
    native.saving = false;
    native.disk.keep_mine = Some(super::disk::Stamp(later));
    assert!(
        !native.disk_conflict(later),
        "Keep mine answered this change"
    );
}

#[test]
fn non_utf8_files_round_trip_in_their_own_encoding() {
    let latin1 = b"caf\xe9 cr\xe8me\n".to_vec();
    let (text, encoding) = TextEncoding::decode(latin1.clone()).unwrap();
    assert_eq!(text, "café crème\n");
    assert_ne!(encoding, TextEncoding::default());
    assert_eq!(encoding.encode(&text).unwrap(), latin1);
    // A BOM is kept, and UTF-16 saves back as UTF-16.
    let bom = b"\xEF\xBB\xBFhi\n".to_vec();
    let (text, encoding) = TextEncoding::decode(bom.clone()).unwrap();
    assert_eq!((text.as_str(), encoding.bom), ("hi\n", true));
    assert_eq!(encoding.encode(&text).unwrap(), bom);
    let utf16 = b"\xFF\xFEh\x00i\x00".to_vec();
    let (text, encoding) = TextEncoding::decode(utf16.clone()).unwrap();
    assert_eq!(text, "hi");
    assert_eq!(encoding.encode(&text).unwrap(), utf16);
    assert!(
        TextEncoding::decode(b"\x00\x01\xff".to_vec()).is_err(),
        "binary"
    );
    // Characters the encoding cannot hold refuse the save.
    assert!(encoding_rs_latin1().encode("€ and 中").is_err());
}

fn encoding_rs_latin1() -> TextEncoding {
    TextEncoding::decode(b"\xe9\n".to_vec()).unwrap().1
}

#[test]
fn reload_edits_turn_the_old_text_into_the_new() {
    let old = "a\nb\nc\nd\n";
    let new = "a\nB\nc\nd\ne\n";
    let mut doc = Doc::new(old);
    apply_edits(&mut doc, &text_edits(old, new));
    assert_eq!(doc.text(), new);
    assert!(doc.undo());
    assert_eq!(doc.text(), old);
}

#[test]
fn large_files_open_in_large_file_mode_until_enabled() {
    let big = "x\n".repeat(LARGE_FILE_BYTES);
    let mut native = NativeDoc::loaded(PathBuf::from("/tmp/large.txt"), &big);
    assert!(native.large_file());
    native.extras.large_override = true;
    assert!(!native.large_file());
    let small = NativeDoc::loaded(PathBuf::from("/tmp/small.txt"), "x\n");
    assert!(!small.large_file());
}

#[test]
fn squiggles_rebuild_only_when_the_text_or_problems_change() {
    let mut native = NativeDoc::loaded(PathBuf::from("/tmp/squiggles.rs"), "let a = 1;\n");
    let at = |line, character| native_lsp::Position { line, character };
    let problems = vec![native_lsp::Diagnostic {
        range: native_lsp::Range {
            start: at(0, 4),
            end: at(0, 5),
        },
        severity: Some(native_lsp::Severity::Error),
        message: "unused".into(),
    }];
    let buffer = native.doc.take().unwrap();
    let (first, _) = native.extras.squiggles(&buffer, (&problems, 1));
    let (again, _) = native.extras.squiggles(&buffer, (&problems, 1));
    assert!(Arc::ptr_eq(&first, &again), "an idle frame reuses them");
    let (cleared, _) = native.extras.squiggles(&buffer, (&[], 2));
    assert!(cleared.is_empty());
}

#[test]
fn format_options_follow_the_file_or_the_vimrc() {
    let tabs = NativeDoc::loaded(PathBuf::from("/tmp/t.c"), "int f() {\n\treturn 1;\n}\n");
    assert!(!tabs.format_options().insert_spaces);
    let two = NativeDoc::loaded(PathBuf::from("/tmp/t.js"), "a {\n  b {\n    c\n  }\n}\n");
    assert_eq!(two.format_options().tab_size, 2);
}

#[test]
fn loading_follows_editorconfig_and_saving_applies_its_rules() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".editorconfig"),
        "root = true\n[*.txt]\nindent_style = tab\ntab_width = 8\n\
         insert_final_newline = true\ntrim_trailing_whitespace = true\n",
    )
    .unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "one  \n    two\nthree ").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    // `.editorconfig` wins over the four spaces the text uses.
    let style = native.doc.as_ref().unwrap().indent_style();
    assert_eq!(style, IndentStyle::tabs(8));
    assert_eq!(
        native.format_options(),
        native_lsp::FormatOptions {
            tab_size: 8,
            insert_spaces: false
        }
    );
    // The caret's line keeps its trailing spaces; the last line gets a
    // newline.
    save(&mut native, &app);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "one  \n    two\nthree\n"
    );
    assert!(!native.dirty());
    let buffer = native.doc.as_mut().unwrap();
    assert!(buffer.undo(), "the cleanup is one undo step");
    assert_eq!(buffer.text(), "one  \n    two\nthree ");
}

#[test]
fn a_save_after_undo_keeps_redo_and_defers_the_save_rules() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".editorconfig"),
        "root = true\n[*]\ntrim_trailing_whitespace = true\n",
    )
    .unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "one\ntwo  \n").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    native.doc.as_mut().unwrap().insert(3, "!");
    assert!(native.doc.as_mut().unwrap().undo());
    // An autosave (or Save) right after Undo: the redo step survives.
    save(&mut native, &app);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo  \n");
    let buffer = native.doc.as_mut().unwrap();
    assert!(buffer.redo(), "the undone edit can still be redone");
    assert_eq!(buffer.text(), "one!\ntwo  \n");
    // With nothing to redo, the next save trims again.
    save(&mut native, &app);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one!\ntwo\n");
}

#[test]
fn a_save_during_a_vim_insert_keeps_the_cleanup_out_of_the_typed_change() {
    use terminator_native_edit::vim::Key as VimKey;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".editorconfig"),
        "root = true\n[*]\ntrim_trailing_whitespace = true\n",
    )
    .unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "one  \ntwo\n").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    let buffer = native.doc.as_mut().unwrap();
    native.engine.press_key(buffer, VimKey::Char('j'));
    native.engine.press_key(buffer, VimKey::Char('o'));
    native.engine.type_text(buffer, "foo");
    // Cmd+S (or an autosave) while Insert mode is still on.
    save(&mut native, &app);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "one  \ntwo\nfoo\n",
        "the cleanup waits for the change to end"
    );
    let buffer = native.doc.as_mut().unwrap();
    native.engine.press_key(buffer, VimKey::Escape);
    save(&mut native, &app);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\nfoo\n");
    // Undo takes the cleanup back first, then the typing alone.
    let buffer = native.doc.as_mut().unwrap();
    assert!(buffer.undo());
    assert_eq!(buffer.text(), "one  \ntwo\nfoo\n");
    assert!(buffer.undo());
    assert_eq!(buffer.text(), "one  \ntwo\n");
}

#[test]
fn a_view_that_stopped_drawing_keeps_no_line_from_the_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".editorconfig"),
        "root = true\n[*]\ntrim_trailing_whitespace = true\n",
    )
    .unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "one  \ntwo\nthree\n").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    // A split view left its caret on line 1; its pane closed since.
    native.enter_view("closed");
    native.enter_view("open");
    let buffer = native.doc.as_mut().unwrap();
    let _ = native.engine.place_cursor(buffer, 2, 0);
    native.drawn.insert("closed".into(), 1);
    native.drawn.insert("open".into(), 9);
    save(&mut native, &app);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\nthree\n");
}

#[test]
fn a_file_without_line_breaks_takes_the_editorconfig_ones() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".editorconfig"),
        "root = true\n[*]\nend_of_line = crlf\n",
    )
    .unwrap();
    let new = dir.path().join("new.txt");
    std::fs::write(&new, "").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &new);
    assert_eq!(native.line_ending, LineEnding::Crlf);
    assert!(!native.dirty());
    native.doc.as_mut().unwrap().insert(0, "a\nb");
    save(&mut native, &app);
    assert_eq!(std::fs::read(&new).unwrap(), b"a\r\nb");
    // A file with breaks keeps its own.
    let old = dir.path().join("old.txt");
    std::fs::write(&old, "a\nb\n").unwrap();
    assert_eq!(open(&app, &old).line_ending, LineEnding::Lf);
}

#[test]
fn remote_snapshots_detect_indentation_without_editorconfig() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let path = crate::remote_ui::snapshot_root(&app.paths.data, "fixture").join("x.c");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path.with_file_name(".editorconfig"),
        "root = true\n[*]\nindent_style = tab\n",
    )
    .unwrap();
    std::fs::write(&path, "a {\n  b;\n}\n").unwrap();
    app.native_doc(&path, true);
    let mut native = app.native_docs.remove(&path).unwrap();
    assert!(!native.read_editorconfig);
    pump(&mut native, &app, |native| native.doc.is_some());
    let style = native.doc.as_ref().unwrap().indent_style();
    assert_eq!(style, IndentStyle::spaces(2));
}

#[test]
fn two_save_and_close_requests_both_wait_for_their_saves() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let (first, second) = (dir.path().join("1.txt"), dir.path().join("2.txt"));
    for path in [&first, &second] {
        app.native_docs
            .insert(path.clone(), NativeDoc::dirty_for_test(path.clone()));
        app.apply_native_close_choice(
            NativeCloseRequest::new(path.clone(), None, CloseOwner::Pane),
            NativeCloseChoice::SaveClose,
        );
    }
    for path in [&first, &second] {
        assert!(app.native_close_after_save.iter().any(|r| r.path == *path));
    }
}

#[test]
fn a_finished_save_is_announced_to_language_servers_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    assert_eq!(native.take_lsp_saved(), None, "a load is not a save");
    native.doc.as_mut().unwrap().insert(0, "// note\n");
    save(&mut native, &app);
    let revision = native.doc.as_ref().unwrap().revision();
    assert_eq!(
        native.take_lsp_saved(),
        Some((revision, "// note\nfn main() {}\n".to_owned()))
    );
    assert_eq!(native.take_lsp_saved(), None, "announced once");
    // A refused write is not announced.
    std::fs::write(&path, "theirs\n").unwrap();
    native.doc.as_mut().unwrap().insert(0, "x");
    save(&mut native, &app);
    assert!(native.save_error.is_some());
    assert_eq!(native.take_lsp_saved(), None);
}

#[test]
fn a_rewrite_on_disk_is_announced_once_the_reload_lands() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn foo() {}\n").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    // What an applied rename does to a clean open buffer.
    std::fs::write(&path, "fn bar() {}\n").unwrap();
    native.request_reload(false);
    native.lsp_saved = true;
    assert_eq!(
        native.take_lsp_saved(),
        None,
        "the buffer is not reloaded yet"
    );
    pump(&mut native, &app, |native| {
        !native.disk.reload_wanted && !native.loading
    });
    let announced = native.take_lsp_saved().map(|(_, text)| text);
    assert_eq!(announced.as_deref(), Some("fn bar() {}\n"));
}

#[test]
fn a_landed_save_sends_did_save_to_the_language_server() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let mut app = app(dir.path());
    let mut native = open(&app, &path);
    native.doc.as_mut().unwrap().insert(0, "// saved\n");
    save(&mut native, &app);
    app.native_docs.insert(path.clone(), native);
    let sync = serde_json::json!({"textDocumentSync": {"change": 2, "save": {}}});
    let mut outbox = app.lsp.open_on_fake_server(&path, sync);
    app.native_lsp_saved(&path);
    app.native_lsp_saved(&path);
    let mut decoder = native_lsp::FrameDecoder::default();
    let mut methods = Vec::new();
    while let Ok(framed) = outbox.try_recv() {
        for body in decoder.feed(&framed) {
            let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
            methods.push(value["method"].as_str().unwrap().to_owned());
        }
    }
    let saves = methods
        .iter()
        .filter(|method| *method == "textDocument/didSave")
        .count();
    assert_eq!(saves, 1, "one write, one didSave: {methods:?}");
    assert_eq!(
        methods.last().map(String::as_str),
        Some("textDocument/didSave")
    );
}

#[test]
fn undo_of_a_crlf_conversion_saves_one_cr_per_break() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed.txt");
    std::fs::write(&path, "a\r\nb\nc\r\n").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    let buffer = native.doc.as_mut().unwrap();
    let text = buffer.text();
    apply_edits(buffer, &text_edits(&text, "a\nb\nc\n"));
    native.line_ending = LineEnding::Crlf;
    assert!(native.doc.as_mut().unwrap().undo());
    save(&mut native, &app);
    assert_eq!(std::fs::read(&path).unwrap(), b"a\r\nb\r\nc\r\n");
}

#[test]
fn an_undo_during_a_save_keeps_the_buffer_dirty_when_it_lands() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memo.txt");
    std::fs::write(&path, "0\n").unwrap();
    let app = app(dir.path());
    let mut native = open(&app, &path);
    apply_edits(native.doc.as_mut().unwrap(), &text_edits("0\n", "1\n"));
    native.start_save(&app.services);
    assert!(native.doc.as_mut().unwrap().undo());
    assert!(!native.dirty(), "the buffer matches the old saved text");
    pump(&mut native, &app, |native| !native.saving);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "1\n");
    assert!(native.dirty(), "the buffer differs from the new disk text");
}

#[test]
fn keep_mine_makes_save_all_overwrite_the_newer_disk_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keep.txt");
    std::fs::write(&path, "old\n").unwrap();
    let mut app = app(dir.path());
    let mut native = open(&app, &path);
    std::fs::write(&path, "theirs\n").unwrap();
    native.doc.as_mut().unwrap().insert(0, "mine ");
    // What the conflict bar's Keep mine sets.
    native.force_save = true;
    app.native_docs.insert(path.clone(), native);
    app.native_save_all();
    let mut native = app.native_docs.remove(&path).unwrap();
    pump(&mut native, &app, |native| !native.saving);
    assert_eq!(native.save_error, None);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine old\n");
}

#[cfg(unix)]
#[test]
fn opening_a_fifo_fails_at_once_and_does_not_block_the_file_pool() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipe");
    let made = std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .unwrap();
    assert!(made.success());
    let Err(error) = super::file_io::NativeFileIo::read(&path, MAX_NATIVE_BYTES) else {
        panic!("a FIFO is not a regular file");
    };
    assert!(error.to_string().contains("Not a regular file"), "{error}");
}

#[test]
fn only_a_disk_conflict_offers_save_anyway_whatever_the_wording() {
    use super::file_io::ChangedOnDisk;
    let conflict = anyhow::Error::from(ChangedOnDisk).context("Save notes.txt");
    assert_eq!(SaveError::of(&conflict), SaveError::ChangedOnDisk);
    let other = anyhow::anyhow!("Replace file: Changed on disk folder is read-only");
    assert!(matches!(SaveError::of(&other), SaveError::Failed(_)));
}
