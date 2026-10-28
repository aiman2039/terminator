use super::*;
use crate::app::drain_updates;
use std::fs;
use std::sync::mpsc::{self, Receiver};
fn fixture() -> (App, egui::Context, tempfile::TempDir, Receiver<Job>) {
    let directory = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(directory.path().into()));
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    (app, ctx, directory, requests)
}
#[test]
fn checkpoint_does_not_focus_an_interrupted_session() {
    let checkpoint = Checkpoint {
        layouts: vec![],
        preferences: None,
        selected: None,
        focused: Some("missing-owner-session".into()),
    };
    assert!(checkpoint.for_state(&State::default()).focused.is_none());
}
struct ServiceCleanup(Paths);
impl Drop for ServiceCleanup {
    fn drop(&mut self) {
        if let Ok(catalog) = generations::Catalog::open(&self.0) {
            for owner in catalog.generations().unwrap_or_default() {
                let _ = rpc(&owner.paths(), Request::Shutdown);
            }
        }
    }
}

fn snapshot(paths: &Paths) -> State {
    let Response::State(state) = rpc(paths, Request::Snapshot).unwrap() else {
        panic!()
    };
    *state
}

fn create_terminal(paths: &Paths, project: &str) -> Session {
    let Response::Created(session) = rpc(
        paths,
        Request::Create {
            project: project.to_owned(),
            cwd: None,
            file: None,
            line: None,
            column: None,
            editor: false,
        },
    )
    .unwrap() else {
        panic!()
    };
    session
}

/// A GUI attached to one real terminal session of an isolated daemon.
struct AttachFixture {
    ctx: egui::Context,
    app: App,
    project: String,
    session: Session,
}

/// A bridge exit that reports failure.
fn failed_exit() -> std::process::ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(256)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(1)
    }
}

struct NewInput<'a> {
    paths: &'a Paths,
    project: &'a str,
    session: &'a Session,
    before: &'a State,
}

impl AttachFixture {
    fn new(input: NewInput<'_>) -> Self {
        let NewInput {
            paths,
            project,
            session,
            before,
        } = input;
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, paths.clone());
        app.apply_state(before.clone());
        Self {
            ctx,
            app,
            project: project.to_owned(),
            session: session.clone(),
        }
    }

    fn paint(&mut self) {
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                workspace_ui::Viewer {
                    app: &mut self.app,
                    strip: false,
                    project: Some(self.project.clone()),
                    tab: None,
                    window: None,
                    render_path: None,
                    render_tab: None,
                }
                .terminal_view(ui, &self.session);
            },
        );
        output.textures_delta.clear();
    }

    fn id(&self) -> &str {
        &self.session.id
    }

    fn bridge(&self) -> u64 {
        self.app.backends[self.id()].id()
    }

    fn exit_bridge(&mut self, bridge: u64) {
        self.app
            .pty_tx
            .send((bridge, egui_term::PtyEvent::ChildExit(failed_exit())))
            .unwrap();
        self.app
            .pty_tx
            .send((bridge, egui_term::PtyEvent::Exit))
            .unwrap();
        self.app.process_updates(&self.ctx);
    }

    fn backoff_remaining(&self) -> Duration {
        self.app.attach_budget[self.id()]
            .remaining(Instant::now())
            .unwrap()
    }

    /// An expired retry delay must reconnect the GUI bridge to the existing
    /// real PTY, even after more than three failures, without a Retry click.
    async fn reattaches_after_expired_delay(&mut self) {
        let mut retry = retry_budget::AttachmentRetry::default();
        for _ in 0..8 {
            retry.record_failure(Instant::now().checked_sub(Duration::from_mins(1)).unwrap());
        }
        self.app.attach_budget.insert(self.id().to_owned(), retry);
        self.paint();
        let id = self.session.id.clone();
        self.app
            .backends
            .get_mut(&id)
            .unwrap()
            .process_command(egui_term::BackendCommand::Write(
                b"printf 'AUTO_%s\\n' HEALED\r".to_vec(),
            ));
        let deadline = Instant::now().checked_add(Duration::from_secs(5)).unwrap();
        loop {
            self.app.process_updates(&self.ctx);
            let text: String = self
                .app
                .backends
                .get_mut(&id)
                .expect("Bridge must remain attached")
                .sync()
                .grid
                .display_iter()
                .map(|cell| cell.c)
                .collect();
            if text.contains("AUTO_HEALED") {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Reattached PTY must return shell output"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(self.app.backends.contains_key(&id));
        assert!(!self.app.attach_error.contains_key(&id));
    }

    /// A failed bridge exit after a slow attachment still preserves
    /// backoff. The daemon and its shell remain alive throughout.
    fn failed_bridge_waits_before_respawning(&mut self) -> u64 {
        let id = self.session.id.clone();
        let first_bridge = self.bridge();
        self.app.attach_started.insert(
            id.clone(),
            Instant::now().checked_sub(Duration::from_secs(5)).unwrap(),
        );
        self.exit_bridge(first_bridge);
        assert!(!self.app.backends.contains_key(&id));
        assert!(self.backoff_remaining() > Duration::from_secs(25));
        assert!(self.app.attach_error[&id].contains("Terminal attachment failed"));
        let next = self.app.next_backend;
        self.paint();
        assert_eq!(
            self.app.next_backend, next,
            "A failed bridge must wait before respawning"
        );
        first_bridge
    }

    /// Once the delay expires a new bridge spawns; late exit events from
    /// the first bridge must not remove it.
    fn respawn_ignores_stale_bridge(&mut self, first_bridge: u64) -> u64 {
        let id = self.session.id.clone();
        let mut expired = retry_budget::AttachmentRetry::default();
        expired.record_failure(Instant::now().checked_sub(Duration::from_mins(1)).unwrap());
        self.app.attach_budget.insert(id.clone(), expired);
        self.paint();
        let second_bridge = self.bridge();
        assert_ne!(first_bridge, second_bridge);
        self.exit_bridge(first_bridge);
        assert_eq!(self.bridge(), second_bridge);
        second_bridge
    }

    /// A stable attachment resets the delay and clears the old error.
    fn stable_attachment_resets_backoff(&mut self, bridge: u64) {
        let id = self.session.id.clone();
        self.app.attach_started.insert(
            id.clone(),
            Instant::now()
                .checked_sub(retry_budget::ATTACH_STABLE_WINDOW)
                .unwrap(),
        );
        self.paint();
        assert!(!self.app.attach_budget.contains_key(&id));
        assert!(!self.app.attach_error.contains_key(&id));
        self.exit_bridge(bridge);
        assert!(self.backoff_remaining() <= Duration::from_secs(1));
    }
}

struct AttachmentHealsAndBacksOffInput<'a> {
    paths: &'a Paths,
    project: &'a str,
    old: &'a Session,
    before: &'a State,
}

