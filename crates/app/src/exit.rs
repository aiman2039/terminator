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
        if let Some(session) = self.focused {
            requests.push(Request::Focus { session });
        }
        requests.push(Request::Heartbeat { focused: false });
        for request in requests {
            anyhow::ensure!(
                matches!(rpc(paths, request)?, Response::Ok),
                "Exit persistence was not acknowledged"
            );
        }
        if let Some(preferences) = self.preferences {
            preferences.save(&paths.data)?;
        }
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
        if let Some(session) = self.focused {
            requests.push(Request::Focus { session });
        }
        requests.push(Request::Heartbeat { focused: false });
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
            layouts: self
                .layouts
                .iter()
                .filter(|(project, _)| !self.layout_readonly.contains(*project))
                .map(|(project, layout)| (project.clone(), layout.clone()))
                .collect(),
            preferences: self.preferences_writable.then(|| self.preferences.clone()),
            selected: self.selected.clone(),
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
            serial: Default::default(),
        }
    }
}
impl JobQueue {
    pub fn supervised(services: gui_services::Services) -> Self {
        Self {
            sender: None,
            services: Some(services),
            serial: Default::default(),
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
    fn fixture() -> (App, egui::Context, tempfile::TempDir, Receiver<Job>) {
        let directory = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(directory.path().into()));
        let (jobs, requests) = mpsc::channel();
        app.jobs = jobs.into();
        (app, ctx, directory, requests)
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
        app.process_updates(&ctx);
        assert!(!app.exit.active());
        assert!(app.error.as_ref().unwrap().contains("disk full"));
        app.begin_exit();
        app.update_tx.send(Update::ExitSaved(1, Ok(()))).unwrap();
        app.process_updates(&ctx);
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
        app.process_updates(&ctx);
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
        app.process_updates(&ctx);
        assert!(matches!(app.exit, Exit::Draining(_, 2)));
    }
    #[test]
    fn timed_out_exit_can_be_retried() {
        let (mut app, ctx, _dir, _) = fixture();
        app.exit = Exit::Saving(Instant::now() - Duration::from_secs(16), 1);
        app.advance_exit(&ctx);
        assert!(!app.exit.active());
        app.begin_exit();
        assert!(matches!(app.exit, Exit::Waiting(_)));
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
        app.process_updates(&ctx);
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
        use std::os::unix::net::UnixListener;
        let directory = tempfile::tempdir().unwrap();
        let paths = Paths::at(directory.path().into());
        paths.init().unwrap();
        fs::write(paths.auth(), "fixture").unwrap();
        let listener = UnixListener::bind(paths.socket()).unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let _: Envelope = read_frame(&mut socket).unwrap();
            write_frame(&mut socket, &Response::Error("disk full".into())).unwrap();
        });
        let checkpoint = Checkpoint {
            layouts: vec![],
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
}
