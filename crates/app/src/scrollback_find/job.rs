//! The search job: submitted to the GUI services as a replaceable read,
//! never run on the GUI thread.
use super::{Found, MAX_BYTES, MAX_RESULTS, Matcher, Slot, TIME_BUDGET, scan_text};
use crate::App;
use crate::gui_services::Services;
use std::time::Instant;
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use terminator_core::{Request, Response};

impl App {
    /// Starts a search for the page's query; a running one is replaced.
    /// Returns at once: the work runs on the services.
    pub(crate) fn start_scrollback_find(&mut self, ctx: &eframe::egui::Context) {
        let query = self.scrollback_find.query.clone();
        let sessions = self.scrollback_find_sessions(self.scrollback_find.all_projects);
        let page = &mut self.scrollback_find;
        page.stop();
        page.generation = page.generation.wrapping_add(1);
        page.searched = query.clone();
        let matcher = match Matcher::new(&query) {
            Ok(matcher) => matcher,
            Err(error) => {
                page.result = Some(Err(error));
                return;
            }
        };
        let cancel = CancellationToken::new();
        let job = Job {
            service: self.services.clone(),
            sessions,
            matcher,
            slot: std::sync::Arc::clone(&page.slot),
            generation: page.generation,
            ctx: ctx.clone(),
        };
        let mut context = OperationContext::new(
            "scrollback",
            "find-all-sessions".into(),
            Policy::ReplaceableRead,
        );
        context.deadline = Instant::now().checked_add(TIME_BUDGET.saturating_mul(2));
        let work = cancel.clone();
        let submitted = self
            .services
            .handle()
            .submit(context, cancel.clone(), async move {
                job.run(work).await;
                Ok(Vec::new())
            });
        page.result = None;
        match submitted {
            Ok(_) => {
                page.running = true;
                page.cancel = Some(cancel);
            }
            Err(error) => page.result = Some(Err(format!("{error:#}"))),
        }
    }
}

/// Everything the job needs, moved off the GUI thread.
struct Job {
    service: Services,
    sessions: Vec<String>,
    matcher: Matcher,
    slot: Slot,
    generation: u64,
    ctx: eframe::egui::Context,
}

impl Job {
    async fn run(self, cancel: CancellationToken) {
        let started = Instant::now();
        let mut found = Found::default();
        let mut bytes: usize = 0;
        for session in &self.sessions {
            if cancel.is_cancelled() {
                return;
            }
            if started.elapsed() >= TIME_BUDGET || bytes >= MAX_BYTES {
                found.stopped = true;
                break;
            }
            let request = Request::History {
                session: session.clone(),
            };
            let Ok(Response::Text(text)) = self.service.client().rpc(request).await else {
                found.failed = found.failed.saturating_add(1);
                continue;
            };
            bytes = bytes.saturating_add(text.len());
            let limit = MAX_RESULTS.saturating_sub(found.hits.len());
            let (matcher, id) = (self.matcher.clone(), session.clone());
            let scanned = self
                .service
                .cpu()
                .run(&cancel, move || {
                    anyhow::Ok(scan_text(&id, &text, &matcher, limit))
                })
                .await;
            let Ok((hits, more)) = scanned else {
                return;
            };
            found.hits.extend(hits);
            found.searched = found.searched.saturating_add(1);
            if more {
                found.truncated = true;
                break;
            }
        }
        *self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((self.generation, Ok(found)));
        self.ctx.request_repaint();
    }
}
