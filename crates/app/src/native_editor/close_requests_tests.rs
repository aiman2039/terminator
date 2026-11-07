use super::test_support::app;
use super::*;

fn dirty(app: &mut App, dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    app.native_docs
        .insert(path.clone(), NativeDoc::dirty_for_test(path.clone()));
    path
}

fn request(path: &Path, owner: CloseOwner) -> NativeCloseRequest {
    NativeCloseRequest::new(path.to_owned(), None, owner)
}

/// A save that ended with the buffer still dirty, as after "Changed on disk".
fn fail_save(app: &mut App, path: &Path) {
    let doc = app.native_docs.get_mut(path).unwrap();
    doc.saving = false;
    doc.save_rx = None;
    doc.save_error = Some("Changed on disk since loading".into());
}

#[test]
fn failed_save_and_quit_stops_the_quit_instead_of_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let path = dirty(&mut app, dir.path(), "note.txt");
    app.pending_app_quit = true;
    app.native_close_after_save
        .push(request(&path, CloseOwner::Quit));
    fail_save(&mut app, &path);
    app.settle_native_closes();
    assert!(app.native_close_after_save.is_empty());
    assert!(!app.pending_app_quit, "the quit ends instead of stalling");
    assert!(app.info.as_ref().unwrap().contains("note.txt"));
    assert!(app.native_dirty(&path), "the unsaved text is kept");
}

#[test]
fn failed_save_and_close_does_not_close_the_tab_later() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let path = dirty(&mut app, dir.path(), "note.txt");
    app.native_close_after_save
        .push(request(&path, CloseOwner::Pane));
    fail_save(&mut app, &path);
    app.settle_native_closes();
    assert!(app.native_close_after_save.is_empty());
    // A later save (or an undo back to the saved text) is not a close.
    app.native_docs.get_mut(&path).unwrap().mark_saved_now();
    assert!(!app.native_close_after_save_settled(&path));
    assert!(app.pending_native_close.is_empty());
}

#[test]
fn refused_save_start_does_not_hold_the_quit() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let path = dirty(&mut app, dir.path(), "note.txt");
    app.pending_app_quit = true;
    // A reload is running: the save cannot start.
    app.native_docs.get_mut(&path).unwrap().loading = true;
    app.answer_native_close(
        request(&path, CloseOwner::Quit),
        Some(NativeCloseChoice::SaveClose),
    );
    app.settle_native_closes();
    assert!(app.pending_app_quit, "waits while the file loads");
    app.native_docs.get_mut(&path).unwrap().loading = false;
    app.settle_native_closes();
    assert!(app.native_close_after_save.is_empty());
    assert!(!app.pending_app_quit);
}

#[test]
fn cancelling_quit_keeps_pane_save_and_close_requests() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let (pane, quit) = (dir.path().join("pane.txt"), dir.path().join("quit.txt"));
    app.pending_app_quit = true;
    app.native_close_after_save.extend([
        request(&pane, CloseOwner::Pane),
        request(&quit, CloseOwner::Quit),
    ]);
    app.ask_native_close(request(&pane, CloseOwner::Pane));
    app.ask_native_close(request(&quit, CloseOwner::Quit));
    app.cancel_app_quit();
    let owners = |requests: &[NativeCloseRequest]| {
        requests
            .iter()
            .map(|r| (r.path.clone(), r.owner.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        owners(&app.native_close_after_save),
        [(pane.clone(), CloseOwner::Pane)]
    );
    assert_eq!(
        owners(&app.native_close_prompts),
        [(pane, CloseOwner::Pane)]
    );
}

#[test]
fn saved_quit_request_closes_the_file_and_lets_the_quit_go_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(dir.path());
    let path = dirty(&mut app, dir.path(), "note.txt");
    app.pending_app_quit = true;
    app.native_close_after_save
        .push(request(&path, CloseOwner::Quit));
    app.native_docs.get_mut(&path).unwrap().mark_saved_now();
    app.settle_native_closes();
    assert!(app.native_close_after_save.is_empty());
    assert!(app.pending_app_quit, "the quit goes on");
    assert!(!app.native_close_waits_for(&CloseOwner::Quit));
}
