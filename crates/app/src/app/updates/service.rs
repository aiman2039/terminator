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
        match result {
            Ok(updates) => self.service_ready.extend(updates),
            Err(async_service::Failure::Cancelled) => self.service_cancelled(&completion.context),
            Err(error) => self.service_failed(&completion.context, &error),
        }
        self.service_completion = Some(completion);
        ctx.request_repaint();
        ControlFlow::Continue(())
    }

    fn service_cancelled(&mut self, context: &async_service::OperationContext) {
        let key = &context.resource;
        if matches!(context.subsystem, "diff" | "files") {
            self.loading.remove(key);
        }
        if context.subsystem == "vnc"
            && let Some(sid) = context.resource.strip_prefix("frame:")
            && let Some(view) = self.vnc_views.get_mut(sid)
        {
            view.fetching = false;
        }
        if context.subsystem == "images"
            && let Some(preview) = self
                .images
                .values_mut()
                .find(|preview| preview.generation == context.generation)
        {
            preview.loading = false;
            preview.cancellation = None;
        }
    }

    fn service_failed(
        &mut self,
        context: &async_service::OperationContext,
        error: &async_service::Failure,
    ) {
        let Some(update) = service_failure_update(context, error) else {
            return;
        };
        if let Update::ResolvedTarget(key, _) = &update {
            self.loading.remove(key);
        }
        self.service_ready.push_back(update);
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
        self.repair_pending = false;
        match result {
            Ok(state) => {
                self.apply_state(*state);
                self.installation_error = None;
                self.error = None;
                self.info = Some("Installation repaired. New terminals can be opened.".into());
            }
            Err(error) => {
                self.error = Some(format!("Could not repair installation: {error}"));
            }
        }
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
                self.exit = exit::Exit::Ready;
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
        self.restart_pending = false;
        self.error = Some(if message.is_empty() {
            "Session service restart did not complete. See restart.log in the data directory for details, then retry.".into()
        } else {
            format!("Session service restart failed: {message}")
        });
    }

    pub(super) fn handle_service_started(&mut self, result: Result<(), String>) {
        self.service_start_pending = false;
        match result {
            Ok(()) => {
                self.info = Some("Session service started. Reconnecting…".into());
            }
            Err(error) => {
                self.error = Some(format!("Could not start session service: {error}"));
            }
        }
    }
}
