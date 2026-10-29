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
        terminator_core::ssh::set_tuning(settings.ssh_tuning());
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
            !has_live_sessions(&relock(&self.state)),
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

/// Live terminals or VNC connections. A VNC connection lives only in this
/// process, so stopping the daemon would drop it like a terminal.
fn has_live_sessions(state: &State) -> bool {
    state.sessions.iter().any(|s| s.lifecycle.live())
        || state.vnc_sessions.iter().any(|s| s.status.live())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_vnc_session_keeps_the_daemon_running() {
        let mut state = State::default();
        assert!(!has_live_sessions(&state));
        state.vnc_sessions.push(
            serde_json::from_value(serde_json::json!({
                "id": "v", "project_id": "", "label": "", "host": "h", "port": 5900,
                "width": 0, "height": 0, "view_only": false, "status": "connected",
                "created": 0, "generation": "g"
            }))
            .expect("minimal VNC session"),
        );
        assert!(has_live_sessions(&state));
    }
}
