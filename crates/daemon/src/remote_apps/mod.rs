//! Remote app windows (`remote-apps-v1`): one session per remote project.
//! The daemon owns it like a VNC session: closing the GUI leaves the bridge
//! running, and apps live on in the host's wprsd even when the bridge stops.
//! See docs/REMOTE_APPS.md.

mod pixels;
mod runtime;
mod window;
mod worker;

#[cfg(test)]
mod tests;

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use terminator_core::remote_apps::{
    ClientMessage, HostMessage, MAX_CLIPBOARD, MAX_INPUT, MAX_WAIT_MS, RemoteAppsState,
    TEXT_MIME_TYPES, UNKNOWN_SESSION, protocol,
};

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

    /// Runs `change` and wakes waiting polls; returns what `change` returns.
    pub(crate) fn update<T>(&self, change: impl FnOnce(&mut Runtime) -> T) -> T {
        let value = change(&mut relock(&self.runtime));
        self.changed.notify_all();
        value
    }

    /// Applies a bridge message; wakes waiting polls only when it changed
    /// something.
    pub(crate) fn apply(&self, framed: protocol::Framed<HostMessage>, rgba: Option<Vec<Vec<u8>>>) {
        if relock(&self.runtime).apply(framed, rgba) {
            self.changed.notify_all();
        }
    }

    pub(crate) fn read<T>(&self, view: impl FnOnce(&Runtime) -> T) -> T {
        view(&relock(&self.runtime))
    }

    /// Stops the session: no more retries, and the bridge's stdin closes
    /// (it exits; the apps stay in wprsd).
    pub(crate) fn request_stop(&self) {
        self.update(Runtime::request_stop);
    }

    /// The runtime once `ready` holds, or after `wait` (then `ready` may
    /// still be false).
    fn wait_until(
        &self,
        wait: Duration,
        ready: impl Fn(&Runtime) -> bool,
    ) -> std::sync::MutexGuard<'_, Runtime> {
        let runtime = relock(&self.runtime);
        self.changed
            .wait_timeout_while(runtime, wait, |r| !ready(r))
            .map_or_else(|poisoned| poisoned.into_inner().0, |(guard, _)| guard)
    }

    /// Waits up to `wait` for a stop request; returns whether one came.
    pub(crate) fn wait_stop(&self, wait: Duration) -> bool {
        self.wait_until(wait, |r| r.stop_requested).stop_requested
    }

    /// Everything, for a viewer that has nothing yet.
    fn snapshot(&self) -> RemoteAppsState {
        relock(&self.runtime).snapshot(0)
    }

    /// What changed after `since`, once there is a change, or `None` after
    /// `wait`.
    fn wait_newer(&self, since: u64, wait: Duration) -> Option<RemoteAppsState> {
        let runtime = self.wait_until(wait, |r| r.revision > since);
        (runtime.revision > since).then(|| runtime.snapshot(since))
    }
}

impl Shared {
    /// The live session of `project`, or a new one.
    pub(crate) fn remote_apps_connect(
        self: &Arc<Self>,
        project_id: &str,
    ) -> Result<RemoteAppsState> {
        let project = relock(&self.state)
            .remote_projects
            .iter()
            .find(|p| p.id == project_id)
            .cloned()
            .context("Unknown SSH project")?;
        let mut sessions = relock(&self.remote_apps);
        if let Some(live) = sessions
            .values()
            .find(|h| h.read(|r| r.live_for(project_id) && !r.stop_requested))
        {
            live.update(|r| r.last_viewed = Instant::now());
            return Ok(live.snapshot());
        }
        let id = terminator_core::id();
        let handle = Arc::new(Handle::new(Runtime::new(
            id.clone(),
            project.id.clone(),
            self.paths.runtime.clone(),
        )));
        // Ended sessions of this project are replaced.
        sessions.retain(|_, h| h.read(|r| r.project_id != project_id || r.status().live()));
        sessions.insert(id, Arc::clone(&handle));
        drop(sessions);
        let snapshot = handle.snapshot();
        let control = self.ssh_control(&project.ssh);
        let shared = Arc::downgrade(self);
        let detach_limit: worker::DetachLimit = Box::new(move || {
            let minutes = relock(&shared.upgrade()?.state)
                .settings
                .vnc_detached_timeout_minutes;
            crate::vnc_session::detach_limit(minutes)
        });
        worker::start(handle, project, control, detach_limit);
        Ok(snapshot)
    }

    pub(crate) fn remote_apps_disconnect(&self, session: &str) -> Result<()> {
        self.remote_apps_handle(session)?.request_stop();
        Ok(())
    }

    pub(crate) fn remote_apps_input(
        &self,
        session: &str,
        messages: Vec<ClientMessage>,
    ) -> Result<()> {
        ensure!(messages.len() <= MAX_INPUT, "Too many input messages");
        self.send_to_bridge(
            session,
            messages.into_iter().map(|m| (m, Vec::new())).collect(),
        )
    }

    /// Offers local clipboard text to the remote apps, then sends `then`
    /// (the paste key) after it on the same stream.
    pub(crate) fn remote_apps_clipboard(
        &self,
        session: &str,
        text: String,
        then: Vec<ClientMessage>,
    ) -> Result<()> {
        ensure!(text.len() <= MAX_CLIPBOARD, "Clipboard text is too large");
        ensure!(then.len() <= MAX_INPUT, "Too many input messages");
        let offer = ClientMessage::ClipboardOffer {
            mime_types: TEXT_MIME_TYPES.iter().map(|m| (*m).to_owned()).collect(),
        };
        let mut inputs = vec![(offer, text.into_bytes())];
        inputs.extend(then.into_iter().map(|m| (m, Vec::new())));
        self.send_to_bridge(session, inputs)
    }

    fn send_to_bridge(&self, session: &str, inputs: Vec<runtime::Input>) -> Result<()> {
        let handle = self.remote_apps_handle(session)?;
        let runtime = relock(&handle.runtime);
        let input = runtime.input().context("Remote apps are not connected")?;
        for item in inputs {
            input
                .send(item)
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
        handle.update(|r| r.last_viewed = Instant::now());
        let wait = Duration::from_millis(u64::from(wait_ms.min(MAX_WAIT_MS)));
        Ok(handle.wait_newer(since, wait))
    }

    fn remote_apps_handle(&self, session: &str) -> Result<Arc<Handle>> {
        relock(&self.remote_apps)
            .get(session)
            .cloned()
            .context(UNKNOWN_SESSION)
    }

    /// Whether a live remote-apps session uses `project`.
    pub(crate) fn remote_apps_live_for(&self, project: &str) -> bool {
        relock(&self.remote_apps)
            .values()
            .any(|h| h.read(|r| r.live_for(project)))
    }
}
