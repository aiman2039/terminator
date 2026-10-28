//! Terminal output events from attached PTY backends.

use std::time::Instant;

use egui_term::PtyEvent;

use crate::*;

impl App {
    pub(super) fn drain_pty_events(&mut self, ctx: &egui::Context) {
        let mut budget = native_jobs::ResultBudget::new();
        loop {
            if !budget.next() {
                ctx.request_repaint();
                break;
            }
            let Ok((id, event)) = self.pty_rx.try_recv() else {
                break;
            };
            self.handle_pty_event(ctx, id, &event);
        }
    }

    fn handle_pty_event(&mut self, ctx: &egui::Context, id: u64, event: &PtyEvent) {
        if let PtyEvent::ClipboardStore(_, text) = event {
            ctx.copy_text(text.clone());
        }
        if let PtyEvent::ChildExit(status) = event
            && !status.success()
        {
            self.record_attach_child_failure(id, status);
        }
        let failed = matches!(event, PtyEvent::Exit) && self.attach_failed.remove(&id);
        if let PtyEvent::Exit = event
            && let Some(session) = self.backend_ids.remove(&id)
            && self.backends.get(&session).is_some_and(|b| b.id() == id)
        {
            self.backend_exited(session, failed);
        }
    }

    fn record_attach_child_failure(&mut self, id: u64, status: &std::process::ExitStatus) {
        self.attach_failed.insert(id);
        if let Some(session) = self.backend_ids.get(&id)
            && self
                .backends
                .get(session)
                .is_some_and(|backend| backend.id() == id)
        {
            self.attach_error.insert(
                session.clone(),
                format!("Terminal attachment failed ({status})."),
            );
        }
    }

    fn backend_exited(&mut self, session: String, failed: bool) {
        self.backends.remove(&session);
        let started = self.attach_started.remove(&session);
        if retry_budget::attach_exit_is_failure(started.map(|at| at.elapsed()), failed) {
            if started.is_some_and(|at| at.elapsed() >= retry_budget::ATTACH_STABLE_WINDOW) {
                self.attach_budget.remove(&session);
            }
            self.attach_budget
                .entry(session.clone())
                .or_default()
                .record_failure(Instant::now());
            self.attach_error
                .entry(session)
                .or_insert_with(|| "Terminal attachment failed.".into());
        } else {
            self.attach_budget.remove(&session);
            self.attach_error.remove(&session);
        }
    }
}
