//! All ordinary GUI commands are submitted without waiting on the UI thread.
use super::jobs::{context, editor_ids, rejection};
#[cfg(feature = "test-support")]
use crate::nvim_rpc;
use crate::{
    After, Job, Tab, Update, clipboard, daemon_connection, diff, editor_close, external_editor,
    image_preview, installation, notify_test, player, search, services, workspace_ops,
};
use anyhow::{Context, Result};

use eframe::egui;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, mpsc::Sender},
    time::{Duration, Instant},
};
#[cfg(feature = "test-support")]
use terminator_core::CommandOptions;
use terminator_core::appearance::{AppearanceFile, config_path};
use terminator_core::{Paths, Request, Response, State, find_executable, sanitize_layout};
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
    async fn recover_service(&self, state: State, cancel: &CancellationToken) -> Result<bool> {
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
        let client = self.client();
        let mut updates = Vec::new();
        match job {
            Job::PrepareLayouts(generation, layouts) => {
                let layouts = self
                    .cpu()
                    .run(&cancel, move || {
                        layouts
                            .into_iter()
                            .map(|(project, layout)| {
                                let value = sanitize_layout(serde_json::to_value(layout)?);
                                let text = value.to_string();
                                Ok((project, value, text))
                            })
                            .collect::<Result<Vec<_>>>()
                    })
                    .await?;
                updates.push(Update::LayoutsPrepared(generation, layouts));
            }
            Job::SaveLayout(project, layout, text) => {
                let result = client
                    .rpc(Request::SaveLayout {
                        project: project.clone(),
                        layout,
                    })
                    .await
                    .and_then(|response| {
                        anyhow::ensure!(
                            matches!(response, Response::Ok),
                            "Layout save was not acknowledged"
                        );
                        Ok(())
                    })
                    .map_err(|e| format!("{e:#}"));
                updates.push(Update::LayoutSaved(project, text, result));
            }

            Job::Control(request, after) => match client.rpc(*request).await? {
                Response::Created(session) => {
                    if let Ok(Response::State(state)) = client.rpc(Request::Snapshot).await {
                        updates.push(Update::State(state));
                    }
                    match after {
                        After::Create(split) => updates.push(Update::Created(session, split, None)),
                        After::CreateAt(anchors, split) => {
                            updates.push(Update::Created(session, split, Some(anchors)));
                        }
                        After::Workspace(id, anchors) => {
                            updates.push(Update::WorkspaceCreated(session, id, anchors));
                        }
                        After::Strip => {
                            updates.push(Update::StripCreated(session, None, Vec::new()));
                        }
                        After::StripAt(anchors, split) => {
                            updates.push(Update::StripCreated(session, split, anchors));
                        }
                        After::Float(viewport, anchors, split) => {
                            updates.push(Update::FloatCreated(session, viewport, split, anchors));
                        }
                        _ => {}
                    }
                }
                Response::Text(text) => {
                    if let After::Text(key) = after {
                        updates.push(Update::Text(key, text));
                    }
                }
                _ => {}
            },
            Job::CloseIdle(target, generation, ids) => {
                let result = client
                    .rpc(Request::CloseIdleSessions {
                        generation,
                        sessions: ids.clone(),
                    })
                    .await
                    .and_then(|response| match response {
                        Response::IdleSessionsClosed(outcomes) => Ok(outcomes),
                        _ => anyhow::bail!("Unexpected idle-close response"),
                    })
                    .map_err(|e| format!("{e:#}"));
                updates.push(Update::IdleClosed(target, ids, result));
            }
            Job::CloseEditors(target, ids, mode, timeout) => {
                let result = editor_close::close_async(client, &ids, mode, timeout)
                    .await
                    .map_err(|e| format!("{e:#}"));
                updates.push(Update::EditorsClosed(target, ids, result));
            }
            Job::Preferences(prefs) => {
                let data = client.paths.data.clone();
                let result = client
                    .catalog
                    .run(&cancel, move || {
                        prefs.save(&data)?;
                        Ok(prefs)
                    })
                    .await
                    .map_err(|e| format!("Save UI preferences: {e:#}"));
                updates.push(Update::PreferencesSaved(Box::new(result)));
            }
            Job::SaveAppearance(theme, expected) => {
                let paths = client.paths.clone();
                let file = client
                    .catalog
                    .run(&cancel, move || {
                        AppearanceFile::save(&config_path(&paths)?, &theme, &expected)
                    })
                    .await?;
                updates.push(Update::Appearance(Box::new(file)));
            }
            Job::ExitSave(id, checkpoint) => {
                let result = async {
                    // Quit pauses polling. Recheck owners here as the daemon can
                    // have exited since the last accepted GUI snapshot.
                    let Response::State(state) = client.snapshot(None).await? else {
                        anyhow::bail!("Could not verify the session service before closing")
                    };
                    self.recover_service(*state, &cancel).await?;
                    let Response::State(state) = client.snapshot(None).await? else {
                        anyhow::bail!("Could not verify the recovered session service")
                    };
                    checkpoint.for_state(&state).save_async(client).await
                }
                .await;
                updates.push(Update::ExitSaved(id, result.map_err(|e| format!("{e:#}"))));
            }
            Job::ExitDrain(id, serial) => updates.push(Update::ExitDrained(id, serial)),
            Job::ResolveTarget(key, text, cwd) => {
                let target = self
                    .fs()
                    .run(&cancel, move || {
                        Ok(services::resolve_target(&text, &cwd).ok())
                    })
                    .await?;
                updates.push(Update::ResolvedTarget(key, target));
            }
            Job::OpenProject(path, generation) => {
                let path = self
                    .fs()
                    .run(&cancel, move || {
                        let path = path.canonicalize()?;
                        anyhow::ensure!(path.is_dir(), "Project must be a directory");
                        Ok(path)
                    })
                    .await?;
                let Response::State(state) = client.rpc(Request::Snapshot).await? else {
                    anyhow::bail!("Expected project inventory")
                };
                let lookup = path.clone();
                let (state, project) = self
                    .fs()
                    .run(&cancel, move || {
                        let project = services::project_for_directory(&state.projects, &lookup)
                            .map(|p| p.id.clone());
                        Ok((state, project))
                    })
                    .await?;
                let (state, project) = if let Some(project) = project {
                    (state, project)
                } else {
                    client
                        .rpc(Request::AddProject { path: path.clone() })
                        .await?;
                    let Response::State(state) = client.rpc(Request::Snapshot).await? else {
                        anyhow::bail!("Expected project inventory")
                    };
                    self.fs()
                        .run(&cancel, move || {
                            let project = services::project_for_directory(&state.projects, &path)
                                .context("Opened project missing from inventory")?
                                .id
                                .clone();
                            Ok((state, project))
                        })
                        .await?
                };
                updates.push(Update::OpenedProject(state, project, generation));
            }
            Job::MigrateAttention | Job::MigrateTypography => {
                let attention = matches!(job, Job::MigrateAttention);
                let Response::State(mut state) = client.rpc(Request::Snapshot).await? else {
                    anyhow::bail!("Expected settings snapshot")
                };
                if attention {
                    state.settings.notifications_side = true;
                    state
                        .settings
                        .keybindings
                        .entry("open_file".into())
                        .or_insert_with(|| "command+O".into());
                } else {
                    state.settings.font_size = 13.0;
                }
                anyhow::ensure!(
                    matches!(
                        client.rpc(Request::Settings(state.settings)).await?,
                        Response::Ok
                    ),
                    "Settings not acknowledged"
                );
                updates.push(if attention {
                    Update::AttentionMigrated(Ok(()))
                } else {
                    Update::TypographyMigrated
                });
                if let Response::State(state) = client.rpc(Request::Snapshot).await? {
                    updates.push(Update::State(state));
                }
            }
            Job::Diff(tab) => {
                if let Tab::Diff { cwd, path, staged } = &tab {
                    let result =
                        diff::document_async(self, cwd.clone(), path.clone(), *staged, &cancel)
                            .await
                            .map_err(|e| format!("{e:#}"));
                    updates.push(Update::Diff(tab.key(), result));
                }
            }
            Job::CreateWorktree(draft) => {
                anyhow::ensure!(
                    matches!(
                        client
                            .rpc(Request::WorktreeAdd {
                                project: draft.source,
                                path: draft.dest.clone(),
                                branch: Some(draft.branch.trim().to_owned())
                                    .filter(|s| !s.is_empty()),
                                start: draft.start.trim().to_owned()
                            })
                            .await?,
                        Response::Ok
                    ),
                    "Unexpected worktree creation response"
                );
                let canonical = self
                    .fs()
                    .run(&cancel, move || Ok(draft.dest.canonicalize()?))
                    .await?;
                let Response::State(state) = client.rpc(Request::Snapshot).await? else {
                    anyhow::bail!("Expected worktree snapshot")
                };
                let project = state
                    .projects
                    .iter()
                    .find(|p| p.path == canonical)
                    .context("Created worktree missing from inventory")?
                    .id
                    .clone();
                updates.push(Update::WorktreeCreated(state, project, draft.open_terminal));
            }
            Job::HookStatus => {
                let status = client
                    .catalog
                    .run(&cancel, move || {
                        let home = std::env::var_os("HOME")
                            .map(PathBuf::from)
                            .unwrap_or_default();
                        Ok(terminator_integrations::AGENTS
                            .iter()
                            .map(|kind| {
                                (
                                    (*kind).to_string(),
                                    terminator_integrations::installed(&home, kind),
                                )
                            })
                            .collect())
                    })
                    .await?;
                updates.push(Update::HookStatus(status));
            }
            Job::PasteClipboard(session) => {
                if let Some(text) = self.0.platform.run(&cancel, clipboard::read_paste).await? {
                    updates.push(Update::ClipboardPaste(session, text));
                }
            }
            Job::Browser(url) => {
                self.0
                    .platform
                    .run(&cancel, move || {
                        open::that(url)?;
                        Ok(())
                    })
                    .await?;
            }
            Job::Install(kind, remove) => {
                let paths = client.paths.clone();
                let message = client
                    .catalog
                    .run(&cancel, move || {
                        anyhow::ensure!(
                            remove || find_executable(&kind).is_some(),
                            "Install the {kind} CLI before configuring its hooks"
                        );
                        let home = std::env::var_os("HOME").context("No home directory")?;
                        let helper = terminator_core::sibling_exe(
                            &std::env::current_exe()?,
                            "terminator-hook",
                        );
                        let path = terminator_integrations::install_at(
                            Path::new(&home),
                            &kind,
                            &helper,
                            remove,
                            &paths,
                        )?;
                        Ok(format!(
                            "{} hooks: {}",
                            if remove { "Removed" } else { "Installed" },
                            path.display()
                        ))
                    })
                    .await?;
                updates.push(Update::Info(message));
            }
            Job::TestNtfy { channel, machine } => {
                let delivered = self
                    .0
                    .platform
                    .run(&cancel, move || {
                        notify_test::send(notify_test::ENDPOINT, &channel, &machine)
                    })
                    .await;
                match delivered {
                    Ok(()) => updates.push(Update::Info(
                        "ntfy test posted. If your phone stayed silent, open the channel in ntfy to subscribe."
                            .into(),
                    )),
                    Err(error) => updates.push(Update::Error(format!(
                        "ntfy test failed: {error:#}"
                    ))),
                }
            }
            Job::External(path) => {
                let (program, args) = if image_preview::supported(&path) {
                    external_editor::image_opener()
                } else {
                    let Response::State(state) = client.rpc(Request::Snapshot).await? else {
                        anyhow::bail!("Expected editor settings snapshot")
                    };
                    (state.settings.external_editor, state.settings.external_args)
                };
                self.launch_external(path, program, args, false, &cancel)
                    .await?;
            }
            Job::TestExternal(path, program, args) => {
                self.launch_external(path, program, args, true, &cancel)
                    .await?;
            }
            Job::RepairInstallation(generation, checkpoint) => {
                let result = async {
                    checkpoint.save_async(client).await?;
                    let paths = client.paths.clone();
                    self.0
                        .platform
                        .run(&cancel, move || {
                            daemon_connection::repair(
                                &paths,
                                &std::env::current_exe()?,
                                &generation,
                            )
                        })
                        .await
                }
                .await
                .map_err(|e| format!("{e:#}"));
                let mut result = result;
                if let Ok(state) = &mut result {
                    client.observe(state);
                }
                updates.push(Update::InstallationRepaired(result));
            }
            Job::RestartSessionService(inventory) => {
                let paths = client.paths.clone();
                let watched = self
                    .0
                    .platform
                    .run(&cancel, move || {
                        installation::begin_restart(
                            installation::restart_invocation(&std::env::current_exe()?, &paths)?,
                            inventory,
                        )
                    })
                    .await?;
                let service = self.clone();
                let cancel = CancellationToken::new();
                let token = cancel.clone();
                let mut context = OperationContext::new(
                    "platform",
                    "restart-observer".into(),
                    Policy::ServiceLifetime,
                );
                context.deadline = None;
                self.handle().submit(context, cancel, async move {
                    use tokio::io::AsyncReadExt;
                    let mut completion = watched.completion.into_tokio()?;
                    let mut bytes = [0; 256];
                    loop {
                        tokio::select! {
                            () = token.cancelled() => return Ok(Vec::new()),
                            n = completion.read(&mut bytes) => if n? == 0 { break; }
                        }
                    }
                    let message = service
                        .fs()
                        .run(&token, move || {
                            Ok(installation::restart_message(
                                watched.log_path,
                                watched.log_start,
                            ))
                        })
                        .await?;
                    Ok(vec![Update::RestartFinished(message)])
                })?;
            }
            Job::Workspace(root, op) => {
                let reply = op.clone();
                let result = self
                    .fs()
                    .run(&cancel, move || workspace_ops::perform(&root, op))
                    .await
                    .map_err(|error| format!("{error:#}"));
                updates.push(Update::Workspace(reply, result));
            }
            Job::Search {
                id,
                root,
                query,
                show_ignored,
            } => {
                let result = self
                    .fs()
                    .run(&cancel, move || {
                        Ok(search::run(&root, &query, show_ignored))
                    })
                    .await;
                let result = match result {
                    Ok(inner) => inner,
                    Err(error) => Err(format!("{error:#}")),
                };
                updates.push(Update::Search(id, result));
            }
            Job::StartSessionService => {
                let paths = client.paths.clone();
                let result = self
                    .0
                    .platform
                    .run(&cancel, move || {
                        daemon_connection::ensure_running(&paths, &std::env::current_exe()?)
                    })
                    .await
                    .map_err(|error| format!("{error:#}"));
                updates.push(Update::ServiceStarted(result));
            }
        }
        Ok(updates)
    }
    async fn launch_external(
        &self,
        path: PathBuf,
        program: String,
        args: Vec<String>,
        test: bool,
        _cancel: &CancellationToken,
    ) -> Result<()> {
        external_editor::launch_supervised(self.clone(), program, args, path).await?;
        if test {
            self.emit(Update::Info("External editor launched with draft settings. Check the selected file in the editor.".into())).await?;
        }
        Ok(())
    }
}
