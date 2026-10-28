use anyhow::Result;
use std::{
    sync::{Arc, Weak, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};
use terminator_core::{Paths, Request, generations};

use crate::server::{Shared, relock};

/// Once a second, keeps this generation in step with the shared catalog:
/// refreshes state, prunes retained history and retires a drained owner.
pub(crate) struct GenerationWorker {
    born: Instant,
    global_prune: Instant,
}

/// What the catalog says about this generation, read under coordination.
struct Survey {
    owner: String,
    owners: Vec<generations::Generation>,
    draining: bool,
    active_owner: bool,
}

impl GenerationWorker {
    pub(crate) fn spawn(weak: Weak<Shared>) {
        thread::spawn(move || {
            let mut worker = Self {
                born: Instant::now(),
                global_prune: Instant::now()
                    .checked_sub(Duration::from_secs(30))
                    .unwrap_or_else(Instant::now),
            };
            loop {
                thread::sleep(Duration::from_secs(1));
                let Some(shared) = weak.upgrade() else {
                    break;
                };
                if shared.shutdown.load(Ordering::Acquire) {
                    break;
                }
                if let Err(error) = worker.tick(&shared) {
                    shared.degraded(&format!("Generation maintenance: {error:#}"));
                }
            }
        });
    }

    fn tick(&mut self, shared: &Arc<Shared>) -> Result<()> {
        let Some(root) = &shared.catalog_paths else {
            return Ok(());
        };
        let survey = self.survey(shared, root)?;
        if survey.active_owner {
            self.maintain_catalog(shared, root, &survey);
        }
        if survey.draining
            && !relock(&shared.state)
                .sessions
                .iter()
                .any(|s| s.lifecycle.live())
        {
            let _ = shared.handle(Request::RetireIfDraining);
        }
        Ok(())
    }

    fn survey(&self, shared: &Shared, root: &Paths) -> Result<Survey> {
        let _coordination = generations::coordinate(root)?;
        let catalog = generations::Catalog::open(root)?;
        let owner = relock(&shared.state).generation.clone();
        catalog.refresh(&mut relock(&shared.state))?;
        if catalog.active()?.as_deref() == Some(&owner) && !shared.shutdown.load(Ordering::Acquire)
        {
            let _ = catalog.restore_serving();
        }
        let owners = catalog.generations()?;
        let draining = owners.iter().any(|g| {
            g.id == owner
                && (g.status == generations::Status::Draining
                    || (g.status == generations::Status::Prepared
                        && self.born.elapsed() > Duration::from_secs(30)))
        });
        let active_owner = catalog.active()?.as_deref() == Some(&owner);
        Ok(Survey {
            owner,
            owners,
            draining,
            active_owner,
        })
    }

    /// The active owner prunes global retention every 30 s and recovers
    /// other generations whose process exited.
    fn maintain_catalog(&mut self, shared: &Arc<Shared>, root: &Paths, survey: &Survey) {
        if self.global_prune.elapsed() >= Duration::from_secs(30) {
            if let Err(error) = shared.prune_global(&survey.owners) {
                shared.degraded(&format!("Global history retention: {error:#}"));
            }
            if let Err(error) = generations::prune_retired(root, generations::RETIRED_RETENTION) {
                shared.degraded(&format!(
                    "Generation retention: {error}",
                    error = terminator_core::ErrorChain(&error)
                ));
            }
            self.global_prune = Instant::now();
        }
        for other in survey
            .owners
            .iter()
            .filter(|g| g.id != survey.owner && g.status != generations::Status::Retired)
        {
            let _ = generations::recover_exited(root, other);
        }
    }
}
