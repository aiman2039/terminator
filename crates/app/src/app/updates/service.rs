//! Service completions, worker lifecycle, exit, and UI control requests.

use std::ops::ControlFlow;
use std::time::Instant;

use crate::*;

impl App {
    /// Moves one finished service operation into `service_ready`. Breaks
    /// when the completion carried no result, so the caller skips this
    /// pass without reading an update.
    pub(super) fn poll_service_completion(&mut self, ctx: &egui::Context) -> ControlFlow<()> {
        self.service_completion.take();
        let Some(mut completion) = self.service_owner.supervisor.try_recv() else {
            return ControlFlow::Continue(());
        };
        let Some(result) = completion.result.take() else {
            return ControlFlow::Break(());
        };
        let id = completion.context.id;
        match result {
            Ok(updates) => {
                self.services.settle(id, Ok(()));
                self.service_ready.extend(updates);
            }
            Err(async_service::Failure::Cancelled) => {
                let owed = self
                    .services
                    .settle(id, Err(&async_service::Failure::Cancelled));
                self.service_cancelled(&completion.context, owed);
            }
            Err(error) => {
                let owed = self.services.settle(id, Err(&error));
                self.service_failed(&completion.context, &error, owed);
            }
        }
        self.service_completion = Some(completion);
        ctx.request_repaint();
        ControlFlow::Continue(())
    }

    /// A cancelled operation. `owed` holds its view's reply when nothing
    /// newer replaced it (a read stopped by Quit that was then cancelled).
    fn service_cancelled(&mut self, context: &async_service::OperationContext, owed: Vec<Update>) {
        let key = &context.resource;
        match context.subsystem {
            "diff" | "files" => {
                self.loading.remove(key);
            }
            git_log::LOG_SUBSYSTEM => self.log_job_ended(key, context.generation, None),
            "vnc" => {
                if let Some(sid) = key.strip_prefix("frame:")
                    && let Some(view) = self.vnc_views.get_mut(sid)
                {
                    view.fetching = false;
                }
            }
            "images" => {
                if let Some(preview) = self
                    .images
                    .values_mut()
                    .find(|preview| preview.generation == context.generation)
                {
                    preview.loading = false;
                    preview.cancellation = None;
                }
            }
            // Its view still waits: answer it as a failure.
            _ => self.service_ready.extend(owed),
        }
    }

    /// A job that ended without its reply. The view that waits for it
    /// gets the reply the job owes (`owed`), else its subsystem's.
    fn service_failed(
        &mut self,
        context: &async_service::OperationContext,
        error: &async_service::Failure,
        owed: Vec<Update>,
    ) {
        if context.subsystem == gui_services::COMMIT_SUBSYSTEM {
            // The commit or push run ended without its reply.
            self.handle_commit_reply(Err(error.to_string()));
            return;
        }
        if context.subsystem == git_log::LOG_SUBSYSTEM {
            let error = format!("Log query stopped: {error}");
            self.log_job_ended(&context.resource, context.generation, Some(error));
            return;
        }
        // A failed service job still ends its restart, repair or start.
        let ended = service_op::failure_update(context, error);
        let updates: Vec<Update> = if !owed.is_empty() {
            owed.into_iter().chain(ended).collect()
        } else {
            ended
                .or_else(|| service_failure_update(context, error))
                .into_iter()
                .collect()
        };
        for update in updates {
            if let Update::ResolvedTarget(key, _) = &update {
                self.loading.remove(key);
            }
            self.service_ready.push_back(update);
        }
    }

    pub(super) fn handle_ui_request(
        &mut self,
        ctx: &egui::Context,
        request: terminator_core::ui_control::Request,
        deadline: Instant,
    ) -> Result<serde_json::Value, String> {
        if Instant::now() >= deadline {
            Err("GUI request expired before processing; no action was performed".into())
        } else {
            self.ui_request(ctx, request).map_err(|e| format!("{e:#}"))
        }
    }