async fn attachment_heals_and_backs_off(input: AttachmentHealsAndBacksOffInput<'_>) {
    let AttachmentHealsAndBacksOffInput {
        paths,
        project,
        old,
        before,
    } = input;
    let mut fixture = AttachFixture::new(NewInput {
        paths,
        project,
        session: old,
        before,
    });
    fixture.reattaches_after_expired_delay().await;
    let first_bridge = fixture.failed_bridge_waits_before_respawning();
    let second_bridge = fixture.respawn_ignores_stale_bridge(first_bridge);
    fixture.stable_attachment_resets_backoff(second_bridge);
    let attached = snapshot(paths);
    assert_eq!(attached.sessions.len(), before.sessions.len());
    let same = attached
        .sessions
        .iter()
        .find(|session| session.id == old.id)
        .unwrap();
    assert_eq!(same.pid, old.pid);
    assert!(same.lifecycle.live());
}

/// Kills the fixture daemon, removes its auth file, and recovers a
/// replacement. Returns the replacement's state.
async fn kill_and_recover(paths: &Paths, before: &State, exe: &std::path::Path) -> State {
    let owner = before
        .generations
        .iter()
        .find(|health| health.owner.id == before.generation)
        .unwrap()
        .owner
        .clone();
    // Kill only this fixture's daemon; its owned PTY disappears with it.
    signals::signal_process(owner.pid.unwrap(), signals::ProcSignal::Kill).unwrap();
    let deadline = Instant::now().checked_add(Duration::from_secs(5)).unwrap();
    while !signals::process_gone(owner.pid.unwrap()) {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Reproduce the reported missing-auth path as well as daemon death.
    fs::remove_file(owner.paths().auth()).unwrap();
    let unavailable = snapshot(paths);
    assert!(!daemon_connection::active_service_available(&unavailable));
    assert!(daemon_connection::recover_unavailable(paths, &unavailable, exe).unwrap());
    let after = snapshot(paths);
    assert!(daemon_connection::active_service_available(&after));
    assert_ne!(before.generation, after.generation);
    after
}

/// A new, user-requested terminal must attach to the replacement.
fn new_terminal_attaches(paths: &Paths, project: &str) -> Session {
    let new = create_terminal(paths, project);
    let mut stream = connect(
        paths,
        Request::Attach {
            session: new.id.clone(),
            rows: 24,
            cols: 80,
        },
        None,
    )
    .unwrap();
    let response: Response = read_frame(&mut stream).unwrap();
    assert!(matches!(response, Response::Data(_)));
    drop(stream);
    new
}

struct CheckpointSavesAfterRecoveryInput<'a> {
    paths: &'a Paths,
    project: &'a str,
    focused: String,
    after: &'a State,
}

