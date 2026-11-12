//! Background snapshot, directory, activation, and appearance polling.
use super::{Services, project_directories};
use crate::{Update, installation};
use anyhow::Result;
use std::{
    fs,
    ops::ControlFlow,
    path::PathBuf,
    time::{Duration, Instant},
};
use terminator_core::appearance::{AppearanceFile, config_path};
use terminator_core::async_client::AsyncClientError;
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use terminator_core::generations::Health;
use terminator_core::{Response, SnapshotHint, State};
use tokio::sync::oneshot;

/// Project availability: (id, path, is an available directory).
type Directories = Vec<(String, PathBuf, bool)>;

/// Project paths may sit on a hung network mount, so each stat runs on its
/// own thread and is polled without waiting. A stuck stat must not occupy
/// the catalog worker that routes every daemon request, nor stall snapshot
/// polling. At most one probe runs; a hung one blocks only later probes.
#[derive(Default)]
struct DirectoryProbe(Option<oneshot::Receiver<Directories>>);

impl DirectoryProbe {
    fn pending(&self) -> bool {
        self.0.is_some()
    }

    /// The finished probe's result, without blocking.
    fn finished(&mut self) -> Option<Directories> {
        let result = self.0.as_mut()?.try_recv();
        match result {
            Ok(directories) => {
                self.0 = None;
                Some(directories)
            }
            Err(oneshot::error::TryRecvError::Empty) => None,
            Err(oneshot::error::TryRecvError::Closed) => {
                self.0 = None;
                None
            }
        }
    }

    fn start(&mut self, probe: impl FnOnce() -> Directories + Send + 'static) -> Result<()> {
        let (sender, receiver) = oneshot::channel();
        std::thread::Builder::new()
            .name("gui-project-stat".into())
            .spawn(move || {
                let _ = sender.send(probe());
            })?;
        self.0 = Some(receiver);
        Ok(())
    }
}

/// One snapshot poll reply.
type SnapshotResult = std::result::Result<Response, AsyncClientError>;

/// Loop state carried between snapshot polls.
struct SnapshotPoller {
    service: Services,
    token: CancellationToken,
    events: tokio::sync::mpsc::Sender<Update>,
    revision: Option<SnapshotHint>,
    recovery_check: Instant,
    unavailable_owners: Vec<Health>,
    projects: Vec<(String, PathBuf)>,
    previous_directories: Vec<(String, PathBuf, bool)>,
    directory_check: Instant,
    directory_probe: DirectoryProbe,
    appearance_source: Option<String>,
    config_check: Instant,
}

impl Services {
    pub(super) fn start_polling(&self, events: tokio::sync::mpsc::Sender<Update>) -> Result<()> {
        let cancel = CancellationToken::new();
        let poller = SnapshotPoller::new(self.clone(), cancel.clone(), events);
        let mut context =
            OperationContext::new("snapshot", "inventory".into(), Policy::ServiceLifetime);
        context.deadline = None;
        self.0.handle.submit(context, cancel, poller.run())?;
        Ok(())
    }
}

impl SnapshotPoller {
    fn new(
        service: Services,
        token: CancellationToken,
        events: tokio::sync::mpsc::Sender<Update>,
    ) -> Self {
        Self {
            service,
            token,
            events,
            revision: None,
            recovery_check: Instant::now()
                .checked_sub(Duration::from_secs(5))
                .unwrap_or_else(Instant::now),
            unavailable_owners: Vec::new(),
            projects: Vec::new(),
            previous_directories: Vec::new(),
            directory_check: Instant::now(),
            directory_probe: DirectoryProbe::default(),
            appearance_source: None,
            config_check: Instant::now()
                .checked_sub(Duration::from_secs(3))
                .unwrap_or_else(Instant::now),
        }
    }

    async fn run(mut self) -> Result<Vec<Update>> {
        loop {
            tokio::select! {
                () = self.token.cancelled() => break,
                () = tokio::time::sleep(Duration::from_millis(100)) => {}
            }
            if self.service.reads_paused() {
                continue;
            }
            let result = tokio::select! {
                () = self.token.cancelled() => break,
                result = self.service.client().snapshot(self.revision.clone()) => result,
            };
            self.note_snapshot(&result);
            if self.recover_unavailable(&result).await?.is_break() {
                continue;
            }
            let update = self.snapshot_update(result).await?;
            let update = self.store_state(update);
            if self.send_update(update).await.is_break()
                || self.check_directories().await?.is_break()
                || self.check_activation().await?.is_break()
                || self.check_appearance().await?.is_break()
            {
                break;
            }
        }
        Ok(Vec::new())
    }

    fn note_snapshot(&mut self, result: &SnapshotResult) {
        if matches!(result, Ok(Response::State(_) | Response::Unchanged)) {
            *self
                .service
                .0
                .snapshot_received
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
        }
        if let Ok(Response::State(state)) = result {
            self.service.set_capabilities(&state.capabilities);
            self.unavailable_owners = state
                .generations
                .iter()
                .filter(|health| health.error.is_some())
                .cloned()
                .collect();
        }
    }

