use anyhow::{Context, Result, ensure};
use std::{sync::atomic::Ordering, time::Instant};
use terminator_core::*;

use super::super::super::shared::{Shared, relock};
use super::Outcome;
use crate::{notifications, presence};

impl Shared {
    pub(super) fn on_snapshot(&self) -> Outcome {
        let mut state = relock(&self.state);
        presence::expire_observations(&mut state, now());
        let mut snapshot = state.clone();
        for profile in &mut snapshot.vnc_profiles {
            if !profile.password.is_empty() {
                profile.credential_saved = true;
                profile.password.clear();
            }
        }
        Outcome::Reply(Response::State(Box::new(snapshot)))
    }

    pub(super) fn on_heartbeat(&self, focused: bool) -> Outcome {
        *relock(&self.focused) = (focused, Instant::now());
        Outcome::Reply(Response::Ok)
    }

    pub(super) fn on_settings(&self, settings: Settings) -> Result<Outcome> {
        settings.validate()?;
        let mut s = relock(&self.state);
        s.settings = settings;
        s.revision = s.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    pub(super) fn on_retire_if_draining(&self) -> Result<Outcome> {
        self.ensure_draining()?;
        self.on_shutdown()
    }

    pub(super) fn on_shutdown(&self) -> Result<Outcome> {
        let _creation_guard = relock(&self.worktree_operations);
        ensure!(
            !relock(&self.state)
                .sessions
                .iter()
                .any(|s| s.lifecycle.live()),
            "Stop running sessions before stopping daemon"
        );
        self.flush_history()?;
        self.persist()?;
        self.shutdown.store(true, Ordering::Release);
        notifications::wake();
        if let Some(root) = &self.catalog_paths {
            generations::Catalog::open(root)?.retire(&relock(&self.state).generation)?;
        }
        Ok(Outcome::Reply(Response::Ok))
    }

    /// Only a prepared or draining generation may retire itself.
    fn ensure_draining(&self) -> Result<()> {
        let root = self
            .catalog_paths
            .as_ref()
            .context("No generation catalog")?;
        let mine = relock(&self.state).generation.clone();
        ensure!(
            generations::Catalog::open(root)?
                .generations()?
                .iter()
                .any(|g| g.id == mine
                    && matches!(
                        g.status,
                        generations::Status::Prepared | generations::Status::Draining
                    )),
            "Generation is active"
        );
        Ok(())
    }
}
