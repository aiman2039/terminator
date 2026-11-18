//! The listing job: submitted to the GUI services as a replaceable read,
//! its file reads on the filesystem pool, never on the GUI thread.
use crate::App;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use terminator_core::agents::conversations::{self, Limits, Listing, Roots};
use terminator_core::async_service::{CancellationToken, Failure, OperationContext, Policy};

impl App {
    /// Starts listing the conversations of `folder`; a running listing is
    /// replaced. Returns at once.
    pub(super) fn start_agent_sessions_job(&mut self, folder: PathBuf) {
        let cancel = CancellationToken::new();
        let mut context =
            OperationContext::new("agent-sessions", "list".into(), Policy::ReplaceableRead);
        context.deadline = Instant::now().checked_add(conversations::TIME_BUDGET.saturating_mul(2));
        let slot = Arc::clone(&self.agent_sessions.slot);
        let generation = self.agent_sessions.generation;
        let fs = self.services.fs().clone();
        let work = cancel.clone();
        let submitted = self
            .services
            .handle()
            .submit(context, cancel.clone(), async move {
                let token = work.clone();
                let listing = fs
                    .run(&work, move || {
                        let stop = || token.is_cancelled();
                        Ok::<_, Failure>(conversations::list(
                            &Roots::discover(),
                            &folder,
                            Limits::default(),
                            &stop,
                        ))
                    })
                    .await;
                if let Ok(listing) = listing {
                    *slot
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some((generation, listing));
                }
                Ok(Vec::new())
            });
        match submitted {
            Ok(_) => self.agent_sessions.cancel = Some(cancel),
            Err(error) => {
                self.agent_sessions.listing = Some(Listing::default());
                self.info = Some(format!("Could not list agent sessions: {error:#}"));
            }
        }
    }
}
