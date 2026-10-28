//! A checkpoint is acknowledged only after all final writes succeed.
#[cfg(test)]
mod tests;

use crate::*;

#[derive(Default)]
pub(crate) enum Exit {
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

pub(crate) struct Checkpoint {
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
    pub(crate) fn begin_exit(&mut self) {
        if self.block_unsaved_diff(None) {
            self.pending_app_quit = false;
            crate::updater::cancel_termination();
            return;
        }
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
    pub(crate) fn accept_app_quit(&mut self, ctx: &egui::Context) {
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
    pub(crate) fn continue_app_quit(&mut self, ctx: &egui::Context) {
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
    pub(crate) fn cancel_app_quit(&mut self) {
        if !self.pending_app_quit {
            return;
        }
        self.pending_app_quit = false;
        self.native_close_after_save = None;
        self.native_close_after_save_issuer = None;
        crate::updater::cancel_termination();
    }
    pub(crate) fn native_installation_cancelled(&mut self) {
        self.exit = Exit::Idle;
        self.services.pause_reads(false);
        self.services.handle().reopen_admission();
    }
    pub(crate) fn cancel_exit(&mut self, error: String) {
        self.exit = Exit::Idle;
        self.services.pause_reads(false);
        self.services.handle().reopen_admission();
        self.error = Some(format!(
            "Could not close Terminator: {error}. Please retry closing."
        ));
        crate::updater::cancel_termination();
    }
    pub(crate) fn exit_checkpoint(&self) -> Result<Checkpoint> {
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
    pub(crate) fn advance_exit(&mut self, ctx: &egui::Context) {
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
pub(crate) struct JobQueue {
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