    pub(super) fn handle_installation_repaired(&mut self, result: Result<Box<State>, String>) {
        self.end_service_op(ServiceOp::Repairing);
        match result {
            Ok(state) => {
                self.apply_state(*state);
                self.installation_error = None;
                self.service_failure = None;
                self.error.clear();
                self.info = Some("Installation repaired. New terminals can be opened.".into());
            }
            Err(error) => {
                let message = format!("Could not repair installation: {error}");
                self.fail_service_op(ServiceTask::Repair, message);
            }
        }
    }

    /// A completion ends only its own operation.
    fn end_service_op(&mut self, op: ServiceOp) {
        if self.service_op == op {
            self.service_op = ServiceOp::Idle;
        }
    }

    fn fail_service_op(&mut self, task: ServiceTask, message: String) {
        self.error.set(message.clone());
        self.service_failure = Some(ServiceFailure { task, message });
    }

    pub(super) fn handle_exit_drained(&mut self, id: u64, serial: u64) {
        let exit::Exit::Draining(started, current) = self.exit else {
            return;
        };
        if current != id {
            return;
        }
        if serial != self.jobs.serial() {
            let _ = self.jobs.send(Job::ExitDrain(id, self.jobs.serial()));
            return;
        }
        match self.exit_checkpoint() {
            Ok(checkpoint) => {
                self.exit = exit::Exit::Saving(started, id);
                if self.jobs.send(Job::ExitSave(id, checkpoint)).is_err() {
                    self.cancel_exit("Worker disconnected".into());
                }
            }
            Err(e) => self.cancel_exit(format!("{e:#}")),
        }
    }

    pub(super) fn handle_exit_saved(
        &mut self,
        ctx: &egui::Context,
        id: u64,
        result: Result<(), String>,
    ) {
        if !matches!(self.exit, exit::Exit::Saving(_, current) if current == id) {
            return;
        }
        match result {
            Ok(()) => {
                self.finish_exit();
                updater::complete_termination(ctx);
            }
            Err(e) => self.cancel_exit(e),
        }
    }

    pub(super) fn handle_metadata(&mut self, generation: u64, data: metadata::Metadata) {
        if generation == self.metadata_generation {
            self.metadata = Some(data);
        }
    }

    pub(super) fn handle_restart_finished(&mut self, message: &str) {
        self.end_service_op(ServiceOp::Restarting);
        let message = if message.is_empty() {
            "Session service restart did not complete. See restart.log in the data directory for details, then retry.".into()
        } else {
            format!("Session service restart failed: {message}")
        };
        self.fail_service_op(ServiceTask::Restart, message);
    }

    pub(super) fn handle_service_started(&mut self, result: Result<(), String>) {
        self.end_service_op(ServiceOp::Starting);
        match result {
            Ok(()) => {
                self.info = Some("Session service started. Reconnecting…".into());
            }
            Err(error) => {
                let message = format!("Could not start session service: {error}");
                self.fail_service_op(ServiceTask::Start, message);
            }
        }
    }
}

/// The reply of a failed job that owes none of its own (see
/// `gui_services::jobs::replies`): known subsystems answer their view,
/// the rest report in the status bar.
pub(crate) fn service_failure_update(
    context: &async_service::OperationContext,
    error: &async_service::Failure,
) -> Option<Update> {
    match context.subsystem {
        // A diff or gutter write; one in a known repository ran on its queue.
        "diff-action" | "workspace-op" if let Some(key) = &context.tab => {
            Some(Update::DiffActionFailed(key.clone(), error.to_string()))
        }
        "diff" => Some(Update::Diff(
            context.resource.clone(),
            Err(error.to_string()),
        )),
        "files" => Some(Update::ResolvedTarget(context.resource.clone(), None)),
        "images" => Some(Update::Image(
            std::path::PathBuf::from(&context.resource),
            context.generation,
            Err(error.to_string()),
        )),
        "vnc" => context
            .resource
            .strip_prefix("frame:")
            .map(|session| Update::VncFrame(session.to_owned(), Err(error.to_string()))),
        "audio-spectrum" => None,
        _ => Some(Update::Error(error.to_string())),
    }
}
