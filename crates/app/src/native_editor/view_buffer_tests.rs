use super::file_io::NativeFileIo;
use super::*;
use std::time::SystemTime;
use terminator_native_edit::vim::Key as VimKey;

#[test]
fn crlf_files_edit_as_lf_and_save_as_crlf() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("win.txt");
    std::fs::write(&path, "a\r\nb\r\n").unwrap();
    let ctx = egui::Context::default();
    let app = App::with_context(&ctx, Paths::at(dir.path().join("state")));
    let mut native = NativeDoc::new(path.clone(), false);
    native.ensure_loading(&app.services);
    let start = std::time::Instant::now();
    while native.doc.is_none() {
        assert!(start.elapsed() < Duration::from_secs(10), "load");
        native.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(native.crlf);
    assert_eq!(native.doc.as_ref().unwrap().text(), "a\nb\n");
    assert!(!native.dirty());
    if let Some(buffer) = native.doc.as_mut() {
        let end = buffer.len_chars();
        buffer.insert(end, "c\n");
    }
    native.start_save(&app.services);
    while native.saving {
        assert!(start.elapsed() < Duration::from_secs(10), "save");
        native.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(native.save_error, None);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\r\nb\r\nc\r\n");
    assert!(!native.dirty());
}

#[test]
fn split_views_keep_their_own_caret_and_the_focused_one_rests_live() {
    let mut native = NativeDoc::new(PathBuf::from("/tmp/views.txt"), false);
    let mut buffer = Doc::new("zero\none\ntwo\n");
    native.enter_view("left");
    let _ = native.engine.place_cursor(&mut buffer, 2, 0);
    // A new view starts at the same caret, then moves on its own.
    native.enter_view("right");
    assert_eq!(native.engine.cursor().line, 2);
    let _ = native.engine.place_cursor(&mut buffer, 0, 1);
    native.focused_view = Some("left".into());
    native.restore_focused_view();
    assert_eq!(native.engine.cursor().line, 2);
    native.enter_view("right");
    assert_eq!(
        native.engine.cursor(),
        terminator_native_edit::vim::Cursor { line: 0, col: 1 }
    );
}

#[test]
fn undoing_back_to_the_saved_text_is_clean() {
    let mut native = NativeDoc::new(PathBuf::from("/tmp/clean.txt"), true);
    let mut buffer = Doc::new("saved");
    native.saved_hash = NativeFileIo::text_hash("saved");
    native.saved_revision = buffer.revision();
    native.engine.press_key(&mut buffer, VimKey::Char('i'));
    native.engine.type_text(&mut buffer, "x");
    native.engine.press_key(&mut buffer, VimKey::Escape);
    native.doc = Some(buffer);
    assert!(native.dirty());
    if let Some(buffer) = native.doc.as_mut() {
        native.engine.undo(buffer);
    }
    assert!(!native.dirty());
}

#[cfg(unix)]
#[test]
fn atomic_save_keeps_permissions_and_symlinks() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("run.sh");
    std::fs::write(&target, "old").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    let link = dir.path().join("link.sh");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    NativeFileIo::write_atomically(&link, b"new").unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o755
    );
    // No temporary file is left behind.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[test]
fn remote_markdown_loads_up_to_two_mib_and_local_limit_stays_one_mib() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let path = crate::remote_ui::snapshot_root(&app.paths.data, "fixture").join("large.md");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes = vec![b'a'; terminator_core::remote::MAX_FILE_BYTES];
    std::fs::write(&path, &bytes).unwrap();
    let limit = app.native_doc(&path, false).read_limit;
    assert_eq!(
        NativeFileIo::read(&path, limit).unwrap().text.len(),
        bytes.len()
    );
    let local = dir.path().join("local.md");
    std::fs::write(&local, &bytes).unwrap();
    assert!(NativeFileIo::read(&local, app.native_doc(&local, false).read_limit).is_err());
    std::fs::write(&path, vec![b'a'; bytes.len() + 1]).unwrap();
    assert!(NativeFileIo::read(&path, limit).is_err());
}

