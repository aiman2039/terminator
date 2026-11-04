use super::disk::{apply_edits, text_edits};
use super::file_io::{NativeFileIo, TextEncoding};
use super::test_support::{app, open, pump, save};
use super::*;
use std::sync::Arc;
use std::time::SystemTime;

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
            .as_deref()
            .is_some_and(|error| error.contains("Changed on disk"))
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
    NativeFileIo::write(&path, b"new\n").unwrap();
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "new\n");
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
    let mut tabs = NativeDoc::loaded(PathBuf::from("/tmp/t.c"), "int f() {\n\treturn 1;\n}\n");
    assert!(!tabs.format_options().insert_spaces);
    let mut two = NativeDoc::loaded(PathBuf::from("/tmp/t.js"), "a {\n  b {\n    c\n  }\n}\n");
    assert_eq!(two.format_options().tab_size, 2);
}

#[test]
fn two_save_and_close_requests_both_wait_for_their_saves() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let (first, second) = (dir.path().join("1.txt"), dir.path().join("2.txt"));
    for path in [&first, &second] {
        app.native_docs
            .insert(path.clone(), NativeDoc::dirty_for_test(path.clone()));
        app.apply_native_close_choice(path, NativeCloseChoice::SaveClose, None);
    }
    assert!(app.native_close_after_save.contains_key(&first));
    assert!(app.native_close_after_save.contains_key(&second));
}