    /// A missing auth file can produce unchanged fallback snapshots even
    /// after its daemon later exits. Recheck process death on the timer,
    /// including unchanged replies. Breaks when the poll should restart.
    async fn recover_unavailable(&mut self, result: &SnapshotResult) -> Result<ControlFlow<()>> {
        if self.recovery_check.elapsed() < Duration::from_secs(5)
            || self.unavailable_owners.is_empty()
            || !matches!(result, Ok(Response::State(_) | Response::Unchanged))
        {
            return Ok(ControlFlow::Continue(()));
        }
        self.recovery_check = Instant::now();
        let state = State {
            generations: self.unavailable_owners.clone(),
            ..State::default()
        };
        match self.service.recover_service(state, &self.token).await {
            Ok(true) => {
                self.revision = None;
                return Ok(ControlFlow::Break(()));
            }
            Ok(false) => {}
            Err(error) => {
                self.service
                    .emit(Update::Error(format!(
                        "Session service recovery failed: {error:#}"
                    )))
                    .await?;
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    async fn snapshot_update(&mut self, result: SnapshotResult) -> Result<Option<Update>> {
        let update = match result {
            Ok(Response::State(state)) => {
                let paths = self.service.client().paths.clone();
                let state = self
                    .service
                    .client()
                    .catalog
                    .run(&self.token, move || {
                        let result =
                            installation::restart_result(&paths, &state, &std::env::current_exe()?);
                        anyhow::Ok((state, result))
                    })
                    .await?;
                if let Err(error) = state.1 {
                    self.service
                        .emit(Update::Error(format!("{error:#}")))
                        .await?;
                }
                self.revision = Some(state.0.snapshot_hint());
                Some(Update::State(state.0))
            }
            Ok(_) => None,
            Err(error) => {
                self.revision = None;
                Some(Update::Error(format!(
                    "Reconnecting: {error}",
                    error = terminator_core::ErrorChain(&error)
                )))
            }
        };
        Ok(update)
    }

    /// Stores state snapshots for the GUI to take; other updates pass through.
    fn store_state(&mut self, update: Option<Update>) -> Option<Update> {
        match update.filter(|_| !self.service.reads_paused()) {
            Some(Update::State(state)) => {
                self.projects = state
                    .projects
                    .iter()
                    .map(|p| (p.id.clone(), p.path.clone()))
                    .collect();
                let previous = self
                    .service
                    .0
                    .snapshot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .replace(state);
                drop(previous);
                self.service.0.ctx.request_repaint();
                None
            }
            other => other,
        }
    }

    async fn send_update(&self, update: Option<Update>) -> ControlFlow<()> {
        let Some(update) = update else {
            return ControlFlow::Continue(());
        };
        tokio::select! {
            () = self.token.cancelled() => return ControlFlow::Break(()),
            result = self.events.send(update) => {
                if result.is_err() {
                    return ControlFlow::Break(());
                }
            }
        }
        self.service.0.ctx.request_repaint();
        ControlFlow::Continue(())
    }

    async fn emit_event(&self, update: Update) -> ControlFlow<()> {
        if self.events.send(update).await.is_err() {
            return ControlFlow::Break(());
        }
        self.service.0.ctx.request_repaint();
        ControlFlow::Continue(())
    }

    async fn check_directories(&mut self) -> Result<ControlFlow<()>> {
        let Some(directories) = self.directory_probe.finished() else {
            if !self.directory_probe.pending()
                && self.directory_check.elapsed() >= Duration::from_secs(1)
            {
                self.directory_check = Instant::now();
                let paths = self.projects.clone();
                self.directory_probe
                    .start(move || project_directories(paths))?;
            }
            return Ok(ControlFlow::Continue(()));
        };
        if directories == self.previous_directories {
            return Ok(ControlFlow::Continue(()));
        }
        self.previous_directories.clone_from(&directories);
        Ok(self
            .emit_event(Update::ProjectDirectories(directories))
            .await)
    }

    /// Catalog/persistence has its own worker, separate from bulk reads.
    async fn check_activation(&self) -> Result<ControlFlow<()>> {
        let paths = self.service.client().paths.clone();
        let activation = self
            .service
            .client()
            .catalog
            .run(&self.token, move || {
                let file = paths.runtime.join("activation");
                let notice = fs::read_to_string(&file).ok();
                if notice.is_some() {
                    let _ = fs::remove_file(file);
                }
                anyhow::Ok(notice)
            })
            .await?;
        let Some(notice) = activation else {
            return Ok(ControlFlow::Continue(()));
        };
        Ok(self.emit_event(Update::Activation(notice)).await)
    }

    async fn check_appearance(&mut self) -> Result<ControlFlow<()>> {
        if self.config_check.elapsed() < Duration::from_secs(1) {
            return Ok(ControlFlow::Continue(()));
        }
        self.config_check = Instant::now();
        let paths = self.service.client().paths.clone();
        let previous = self.appearance_source.clone();
        let changed = self
            .service
            .client()
            .catalog
            .run(&self.token, move || {
                let path = config_path(&paths)?;
                let source = fs::read_to_string(&path).unwrap_or_default();
                if previous.as_ref() == Some(&source) {
                    return Ok(None);
                }
                anyhow::Ok(Some((source, AppearanceFile::load(&path)?)))
            })
            .await;
        let update = match changed {
            Ok(Some((source, file))) => {
                self.appearance_source = Some(source);
                Update::Appearance(Box::new(file))
            }
            Ok(None) => return Ok(ControlFlow::Continue(())),
            Err(error) => Update::Error(format!("Appearance: {error:#}")),
        };
        Ok(self.emit_event(update).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hung_project_stat_does_not_block_the_poller() {
        let mut probe = DirectoryProbe::default();
        let (release, hung) = std::sync::mpsc::channel::<()>();
        probe
            .start(move || {
                let _ = hung.recv();
                vec![("a".into(), "/a".into(), true)]
            })
            .unwrap();
        assert!(probe.pending());
        assert_eq!(probe.finished(), None);
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let directories = loop {
            if let Some(directories) = probe.finished() {
                break directories;
            }
            assert!(Instant::now() < deadline, "probe never finished");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(directories, vec![("a".into(), "/a".into(), true)]);
        assert!(!probe.pending());
    }
}