#[test]
fn closing_a_clean_editor_copy_keeps_the_other_views_undo_history() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let path = dir.path().join("shared.txt");
    let pane = Tab::NativeEditor { path: path.clone() };
    let mut native = NativeDoc::new(path.clone(), true);
    let mut buffer = Doc::new("original");
    native.engine.press_key(&mut buffer, VimKey::Char('i'));
    native.engine.type_text(&mut buffer, "saved ");
    native.engine.press_key(&mut buffer, VimKey::Escape);
    native.saved_revision = buffer.revision();
    native.doc = Some(buffer);
    app.native_docs.insert(path.clone(), native);
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(egui_dock::DockState::new(vec![pane.clone(), pane.clone()])),
    );
    let at = app.layouts["a"].find_tab(&pane).unwrap();
    {
        let mut viewer = Viewer {
            app: &mut app,
            strip: false,
            project: Some("a".into()),
            tab: None,
            window: None,
            render_path: None,
            render_tab: None,
        };
        assert_eq!(
            viewer.on_close(&mut pane.clone(), at.node_path()),
            OnCloseResponse::Close
        );
    }
    app.layouts.get_mut("a").unwrap().remove_tab(at);
    app.prune_native_docs();
    let native = app
        .native_docs
        .get_mut(&path)
        .expect("shared buffer remains");
    let buffer = native.doc.as_mut().unwrap();
    native.engine.undo(buffer);
    assert_eq!(buffer.text(), "original");
}

#[test]
fn an_edit_during_a_save_stays_unsaved_when_the_write_lands() {
    let mut native = NativeDoc::new(PathBuf::from("/tmp/save-race.txt"), true);
    native.doc = Some(Doc::new("one"));
    native.saved_revision = native.doc.as_ref().map_or(0, Doc::revision);
    assert!(!native.dirty(), "a freshly loaded buffer is clean");
    // The write snapshots the revision it is sending to disk.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    native.save_rx = Some(rx);
    native.saving = true;
    native.save_revision = native.doc.as_ref().map_or(0, Doc::revision);
    // Typing while the write runs.
    native.doc.as_mut().expect("buffer").insert(3, " more");
    assert!(native.dirty());
    tx.send(Ok(Some(SystemTime::now())))
        .expect("deliver the save result");
    native.poll();
    assert!(!native.saving);
    assert!(
        native.dirty(),
        "the buffer changed after the snapshot: it is still unsaved"
    );
    assert_eq!(native.doc.as_ref().expect("buffer").text(), "one more");

    // A save that takes the newer text clears the flag again.
    native.save_revision = native.doc.as_ref().map_or(0, Doc::revision);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    native.save_rx = Some(rx);
    native.saving = true;
    tx.send(Ok(Some(SystemTime::now())))
        .expect("deliver the save result");
    native.poll();
    assert!(!native.dirty(), "the written revision is clean again");
}

#[test]
fn disk_mtime_reads_are_throttled_and_off_thread() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("throttled.txt");
    std::fs::write(&path, "one").unwrap();
    let mut native = NativeDoc::new(path, true);
    native.loaded_mtime = Some(SystemTime::UNIX_EPOCH);
    // No reading yet: a probe is due and the loaded time stands in.
    assert!(native.mtime_probe_due());
    // A probe in flight is not started twice.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    native.mtime_rx = Some(rx);
    assert!(!native.mtime_probe_due());
    let fresh = SystemTime::UNIX_EPOCH + Duration::from_secs(5);
    tx.send(Some(fresh)).unwrap();
    native.poll_mtime();
    assert!(native.mtime_rx.is_none());
    assert_eq!(native.mtime_check.map(|(_, m)| m), Some(Some(fresh)));
    // Inside the TTL the recorded reading stands in for a fresh stat.
    assert!(!native.mtime_probe_due());
    native.mtime_check = Some((
        Instant::now()
            .checked_sub(MTIME_TTL + Duration::from_millis(1))
            .expect("instant inside the clock range"),
        Some(fresh),
    ));
    assert!(native.mtime_probe_due(), "an expired reading is re-checked");
    // A save or load drops a stat that may predate the write.
    let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
    native.mtime_rx = Some(rx);
    native.reset_mtime();
    assert!(native.mtime_rx.is_none() && native.mtime_check.is_none());
}