async fn checkpoint_saves_after_recovery(input: CheckpointSavesAfterRecoveryInput<'_>) {
    let CheckpointSavesAfterRecoveryInput {
        paths,
        project,
        focused,
        after,
    } = input;
    let client = async_client::Client::new(
        paths.clone(),
        async_service::NativePool::new("exit-catalog-test", 1).unwrap(),
        async_service::NativePool::new("exit-cpu-test", 1).unwrap(),
    );
    let checkpoint = Checkpoint {
        layouts: vec![(project.to_owned(), Workspace::empty())],
        preferences: Some(UiPreferences::default()),
        selected: Some(project.to_owned()),
        focused: Some(focused),
    };
    checkpoint
        .for_state(after)
        .save_async(&client)
        .await
        .unwrap();
    assert!(paths.data.join("ui-preferences.json").exists());
    let saved = snapshot(paths);
    assert_eq!(saved.selected_project.as_deref(), Some(project));
    assert!(
        saved
            .projects
            .iter()
            .find(|p| p.id == project)
            .unwrap()
            .layout
            .is_object()
    );
}

#[tokio::test]
#[ignore = "requires workspace binaries; uses isolated real daemon and PTY"]
async fn dead_service_recovers_attachment_and_exit_checkpoint() {
    let dir = tempfile::Builder::new()
        .prefix("exit-recovery-")
        .tempdir_in("/tmp")
        .unwrap();
    let paths = Paths::at(dir.path().into());
    paths.init().unwrap();
    let _cleanup = ServiceCleanup(paths.clone());
    let bins = std::env::var_os("TERMINATOR_TEST_BIN_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug"));
    let exe = bins.join(exe_name("terminator"));
    daemon_connection::ensure_running(&paths, &exe).unwrap();
    rpc(
        &paths,
        Request::AddProject {
            path: dir.path().into(),
        },
    )
    .unwrap();
    let project = snapshot(&paths).projects[0].id.clone();
    let old = create_terminal(&paths, &project);
    let before = snapshot(&paths);
    attachment_heals_and_backs_off(AttachmentHealsAndBacksOffInput {
        paths: &paths,
        project: &project,
        old: &old,
        before: &before,
    })
    .await;
    let after = kill_and_recover(&paths, &before, &exe).await;
    let historical = after
        .sessions
        .iter()
        .find(|session| session.id == old.id)
        .unwrap();
    assert_eq!(historical.lifecycle, Lifecycle::Interrupted);
    assert!(historical.pid.is_none());
    assert!(
        !after
            .sessions
            .iter()
            .any(|session| session.lifecycle.live())
    );
    let new = new_terminal_attaches(&paths, &project);
    checkpoint_saves_after_recovery(CheckpointSavesAfterRecoveryInput {
        paths: &paths,
        project: &project,
        focused: old.id,
        after: &after,
    })
    .await;
    rpc(&paths, Request::Stop { session: new.id }).unwrap();
}

