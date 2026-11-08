//! Remote app windows (`remote-apps-v1`): one session per remote project.
//! The daemon owns it like a VNC session: closing the GUI leaves the bridge
//! running, and apps live on in the host's wprsd even when the bridge stops.
//! See docs/REMOTE_APPS.md.

mod runtime;
mod worker;

#[cfg(test)]
mod tests;

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use terminator_core::remote_apps::{ClientMessage, MAX_INPUT, MAX_WAIT_MS, RemoteAppsState};

use crate::server::{Shared, relock};
pub(crate) use runtime::Runtime;

/// A session's state and the condition the GUI's long poll waits on.
pub(crate) struct Handle {
    runtime: Mutex<Runtime>,
    changed: Condvar,
}

impl Handle {
    fn new(runtime: Runtime) -> Self {
        Self {
            runtime: Mutex::new(runtime),
            changed: Condvar::new(),
        }
    }

    /// Runs `change` and wakes waiting polls.
    pub(crate) fn update(&self, change: impl FnOnce(&mut Runtime)) {
        change(&mut relock(&self.runtime));
        self.changed.notify_all();
    }

    fn snapshot(&self) -> RemoteAppsState {
        relock(&self.runtime).snapshot()
    }

    /// The state once its revision passes `since`, or `None` after `wait`.
    fn wait_newer(&self, since: u64, wait: Duration) -> Option<RemoteAppsState> {
        let deadline = Instant::now().checked_add(wait);
        let mut runtime = relock(&self.runtime);
        loop {
            if runtime.revision > since {
                return Some(runtime.snapshot());
            }
            let left = deadline?.checked_duration_since(Instant::now())?;
            runtime = self
                .changed
                .wait_timeout(runtime, left)
                .map(|(guard, _)| guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner().0);
        }
    }
}

impl Shared {
    /// The live session of `project`, or a new one.
    pub(crate) fn remote_apps_connect(&self, project_id: &str) -> Result<RemoteAppsState> {
        let project = relock(&self.state)
            .remote_projects
            .iter()
            .find(|p| p.id == project_id)
            .cloned()
            .context("Unknown SSH project")?;
        let mut sessions = relock(&self.remote_apps);
        if let Some(live) = sessions.values().find(|h| {
            let r = relock(&h.runtime);
            r.project_id == project_id && r.status.live()
        }) {
            return Ok(live.snapshot());
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let handle = Arc::new(Handle::new(Runtime::new(
            id.clone(),
            project.id.clone(),
            self.paths.runtime.clone(),
        )));
        // Ended sessions of this project are replaced.
        sessions.retain(|_, h| {
            let r = relock(&h.runtime);
            r.project_id != project_id || r.status.live()
        });
        sessions.insert(id, Arc::clone(&handle));
        drop(sessions);
        worker::start(&handle, &project, self.ssh_control(&project.ssh));
        Ok(handle.snapshot())
    }

    pub(crate) fn remote_apps_disconnect(&self, session: &str) -> Result<()> {
        let handle = self.remote_apps_handle(session)?;
        // Dropping the input sender closes the bridge's stdin; it exits and
        // the worker marks the session disconnected.
        handle.update(|r| r.input = None);
        Ok(())
    }

    pub(crate) fn remote_apps_input(
        &self,
        session: &str,
        messages: Vec<ClientMessage>,
    ) -> Result<()> {
        ensure!(messages.len() <= MAX_INPUT, "Too many input messages");
        let handle = self.remote_apps_handle(session)?;
        let runtime = relock(&handle.runtime);
        let input = runtime
            .input
            .as_ref()
            .context("Remote apps are not connected")?;
        for message in messages {
            input
                .send(message)
                .ok()
                .context("Remote apps are not connected")?;
        }
        Ok(())
    }

    /// `None` when nothing changed after `since` within `wait_ms`.
    pub(crate) fn remote_apps_state(
        &self,
        session: &str,
        since: u64,
        wait_ms: u32,
    ) -> Result<Option<RemoteAppsState>> {
        let handle = self.remote_apps_handle(session)?;
        let wait = Duration::from_millis(u64::from(wait_ms.min(MAX_WAIT_MS)));
        Ok(handle.wait_newer(since, wait))
    }

    fn remote_apps_handle(&self, session: &str) -> Result<Arc<Handle>> {
        relock(&self.remote_apps)
            .get(session)
            .cloned()
            .context("Unknown remote-apps session")
    }

    /// Whether a live remote-apps session uses `project`.
    pub(crate) fn remote_apps_live_for(&self, project: &str) -> bool {
        relock(&self.remote_apps).values().any(|h| {
            let r = relock(&h.runtime);
            r.project_id == project && r.status.live()
        })
    }
}
