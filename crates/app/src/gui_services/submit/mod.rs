//! All ordinary GUI commands are submitted without waiting on the UI thread.
mod execute;
mod git_jobs;
mod local_jobs;
mod polling;
mod remote_jobs;

use execute::MISROUTED;

use super::jobs::JobRouting;
#[cfg(feature = "test-support")]
use crate::nvim_rpc;
use crate::{Job, Update, daemon_connection, player};
use anyhow::Result;

use eframe::egui;
#[cfg(feature = "test-support")]
use std::time::Duration;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, mpsc::Sender},
    time::Instant,
};
#[cfg(feature = "test-support")]
use terminator_core::CommandOptions;
use terminator_core::{Paths, State};
use terminator_core::{
    async_client::Client,
    async_process::Processes,
    async_service::{CancellationToken, Handle, NativePool, OperationContext, Policy, Supervisor},
};

pub(crate) fn project_directories(
    projects: Vec<(String, PathBuf)>,
) -> Vec<(String, PathBuf, bool)> {
    projects
        .into_iter()
        .map(|(id, path)| {
            let available = match fs::metadata(&path) {
                Ok(metadata) => metadata.is_dir(),
                Err(error) => !matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ),
            };
            (id, path, available)
        })
        .collect()
}
#[derive(Clone)]
pub struct Services(Arc<Inner>);
struct Inner {
    pub handle: Handle<Vec<Update>, anyhow::Error>,
    pub client: Client,
    pub fs: NativePool,
    pub platform: NativePool,
    pub processes: Processes,
    legacy_updates: Sender<Update>,
    events: tokio::sync::mpsc::Sender<Update>,
    ctx: egui::Context,
    paused: std::sync::atomic::AtomicBool,
    snapshot_received: std::sync::Mutex<Option<Instant>>,
    editors: std::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
    snapshot: Arc<std::sync::Mutex<Option<Box<State>>>>,
}
pub struct Owner {
    pub supervisor: Supervisor<Vec<Update>, anyhow::Error>,
    pub events: tokio::sync::mpsc::Receiver<Update>,
    pub snapshot: Arc<std::sync::Mutex<Option<Box<State>>>>,
}
/// Narrow background-decode handle; see [`Services::submit`].
#[derive(Clone)]
pub struct Submit {
    handle: Handle<Vec<Update>, anyhow::Error>,
    fs: NativePool,
    cpu: NativePool,
}
impl Submit {
    pub fn handle(&self) -> &Handle<Vec<Update>, anyhow::Error> {
        &self.handle
    }
    pub fn fs(&self) -> &NativePool {
        &self.fs
    }
    pub fn cpu(&self) -> &NativePool {
        &self.cpu
    }
}
impl Services {
    pub fn new(paths: Paths, ctx: egui::Context, updates: Sender<Update>) -> Result<(Self, Owner)> {
        let wake = ctx.clone();
        let supervisor = Supervisor::new(move || wake.request_repaint())?;
        let catalog = NativePool::new("gui-catalog", 1)?;
        let cpu = NativePool::new("gui-cpu", 2)?;
        let fs = NativePool::new("gui-files", 2)?;
        let platform = NativePool::new("gui-platform", 1)?;
        let cancellation = CancellationToken::new();
        let (processes, actor) = Processes::new(cancellation.clone());
        let mut context =
            OperationContext::new("processes", "children".into(), Policy::ServiceLifetime);
        context.deadline = None;
        supervisor
            .handle
            .submit(context, cancellation, async move {
                actor.await?;
                Ok(Vec::new())
            })?;
        let (events, incoming) = tokio::sync::mpsc::channel(32);
        let snapshot = Arc::new(std::sync::Mutex::new(None));
        let service = Self(Arc::new(Inner {
            handle: supervisor.handle.clone(),
            client: Client::new(paths, catalog, cpu),
            fs,
            platform,
            processes,
            legacy_updates: updates,
            events: events.clone(),
            ctx,
            paused: std::sync::atomic::AtomicBool::default(),
            snapshot_received: std::sync::Mutex::default(),
            editors: std::sync::Mutex::default(),
            snapshot: Arc::clone(&snapshot),
        }));
        let cpu = service.cpu().clone();
        supervisor.handle.submit(
            OperationContext::new("radio", "catalog".into(), Policy::ReplaceableRead),
            CancellationToken::new(),
            async move {
                let catalog = cpu
                    .run(&CancellationToken::new(), || {
                        let catalog = Arc::new(player::radio::catalog().to_vec());
                        player::radio::categories();
                        anyhow::Ok(catalog)
                    })
                    .await?;
                Ok(vec![Update::RadioCatalog(catalog)])
            },
        )?;
        if !cfg!(test) {
            service.start_polling(events)?;
        }
        Ok((
            service,
            Owner {
                supervisor,
                events: incoming,
                snapshot,
            },
        ))
    }
    pub fn presence_fresh(&self) -> Option<bool> {
        self.0
            .snapshot_received
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .map(|received| {
                received.elapsed().as_secs() <= terminator_core::agents::STALE_AFTER_SECS
            })
    }
    pub fn client(&self) -> &Client {
        &self.0.client
    }
    pub fn fs(&self) -> &NativePool {
        &self.0.fs
    }
    pub fn cpu(&self) -> &NativePool {
        &self.0.client.cpu
    }
    pub(super) fn platform(&self) -> &NativePool {
        &self.0.platform
    }
    pub fn processes(&self) -> &Processes {
        &self.0.processes
    }
    pub fn handle(&self) -> &Handle<Vec<Update>, anyhow::Error> {
        &self.0.handle
    }
    /// Narrow background-decode handle: job submission plus the filesystem
    /// and CPU pools, without the daemon client, process pool, or snapshots.
    pub fn submit(&self) -> Submit {
        Submit {
            handle: self.0.handle.clone(),
            fs: self.0.fs.clone(),
            cpu: self.0.client.cpu.clone(),
        }
    }
    pub async fn emit(&self, update: Update) -> Result<()> {
        self.0
            .events
            .send(update)
            .await
            .map_err(|_| anyhow::anyhow!("GUI disconnected"))?;
        self.0.ctx.request_repaint();
        Ok(())
    }
    pub fn pause_reads(&self, paused: bool) {
        self.0
            .paused
            .store(paused, std::sync::atomic::Ordering::Release);
        if paused {
            self.handle().cancel_reads();
        }
    }
    pub fn reads_paused(&self) -> bool {
        self.0.paused.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(super) async fn recover_service(
        &self,
        state: State,
        cancel: &CancellationToken,
    ) -> Result<bool> {
        if !state
            .generations
            .iter()
            .any(|health| health.error.is_some())
        {
            return Ok(false);
        }
        let paths = self.client().paths.clone();
        self.0
            .platform
            .run(cancel, move || {
                daemon_connection::recover_unavailable(&paths, &state, &std::env::current_exe()?)
            })
            .await
    }
    pub async fn emit_read(&self, update: Update) -> Result<()> {
        if !self.reads_paused() {
            self.emit(update).await?;
        }
        Ok(())
    }
    pub fn diagnostics(&self) -> serde_json::Value {
        let tasks = self.handle().diagnostics();
        serde_json::json!({"active":tasks.active,"queued":tasks.queued,"actors":tasks.actors,"required":tasks.required,
        "outstanding":tasks.outstanding,"completed":tasks.completed,"panics":tasks.panics,"oldest_ms":tasks.oldest_ms,
        "children":self.processes().children(),"native":{
            "files":{"running":self.fs().occupancy(),"queued":self.fs().queued()},
            "cpu":{"running":self.cpu().occupancy(),"queued":self.cpu().queued()},
            "catalog":{"running":self.client().catalog.occupancy(),"queued":self.client().catalog.queued()},
            "platform":{"running":self.0.platform.occupancy(),"queued":self.0.platform.queued()}
        }})
    }
    #[cfg(feature = "test-support")]
    pub fn fixture_stalls(&self, socket: PathBuf) -> Result<()> {
        let service = self.clone();
        let token = CancellationToken::new();
        let cancel = token.clone();
        let mut context = OperationContext::new(
            "fixture",
            "stalled-services".into(),
            Policy::ServiceLifetime,
        );
        context.deadline = None;
        self.handle().submit(context, token, async move {
            let nvim = async {
                loop {
                    if let Ok(mut connection) = nvim_rpc::AsyncConnection::connect(
                        &socket,
                        Duration::from_millis(400),
                        service.cpu().clone(),
                    )
                    .await
                    {
                        let _ = connection
                            .call("nvim_get_mode", serde_json::json!([]), 4096)
                            .await;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            };
            let git = async {
                loop {
                    let mut command = std::process::Command::new("sh");
                    command.args(["-c", "sleep 10"]);
                    let _ = service
                        .processes()
                        .run(
                            command,
                            CommandOptions::default(),
                            Some("fixture-stalled-git".into()),
                        )
                        .await;
                }
            };
            tokio::select! { () = cancel.cancelled() => {}, _ = nvim => {}, _ = git => {} }
            Ok(Vec::new())
        })?;
        Ok(())
    }
    pub fn dialog(&self, future: impl std::future::Future<Output = Result<()>> + Send + 'static) {
        let context = OperationContext::new("platform", "picker".into(), Policy::OrderedMutation);
        let result = self
            .handle()
            .submit(context, CancellationToken::new(), async move {
                match future.await {
                    Ok(()) => Ok(Vec::new()),
                    Err(error) => Ok(vec![
                        Update::TestPickerClosed,
                        Update::Error(format!("File picker: {error:#}")),
                    ]),
                }
            });
        if let Err(error) = result {
            let _ = self.0.legacy_updates.send(Update::TestPickerClosed);
            let _ = self.0.legacy_updates.send(Update::Error(error.to_string()));
            self.0.ctx.request_repaint();
        }
    }
    pub fn idle(&self) -> bool {
        self.0.handle.diagnostics().required == 0
    }
    pub fn send(&self, job: Job) -> Result<(), ()> {
        let context = JobRouting::context(&job);
        let rejected = JobRouting::rejection(&job);
        let banner = context.policy != Policy::ReplaceableRead;
        let service = self.clone();
        let cancel = CancellationToken::new();
        let work_cancel = cancel.clone();
        match self.0.handle.submit(context, cancel, async move {
            service.execute(job, work_cancel).await
        }) {
            Ok(_) => Ok(()),
            Err(error) => {
                for update in rejected {
                    let _ = self.0.legacy_updates.send(update);
                }
                if banner {
                    let _ = self.0.legacy_updates.send(Update::Error(error.to_string()));
                }
                self.0.ctx.request_repaint();
                Err(())
            }
        }
    }
}