#[test]
fn duplicate_restart_requests_reuse_the_pending_attempt() {
    let (mut app, ctx, _dir, requests) = fixture();
    app.begin_exit();
    app.advance_exit(&ctx);
    app.begin_exit();
    app.advance_exit(&ctx);
    assert_eq!(app.exit_attempt, 1);
    assert_eq!(requests.try_iter().count(), 1);
}
#[test]
fn failed_checkpoint_restores_interaction_and_old_ack_cannot_exit_retry() {
    let (mut app, ctx, _dir, _) = fixture();
    app.exit = Exit::Saving(Instant::now(), 1);
    app.exit_attempt = 1;
    app.update_tx
        .send(Update::ExitSaved(1, Err("disk full".into())))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(!app.exit.active());
    assert!(app.error.as_ref().unwrap().contains("disk full"));
    app.begin_exit();
    app.update_tx.send(Update::ExitSaved(1, Ok(()))).unwrap();
    drain_updates(&mut app, &ctx);
    assert!(matches!(app.exit, Exit::Waiting(_)));
    assert_eq!(app.exit_attempt, 2);
}
#[test]
fn a_pending_picker_finishes_before_the_worker_is_drained() {
    let (mut app, ctx, _dir, requests) = fixture();
    app.picker_active = true;
    app.begin_exit();
    app.advance_exit(&ctx);
    assert!(requests.try_recv().is_err());
    app.picker_active = false;
    app.advance_exit(&ctx);
    assert!(matches!(requests.try_recv(), Ok(Job::ExitDrain(..))));
}
#[test]
fn follow_up_jobs_require_another_drain() {
    let (mut app, ctx, _dir, requests) = fixture();
    app.begin_exit();
    app.advance_exit(&ctx);
    let Job::ExitDrain(id, serial) = requests.recv().unwrap() else {
        panic!()
    };
    app.send(Request::Focus {
        session: "created-asynchronously".into(),
    });
    app.update_tx.send(Update::ExitDrained(id, serial)).unwrap();
    drain_updates(&mut app, &ctx);
    assert!(matches!(requests.recv().unwrap(), Job::Control(..)));
    assert!(matches!(requests.recv().unwrap(), Job::ExitDrain(..)));
    assert!(matches!(app.exit, Exit::Draining(..)));
}
#[test]
fn cancelled_native_installation_allows_another_checkpoint() {
    let (mut app, ctx, _dir, requests) = fixture();
    app.exit_attempt = 1;
    app.exit = Exit::Ready;
    app.native_installation_cancelled();
    app.begin_exit();
    app.advance_exit(&ctx);
    assert_eq!(app.exit_attempt, 2);
    assert!(matches!(requests.recv().unwrap(), Job::ExitDrain(2, _)));
    // A late success from the cancelled installation cannot close the retry.
    app.update_tx.send(Update::ExitSaved(1, Ok(()))).unwrap();
    drain_updates(&mut app, &ctx);
    assert!(matches!(app.exit, Exit::Draining(_, 2)));
}
#[test]
fn timed_out_exit_can_be_retried() {
    let (mut app, ctx, _dir, _) = fixture();
    app.exit = Exit::Saving(
        Instant::now()
            .checked_sub(Duration::from_secs(16))
            .unwrap_or_else(Instant::now),
        1,
    );
    app.advance_exit(&ctx);
    assert!(!app.exit.active());
    app.begin_exit();
    assert!(matches!(app.exit, Exit::Waiting(_)));
}
#[test]
fn unknown_project_layout_save_does_not_abort_exit() {
    let (mut app, ctx, _dir, _) = fixture();
    app.begin_exit();
    app.update_tx
        .send(Update::LayoutSaved(
            "ghost".into(),
            "{}".into(),
            Err("Unknown project".into()),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.exit.active());
    assert!(app.error.is_none());
}
#[test]
fn exit_checkpoint_omits_unknown_projects() {
    let (mut app, _, _dir, _) = fixture();
    app.state.projects.push(Project {
        id: "project".into(),
        name: "Project".into(),
        path: _dir.path().into(),
        layout: serde_json::Value::Null,
    });
    app.layouts.insert("project".into(), Workspace::empty());
    app.layouts.insert("ghost".into(), Workspace::empty());
    app.selected = Some("ghost".into());
    let checkpoint = app.exit_checkpoint().unwrap();
    assert_eq!(
        checkpoint
            .layouts
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["project"]
    );
    assert!(checkpoint.selected.is_none());
}
#[test]
fn final_checkpoint_includes_pending_created_tab_and_preserves_unknown_layouts() {
    let (mut app, ctx, _dir, requests) = fixture();
    app.state.projects.push(Project {
        id: "project".into(),
        name: "Project".into(),
        path: _dir.path().into(),
        layout: serde_json::Value::Null,
    });
    let session = Session {
        remote: None,
        review: false,
        id: "editor".into(),
        project_id: "project".into(),
        label: "Editor".into(),
        cwd: _dir.path().into(),
        kind: SessionKind::Editor,
        file: None,
        lifecycle: Lifecycle::Running,
        created: 0,
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: "fixture".into(),
        pid: Some(42),
        truncated: false,
        cwd_confirmed: true,
    };
    app.begin_exit();
    app.advance_exit(&ctx);
    let Job::ExitDrain(id, serial) = requests.recv().unwrap() else {
        panic!()
    };
    app.update_tx
        .send(Update::WorkspaceCreated(
            session,
            "pending-tab".into(),
            vec![],
        ))
        .unwrap();
    app.update_tx.send(Update::ExitDrained(id, serial)).unwrap();
    drain_updates(&mut app, &ctx);
    let checkpoint = app.exit_checkpoint().unwrap();
    assert!(
        checkpoint
            .layouts
            .iter()
            .any(|(_, layout)| serde_json::to_string(layout).unwrap().contains("editor"))
    );
    app.layout_readonly.insert("project".into());
    assert!(app.exit_checkpoint().unwrap().layouts.is_empty());
}
#[test]
fn a_drained_socket_is_not_a_successful_persistence_acknowledgment() {
    let directory = tempfile::tempdir().unwrap();
    let paths = Paths::at(directory.path().into());
    paths.init().unwrap();
    fs::write(paths.auth(), "fixture").unwrap();
    let listener = transport::Listener::bind_ipc(&paths.socket()).unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let _: Envelope = read_frame(&mut socket).unwrap();
        write_frame(&mut socket, &Response::Error("disk full".into())).unwrap();
    });
    let checkpoint = Checkpoint {
        layouts: vec![("project".into(), Workspace::empty())],
        preferences: Some(UiPreferences::default()),
        selected: None,
        focused: None,
    };
    assert!(
        checkpoint
            .save(&paths)
            .unwrap_err()
            .to_string()
            .contains("disk full")
    );
    server.join().unwrap();
    assert!(!paths.data.join("ui-preferences.json").exists());
}
#[tokio::test]
async fn failed_heartbeat_does_not_veto_a_saved_exit() {
    let directory = tempfile::Builder::new()
        .prefix("exit-heartbeat-")
        .tempdir_in(if cfg!(unix) {
            std::path::PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        })
        .unwrap();
    let paths = Paths::at(directory.path().into());
    paths.init().unwrap();
    fs::write(paths.auth(), "fixture").unwrap();
    let listener = transport::Listener::bind_ipc(&paths.socket()).unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let request: Envelope = read_frame(&mut socket).unwrap();
        assert!(matches!(request.request, Request::SaveLayout { .. }));
        write_frame(&mut socket, &Response::Ok).unwrap();
        let (mut socket, _) = listener.accept().unwrap();
        let request: Envelope = read_frame(&mut socket).unwrap();
        assert!(matches!(
            request.request,
            Request::Heartbeat { focused: false }
        ));
        write_frame(&mut socket, &Response::Error("Owner unavailable".into())).unwrap();
    });
    let client = async_client::Client::new(
        paths.clone(),
        async_service::NativePool::new("heartbeat-catalog-test", 1).unwrap(),
        async_service::NativePool::new("heartbeat-cpu-test", 1).unwrap(),
    );
    Checkpoint {
        layouts: vec![("project".into(), Workspace::empty())],
        preferences: Some(UiPreferences::default()),
        selected: None,
        focused: None,
    }
    .save_async(&client)
    .await
    .unwrap();
    server.join().unwrap();
    assert!(paths.data.join("ui-preferences.json").exists());
}
#[test]
fn dirty_native_buffer_holds_quit_until_the_buffer_is_gone() {
    let (mut app, ctx, dir, _requests) = fixture();
    let path = dir.path().join("note.txt");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    app.accept_app_quit(&ctx);
    assert!(app.pending_app_quit);
    assert_eq!(
        app.native_close_prompt
            .as_ref()
            .map(|(prompt, _)| prompt.as_path()),
        Some(path.as_path())
    );
    assert!(!app.exit.active());
    app.native_close_prompt = None;
    app.native_docs.clear();
    app.continue_app_quit(&ctx);
    assert!(!app.pending_app_quit);
    assert!(matches!(app.exit, Exit::Waiting(_)));
}
#[test]
fn dismissing_the_unsaved_prompt_cancels_the_quit() {
    let (mut app, ctx, dir, _requests) = fixture();
    let path = dir.path().join("note.txt");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path),
    );
    app.accept_app_quit(&ctx);
    app.cancel_app_quit();
    app.native_close_prompt = None;
    app.continue_app_quit(&ctx);
    assert!(!app.pending_app_quit);
    assert!(!app.exit.active());
}
#[test]
fn quit_without_a_dirty_native_buffer_starts_the_checkpoint() {
    let (mut app, ctx, _dir, _requests) = fixture();
    app.accept_app_quit(&ctx);
    assert!(!app.pending_app_quit);
    assert!(matches!(app.exit, Exit::Waiting(_)));
}
