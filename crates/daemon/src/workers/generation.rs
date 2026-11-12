use anyhow::Result;
use std::{
    sync::{Arc, Weak, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};
use terminator_core::{Paths, Request, generations, runtime_dir};

use crate::server::{Shared, relock};

/// Warning prefixes; each clears when its source succeeds again.
const PERSIST: &str = "Agent state not saved: ";
const MAINTENANCE: &str = "Generation maintenance: ";
const PRUNE_GLOBAL: &str = "Global history retention: ";
const PRUNE_RETIRED: &str = "Generation retention: ";

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
            let mut kept_alive = Instant::now();
            loop {
                thread::sleep(Duration::from_secs(1));
                let Some(shared) = weak.upgrade() else {
                    break;
                };
                if shared.shutdown.load(Ordering::Acquire) {
                    break;
                }
                if shared.persist_pending.load(Ordering::Acquire) {
                    match shared.persist() {
                        Err(error) => shared.degraded(&format!("{PERSIST}{error:#}")),
                        Ok(()) => shared.recovered(PERSIST),
                    }
                }
                match worker.tick(&shared) {
                    Err(error) => shared.degraded(&format!("{MAINTENANCE}{error:#}")),
                    Ok(()) => shared.recovered(MAINTENANCE),
                }
                if kept_alive.elapsed() >= runtime_dir::KEEP_ALIVE_INTERVAL {
                    kept_alive = Instant::now();
                    keep_runtime_files(&shared);
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
        // A newer generation may write workspace data this build cannot
        // read. Draining generations do not need it: they must still see
        // that they drain and retire, so the error waits until then.
        let refreshed = catalog.refresh(&mut relock(&shared.state));
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
        refresh_needed(refreshed.map_err(Into::into), draining)?;
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
            match shared.prune_global(&survey.owners) {
                // Another holder has the coordination lock: retry on the next tick.
                Err(error) if is_busy(&error) => return,
                Err(error) => shared.degraded(&format!("{PRUNE_GLOBAL}{error:#}")),
                Ok(()) => shared.recovered(PRUNE_GLOBAL),
            }
            match generations::prune_retired(root, generations::RETIRED_RETENTION) {
                Err(error) => shared.degraded(&format!(
                    "{PRUNE_RETIRED}{error}",
                    error = terminator_core::ErrorChain(&error)
                )),
                Ok(_) => shared.recovered(PRUNE_RETIRED),
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

/// The OS temp cleaner deletes untouched runtime files. Without `auth`
/// clients cannot reach this daemon, so restore it and keep the locks
/// fresh. Best effort: a later pass retries.
fn keep_runtime_files(shared: &Shared) {
    let _ = runtime_dir::keep_token(&shared.paths.auth(), &shared.auth);
    let _ = runtime_dir::touch(&shared.paths.runtime.join("daemon.lock"));
    if let Some(root) = &shared.catalog_paths {
        let _ = runtime_dir::touch(&root.runtime.join("daemon.lock"));
    }
}

/// A bounded coordination wait expired; background work retries later.
fn is_busy(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<generations::GenerationsError>(),
            Some(generations::GenerationsError::Busy { .. })
        )
    })
}

/// A failed workspace refresh stops the survey, except while this
/// generation drains: it does not need the shared workspace, and it must
/// still see that it drains so it can retire.
fn refresh_needed(refreshed: Result<()>, draining: bool) -> Result<()> {
    if draining { Ok(()) } else { refreshed }
}

#[cfg(test)]
mod tests {
    use super::refresh_needed;

    #[test]
    fn a_draining_generation_retires_even_when_it_cannot_read_newer_workspace() {
        let unreadable = || Err(anyhow::anyhow!("unknown variant `from_the_future`"));
        assert!(refresh_needed(unreadable(), true).is_ok());
        assert!(refresh_needed(unreadable(), false).is_err());
        assert!(refresh_needed(Ok(()), false).is_ok());
    }
}
