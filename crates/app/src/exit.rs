//! A checkpoint is acknowledged only after all final writes succeed.
use super::*;

#[derive(Default)]
pub(super) enum Exit {
    #[default]
    Idle,
    Waiting(Instant),
    Draining(Instant, u64),
    Saving(Instant, u64),
    Ready,
}
impl Exit {
    pub fn active(&self) -> bool {
        !matches!(self, Self::Idle)
    }
}

pub(super) struct Checkpoint {
    layouts: Vec<(String, Workspace)>,
    preferences: Option<UiPreferences>,
    selected: Option<String>,
    focused: Option<String>,
}
impl Checkpoint {
    pub(crate) fn for_state(mut self, state: &State) -> Self {
        // Focus is transient owner state, not layout persistence. A dead or
        // retired owner cannot acknowledge it and must not block closing.
        self.focused = self.focused.filter(|id| {
            state.sessions.iter().any(|session| {
                session.id == *id
                    && session.lifecycle.live()
                    && state.generations.iter().all(|health| {
                        health.owner.id != session.generation
                            || (health.error.is_none()
                                && health.owner.status != generations::Status::Retired)
                    })
            })
        });
        self
    }
    #[cfg(test)]
    pub fn save(self, paths: &Paths) -> Result<()> {
        let mut requests = self
            .layouts
            .into_iter()
            .map(|(project, layout)| {
                Ok(Request::SaveLayout {
                    project,
                    layout: sanitize_layout(serde_json::to_value(layout)?),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if let Some(project) = self.selected {
            requests.push(Request::SelectProject { project });
        }
        for request in requests {
            anyhow::ensure!(
                matches!(rpc(paths, request)?, Response::Ok),
                "Exit persistence was not acknowledged"
            );
        }
        if let Some(preferences) = self.preferences {
            preferences.save(&paths.data)?;
        }
        if let Some(session) = self.focused {
            let _ = rpc(paths, Request::Focus { session });
        }
        // Heartbeat is transient visibility, not a persistence acknowledgment.
        // An unavailable draining owner must not veto an already saved exit.
        let _ = rpc(paths, Request::Heartbeat { focused: false });
        Ok(())
    }
    pub async fn save_async(self, client: &async_client::Client) -> Result<()> {
        let mut requests = client
            .catalog
            .run(&async_service::CancellationToken::new(), move || {
                self.layouts
                    .into_iter()
                    .map(|(project, layout)| {
                        Ok(Request::SaveLayout {
                            project,
                            layout: sanitize_layout(serde_json::to_value(layout)?),
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .await?;
        if let Some(project) = self.selected {
            requests.push(Request::SelectProject { project });
        }
        for request in requests {
            anyhow::ensure!(
                matches!(client.rpc(request).await?, Response::Ok),
                "Exit persistence was not acknowledged"
            );
        }
        if let Some(preferences) = self.preferences {
            let data = client.paths.data.clone();
            client
                .catalog
                .run(&async_service::CancellationToken::new(), move || {
                    preferences.save(&data)
                })
                .await?;
        }
        if let Some(session) = self.focused {
            let _ = client.rpc(Request::Focus { session }).await;
        }
        let _ = client.rpc(Request::Heartbeat { focused: false }).await;
        Ok(())
    }
}
impl App {
    pub(super) fn begin_exit(&mut self) {
        if !self.exit.active() {
            self.player.stop();
            self.services.pause_reads(true);
            self.browser_host.shutdown();
            self.exit_attempt = self.exit_attempt.wrapping_add(1);
            self.exit = Exit::Waiting(Instant::now());
        }
    }
    fn dirty_native_path(&self) -> Option<PathBuf> {
        self.native_docs
            .iter()
            .find_map(|(path, doc)| doc.dirty().then(|| path.clone()))
    }
    /// Window close, Quit, and Sparkle's terminate all arrive here.
    /// A dirty native buffer must be resolved before the checkpoint, and that
    /// resolution has to come back to this quit.
    pub(super) fn accept_app_quit(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        if self.exit.active() {
            return;
        }
        // Floating panes rejoin their workspaces first, so dirty-buffer
        // prompts and the exit checkpoint see the complete layout.
        self.dock_back_all_floating();
        let waiting_on_native = self.dirty_native_path().is_some()
            || self.native_close_prompt.is_some()
            || self.native_close_after_save.is_some()
            || !self.pending_native_close.is_empty();
        if waiting_on_native {
            self.pending_app_quit = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.continue_app_quit(ctx);
        } else {
            self.pending_app_quit = false;
            self.begin_exit();
        }
    }
    pub(super) fn continue_app_quit(&mut self, ctx: &egui::Context) {
        if !self.pending_app_quit || self.exit.active() {
            return;
        }
        if self.native_close_after_save.is_some() || !self.pending_native_close.is_empty() {
            return;
        }
        if self.native_close_prompt.is_some() {
            return;
        }
        if let Some(path) = self.dirty_native_path() {
            self.native_close_prompt = Some((path, None));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            return;
        }
        self.pending_app_quit = false;
        self.begin_exit();
    }
    pub(super) fn cancel_app_quit(&mut self) {
        if !self.pending_app_quit {
            return;
        }
        self.pending_app_quit = false;
        self.native_close_after_save = None;
        self.native_close_after_save_issuer = None;
        crate::updater::cancel_termination();
    }
    pub(super) fn native_installation_cancelled(&mut self) {
        self.exit = Exit::Idle;
        self.services.pause_reads(false);
        self.services.handle().reopen_admission();
    }
    pub(super) fn cancel_exit(&mut self, error: String) {
        self.exit = Exit::Idle;
        self.services.pause_reads(false);
        self.services.handle().reopen_admission();
        self.error = Some(format!(
            "Could not close Terminator: {error}. Please retry closing."
        ));
        crate::updater::cancel_termination();
    }
    pub(super) fn exit_checkpoint(&self) -> Result<Checkpoint> {
        Ok(Checkpoint {
            layouts: self.persistable_layouts(),
            preferences: self.preferences_writable.then(|| self.preferences.clone()),
            selected: self
                .selected
                .clone()
                .filter(|project| self.has_project(project)),
            focused: self.active_session.clone(),
        })
    }
    pub(super) fn advance_exit(&mut self, ctx: &egui::Context) {
        let started = match self.exit {
            Exit::Waiting(t) | Exit::Draining(t, _) | Exit::Saving(t, _) => t,
            _ => return,
        };
        if started.elapsed() > Duration::from_secs(15) {
            self.cancel_exit("Timed out waiting for pending operations or persistence".into());
            return;
        }
        if let Some(services) = &self.jobs.services {
            let applied = self.service_ready.is_empty()
                && self.service_owner.events.is_empty()
                && !self.service_owner.supervisor.has_results()
                && self
                    .service_owner
                    .snapshot
                    .try_lock()
                    .ok()
                    .is_some_and(|state| state.is_none());
            if matches!(self.exit, Exit::Waiting(_))
                && !self.picker_active
                && services.idle()
                && applied
            {
                match self.exit_checkpoint() {
                    Ok(checkpoint) => {
                        self.exit = Exit::Saving(started, self.exit_attempt);
                        if self
                            .jobs
                            .send(Job::ExitSave(self.exit_attempt, checkpoint))
                            .is_err()
                        {
                            self.cancel_exit(
                                "Services are busy; final checkpoint was not admitted".into(),
                            );
                        } else {
                            self.services.handle().close_admission();
                        }
                    }
                    Err(error) => self.cancel_exit(format!("{error:#}")),
                }
            }
            ctx.request_repaint_after(Duration::from_millis(20));
            return;
        }
        if matches!(self.exit, Exit::Waiting(_)) && !self.picker_active {
            self.exit = Exit::Draining(started, self.exit_attempt);
            if self
                .jobs
                .send(Job::ExitDrain(self.exit_attempt, self.jobs.serial()))
                .is_err()
            {
                self.cancel_exit("Worker disconnected".into());
            }
        }
        ctx.request_repaint_after(Duration::from_millis(20));
    }
}

/// Count ordinary queued jobs so follow-up work produced while applying a drain
/// is itself drained before taking the final GUI snapshot.
#[derive(Clone)]
pub(super) struct JobQueue {
    sender: Option<Sender<Job>>,
    services: Option<gui_services::Services>,
    serial: std::sync::Arc<std::sync::atomic::AtomicU64>,
}
impl From<Sender<Job>> for JobQueue {
    fn from(sender: Sender<Job>) -> Self {
        Self {
            sender: Some(sender),
            services: None,
            serial: std::sync::Arc::default(),
        }
    }
}
impl JobQueue {
    pub fn supervised(services: gui_services::Services) -> Self {
        Self {
            sender: None,
            services: Some(services),
            serial: std::sync::Arc::default(),
        }
    }
    pub fn serial(&self) -> u64 {
        self.serial.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn send(&self, job: Job) -> Result<(), ()> {
        if !matches!(job, Job::ExitDrain(..) | Job::ExitSave(..)) {
            self.serial
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }
        if let Some(services) = &self.services {
            services.send(job)
        } else {
            self.sender.as_ref().ok_or(())?.send(job).map_err(|_| ())
        }
    }
}

#[cfg(test)]
mod tests {
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
    #[tokio::test]
    #[ignore = "requires workspace binaries; uses isolated real daemon and PTY"]
    async fn dead_service_recovers_attachment_and_exit_checkpoint() {
        let dir = tempfile::Builder::new()
            .prefix("exit-recovery-")
            .tempdir_in("/tmp")
            .unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        struct Cleanup(Paths);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Ok(catalog) = generations::Catalog::open(&self.0) {
                    for owner in catalog.generations().unwrap_or_default() {
                        let _ = rpc(&owner.paths(), Request::Shutdown);
                    }
                }
            }
        }
        let _cleanup = Cleanup(paths.clone());
        let bins = std::env::var_os("TERMINATOR_TEST_BIN_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/debug")
            });
        let exe = bins.join(exe_name("terminator"));
        daemon_connection::ensure_running(&paths, &exe).unwrap();
        rpc(
            &paths,
            Request::AddProject {
                path: dir.path().into(),
            },
        )
        .unwrap();
        let Response::State(state) = rpc(&paths, Request::Snapshot).unwrap() else {
            panic!()
        };
        let project = state.projects[0].id.clone();
        let create = || Request::Create {
            project: project.clone(),
            cwd: None,
            file: None,
            line: None,
            column: None,
            editor: false,
        };
        let Response::Created(old) = rpc(&paths, create()).unwrap() else {
            panic!()
        };
        let Response::State(before) = rpc(&paths, Request::Snapshot).unwrap() else {
            panic!()
        };
        let owner = before
            .generations
            .iter()
            .find(|health| health.owner.id == before.generation)
            .unwrap()
            .owner
            .clone();
        // Kill only this fixture's daemon; its owned PTY disappears with it.
        signals::signal_process(owner.pid.unwrap(), signals::ProcSignal::Kill).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !signals::process_gone(owner.pid.unwrap()) {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // Reproduce the reported missing-auth path as well as daemon death.
        fs::remove_file(owner.paths().auth()).unwrap();
        let Response::State(unavailable) = rpc(&paths, Request::Snapshot).unwrap() else {
            panic!()
        };
        assert!(!daemon_connection::active_service_available(&unavailable));
        assert!(daemon_connection::recover_unavailable(&paths, &unavailable, &exe).unwrap());
        let Response::State(after) = rpc(&paths, Request::Snapshot).unwrap() else {
            panic!()
        };
        assert!(daemon_connection::active_service_available(&after));
        assert_ne!(before.generation, after.generation);
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
        // A new, user-requested terminal must attach to the replacement.
        let Response::Created(new) = rpc(&paths, create()).unwrap() else {
            panic!()
        };
        let mut stream = connect(
            &paths,
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
        let client = async_client::Client::new(
            paths.clone(),
            async_service::NativePool::new("exit-catalog-test", 1).unwrap(),
            async_service::NativePool::new("exit-cpu-test", 1).unwrap(),
        );
        let checkpoint = Checkpoint {
            layouts: vec![(project.clone(), Workspace::empty())],
            preferences: Some(UiPreferences::default()),
            selected: Some(project.clone()),
            focused: Some(old.id),
        };
        checkpoint
            .for_state(&after)
            .save_async(&client)
            .await
            .unwrap();
        assert!(paths.data.join("ui-preferences.json").exists());
        let Response::State(saved) = rpc(&paths, Request::Snapshot).unwrap() else {
            panic!()
        };
        assert_eq!(saved.selected_project.as_deref(), Some(project.as_str()));
        assert!(
            saved
                .projects
                .iter()
                .find(|p| p.id == project)
                .unwrap()
                .layout
                .is_object()
        );
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
            super::native_editor::NativeDoc::dirty_for_test(path.clone()),
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
            super::native_editor::NativeDoc::dirty_for_test(path),
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
}
