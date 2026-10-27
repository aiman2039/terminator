//! All ordinary GUI commands are submitted without waiting on the UI thread.
use super::jobs::{context, editor_ids, rejection};
#[cfg(feature = "test-support")]
use crate::nvim_rpc;
use crate::{Job, Update, daemon_connection, installation, player};
use anyhow::Result;

use eframe::egui;
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, mpsc::Sender},
    time::{Duration, Instant},
};
#[cfg(feature = "test-support")]
use terminator_core::CommandOptions;
use terminator_core::appearance::{AppearanceFile, config_path};
use terminator_core::{Paths, Response, State};
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
    pub handle: Handle<Vec<Update>>,
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
    pub supervisor: Supervisor<Vec<Update>>,
    pub events: tokio::sync::mpsc::Receiver<Update>,
    pub snapshot: Arc<std::sync::Mutex<Option<Box<State>>>>,
}
/// Narrow background-decode handle; see [`Services::submit`].
#[derive(Clone)]
pub struct Submit {
    handle: Handle<Vec<Update>>,
    fs: NativePool,
    cpu: NativePool,
}
impl Submit {
    pub fn handle(&self) -> &Handle<Vec<Update>> {
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
                        Ok(catalog)
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
    pub fn handle(&self) -> &Handle<Vec<Update>> {
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
        let context = context(&job);
        let rejected = rejection(&job);
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
    fn start_polling(&self, events: tokio::sync::mpsc::Sender<Update>) -> Result<()> {
        let service = self.clone();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let mut context =
            OperationContext::new("snapshot", "inventory".into(), Policy::ServiceLifetime);
        context.deadline = None;
        self.0.handle.submit(context, cancel, async move {
            let mut revision = None;
            let mut recovery_check = Instant::now()
                .checked_sub(Duration::from_secs(5))
                .unwrap_or_else(Instant::now);
            let mut unavailable_owners = Vec::new();
            let mut projects = Vec::new();
            let mut previous_directories = Vec::new();
            let mut directory_check = Instant::now();
            let mut appearance_source = None;
            let mut config_check = Instant::now()
                .checked_sub(Duration::from_secs(3))
                .unwrap_or_else(Instant::now);
            loop {
                tokio::select! {
                    () = token.cancelled() => break,
                    () = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
                if service.reads_paused() { continue; }
                let result = tokio::select! {
                    () = token.cancelled() => break,
                    result = service.client().snapshot(revision.clone()) => result,
                };
                if matches!(&result, Ok(Response::State(_) | Response::Unchanged)) {
                    *service
                        .0
                        .snapshot_received
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some(Instant::now());
                }
                if let Ok(Response::State(state)) = &result {
                    unavailable_owners = state.generations.iter()
                        .filter(|health| health.error.is_some()).cloned().collect();
                }
                // A missing auth file can produce unchanged fallback snapshots
                // even after its daemon later exits. Recheck process death on
                // the timer, including unchanged replies.
                if recovery_check.elapsed() >= Duration::from_secs(5)
                    && !unavailable_owners.is_empty()
                    && matches!(&result, Ok(Response::State(_) | Response::Unchanged))
                {
                    recovery_check = Instant::now();
                    let state = State { generations: unavailable_owners.clone(), ..State::default() };
                    match service.recover_service(state, &token).await {
                        Ok(true) => { revision = None; continue; }
                        Ok(false) => {}
                        Err(error) => {
                            service.emit(Update::Error(format!("Session service recovery failed: {error:#}"))).await?;
                        }
                    }
                }
                let update = match result {
                    Ok(Response::State(state)) => {
                        let paths = service.client().paths.clone();
                        let state = service.client().catalog.run(&token, move || {
                            let result = installation::restart_result(&paths, &state, &std::env::current_exe()?);
                            Ok((state, result))
                        }).await?;
                        if let Err(error) = state.1 { service.emit(Update::Error(format!("{error:#}"))).await?; }
                        revision = Some(state.0.snapshot_hint()); Some(Update::State(state.0))
                    }
                    Ok(_) => None,
                    Err(error) => { revision = None; Some(Update::Error(format!("Reconnecting: {error:#}"))) }
                };
                let update = match update.filter(|_| !service.reads_paused()) {
                    Some(Update::State(state)) => {
                        projects = state.projects.iter().map(|p| (p.id.clone(), p.path.clone())).collect();
                        let previous = service
                            .0
                            .snapshot
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .replace(state);
                        drop(previous);
                        service.0.ctx.request_repaint();
                        None
                    }
                    other => other,
                };
                if let Some(update) = update {
                    tokio::select! { () = token.cancelled() => break, result = events.send(update) => { if result.is_err() { break; } } }
                    service.0.ctx.request_repaint();
                }
                if directory_check.elapsed() >= Duration::from_secs(1) {
                    directory_check = Instant::now();
                    let paths = projects.clone();
                    let directories = service.client().catalog.run(&token, move || Ok(project_directories(paths))).await?;
                    if directories != previous_directories {
                        previous_directories = directories.clone();
                        if events.send(Update::ProjectDirectories(directories)).await.is_err() { break; }
                        service.0.ctx.request_repaint();
                    }
                }
                // Catalog/persistence has its own worker, separate from bulk reads.
                let paths = service.client().paths.clone();
                let activation = service.client().catalog.run(&token, move || {
                    let file = paths.runtime.join("activation");
                    let notice = fs::read_to_string(&file).ok();
                    if notice.is_some() { let _ = fs::remove_file(file); }
                    Ok(notice)
                }).await?;
                if let Some(notice) = activation { if events.send(Update::Activation(notice)).await.is_err() { break; } service.0.ctx.request_repaint(); }
                if config_check.elapsed() >= Duration::from_secs(1) {
                    config_check = Instant::now();
                    let paths = service.client().paths.clone();
                    let previous = appearance_source.clone();
                    let changed = service.client().catalog.run(&token, move || {
                        let path = config_path(&paths)?;
                        let source = fs::read_to_string(&path).unwrap_or_default();
                        if previous.as_ref() == Some(&source) { return Ok(None); }
                        Ok(Some((source, AppearanceFile::load(&path)?)))
                    }).await;
                    let update = match changed {
                        Ok(Some((source, file))) => { appearance_source = Some(source); Some(Update::Appearance(Box::new(file))) }
                        Ok(None) => None,
                        Err(error) => Some(Update::Error(format!("Appearance: {error:#}"))),
                    };
                    if let Some(update) = update { if events.send(update).await.is_err() { break; } service.0.ctx.request_repaint(); }
                }
            }
            Ok(Vec::new())
        })?;
        Ok(())
    }
    async fn execute(&self, job: Job, cancel: CancellationToken) -> Result<Vec<Update>> {
        let ids = editor_ids(&job);
        let locks: Vec<_> = {
            let mut editors = self
                .0
                .editors
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            editors.retain(|_, lock| lock.strong_count() != 0);
            ids.iter()
                .map(|id| {
                    let lock = editors
                        .get(id)
                        .and_then(std::sync::Weak::upgrade)
                        .unwrap_or_default();
                    editors.insert(id.clone(), Arc::downgrade(&lock));
                    lock
                })
                .collect()
        };
        let mut guards = Vec::new();
        for lock in locks {
            guards.push(lock.lock_owned().await);
        }
        if let Job::Control(request, _) = &job
            && let Some(updates) = self.reuse_review(request, &cancel).await?
        {
            return Ok(updates);
        }
        let mut updates = Vec::new();
        match job {
            Job::PrepareLayouts(generation, layouts) => {
                self.prepare_layouts(generation, layouts, cancel, &mut updates)
                    .await?
            }
            Job::SaveLayout(project, layout, text) => {
                self.save_layout(project, layout, text, &mut updates)
                    .await?
            }
            Job::Control(request, after) => self.control(request, after, &mut updates).await?,
            Job::CloseIdle(target, generation, ids) => {
                self.close_idle(target, generation, ids, &mut updates)
                    .await?
            }
            Job::CloseEditors(target, ids, mode, timeout) => {
                self.close_editors(target, ids, mode, timeout, &mut updates)
                    .await?
            }
            Job::Preferences(prefs) => self.save_preferences(prefs, cancel, &mut updates).await?,
            Job::SaveAppearance(theme, expected) => {
                self.save_appearance(theme, expected, cancel, &mut updates)
                    .await?
            }
            Job::ExitSave(id, checkpoint) => {
                self.exit_save(id, checkpoint, cancel, &mut updates).await?
            }
            Job::ExitDrain(id, serial) => updates.push(Update::ExitDrained(id, serial)),
            Job::ResolveTarget(key, text, cwd) => {
                self.resolve_target(key, text, cwd, cancel, &mut updates)
                    .await?
            }
            Job::OpenProject(path, generation) => {
                self.open_project(path, generation, cancel, &mut updates)
                    .await?
            }
            Job::MigrateAttention => self.migrate_settings(true, &mut updates).await?,
            Job::MigrateTypography => self.migrate_settings(false, &mut updates).await?,
            Job::DiffAction(tab, request) => {
                self.diff_action(tab, request, cancel, &mut updates).await?
            }
            Job::DiffContext(tab, view) => {
                self.diff_context(tab, view, cancel, &mut updates).await?
            }
            Job::DiffExpand(tab, doc, gap, part) => {
                self.diff_expand(tab, doc, (gap, part), cancel, &mut updates)
                    .await?
            }
            Job::Diff(tab) => self.diff(tab, cancel, &mut updates).await?,
            Job::CreateWorktree(draft) => self.create_worktree(draft, cancel, &mut updates).await?,
            Job::HookStatus => self.hook_status(cancel, &mut updates).await?,
            Job::PasteClipboard(session) => {
                self.paste_clipboard(session, cancel, &mut updates).await?
            }
            Job::Browser(url) => self.open_browser(url, cancel).await?,
            Job::Install(kind, remove) => self.install(kind, remove, cancel, &mut updates).await?,
            Job::TestNtfy { channel, machine } => {
                self.test_ntfy(channel, machine, cancel, &mut updates)
                    .await?
            }
            Job::External(path) => self.open_external(path, cancel).await?,
            Job::GitBase(path, generation) => {
                let base = self.git_base_text(&path).await;
                updates.push(Update::GitBase(path, generation, base));
            }
            Job::LinePreviews(generation, files) => {
                let lines = self
                    .fs()
                    .run(&cancel, move || {
                        Ok(crate::native_editor::read_line_previews(files))
                    })
                    .await
                    .unwrap_or_default();
                updates.push(Update::LinePreviews(generation, lines));
            }
            Job::TestExternal(path, program, args) => {
                self.test_external(path, program, args, cancel).await?
            }
            Job::RepairInstallation(generation, checkpoint) => {
                self.repair_installation(generation, checkpoint, cancel, &mut updates)
                    .await?
            }
            Job::RestartSessionService(inventory) => {
                self.restart_session_service(inventory, cancel).await?
            }
            Job::RemoteAdd {
                project,
                probe,
                generation,
            } => {
                self.remote_add(project, probe, generation, &mut updates)
                    .await?
            }
            Job::RemoteRead { project, operation } => {
                self.remote_read(project, operation, &mut updates).await?
            }
            Job::HostKey { ssh, keys } => self.host_key(ssh, keys, &mut updates).await?,
            Job::SshConfigHosts(generation) => {
                self.ssh_config_hosts(generation, cancel, &mut updates)
                    .await?
            }
            Job::Forwards(request) => self.forwards(request, &mut updates).await?,
            Job::SshControlStatus(targets) => updates.push(Update::SshControlStatus(
                crate::hosts_ui::check_controls(targets).await,
            )),
            Job::RemoteAsset {
                project,
                path,
                cache_root,
                after,
            } => {
                self.remote_asset(project, path, cache_root, after, cancel, &mut updates)
                    .await?
            }
            Job::RemoteTransfer { project, pick } => {
                self.remote_transfer(project, pick, cancel, &mut updates)
                    .await?
            }
            Job::RemoteTransferPoll { project, id } => {
                self.remote_transfer_poll(project, id, &mut updates).await?
            }
            Job::VncPickTransfer {
                session,
                direction,
                remote,
            } => {
                self.vnc_pick_transfer(session, direction, remote, cancel, &mut updates)
                    .await?
            }
            Job::VncTransferPoll { session, id } => {
                self.vnc_transfer_poll(session, id, &mut updates).await?
            }
            Job::VncConnectTab { tab_id, request } => {
                self.vnc_connect_tab(tab_id, request, &mut updates).await?
            }
            Job::VncFrame {
                session,
                since,
                regions,
            } => {
                self.vnc_frame(session, since, regions, cancel, &mut updates)
                    .await?
            }
            Job::Workspace(root, op) => self.workspace_op(root, op, cancel, &mut updates).await?,
            Job::Search {
                id,
                root,
                query,
                show_ignored,
            } => {
                self.search(id, root, query, show_ignored, cancel, &mut updates)
                    .await?
            }
            Job::GitView(tab) => self.git_view(tab, cancel, &mut updates).await?,
            Job::StartSessionService => self.start_session_service(cancel, &mut updates).await?,
        }
        Ok(updates)
    }
}
