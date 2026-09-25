//! All ordinary GUI commands are submitted without waiting on the UI thread.
use super::*;
use std::sync::Arc;
use terminator_core::{
    async_client::Client,
    async_process::Processes,
    async_service::{CancellationToken, Handle, NativePool, OperationContext, Policy, Supervisor},
};

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
    editors: std::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>,
    snapshot: Arc<std::sync::Mutex<Option<Box<State>>>>,
}
pub struct Owner {
    pub supervisor: Supervisor<Vec<Update>>,
    pub events: tokio::sync::mpsc::Receiver<Update>,
    pub snapshot: Arc<std::sync::Mutex<Option<Box<State>>>>,
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
            paused: Default::default(),
            editors: Default::default(),
            snapshot: snapshot.clone(),
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
            tokio::select! { _ = cancel.cancelled() => {}, _ = nvim => {}, _ = git => {} }
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
            let mut appearance_source = None;
            let mut config_check = Instant::now() - Duration::from_secs(3);
            loop {
                tokio::select! {
                    _ = token.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
                if service.reads_paused() { continue; }
                let result = tokio::select! {
                    _ = token.cancelled() => break,
                    result = service.client().snapshot(revision.clone()) => result,
                };
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
                        let previous = service.0.snapshot.lock().unwrap().replace(state);
                        drop(previous);
                        service.0.ctx.request_repaint();
                        None
                    }
                    other => other,
                };
                if let Some(update) = update {
                    tokio::select! { _ = token.cancelled() => break, result = events.send(update) => { if result.is_err() { break; } } }
                    service.0.ctx.request_repaint();
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
            let mut editors = self.0.editors.lock().unwrap();
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
                            updates.push(Update::Created(session, split, Some(anchors)))
                        }
                        After::Workspace(id, anchors) => {
                            updates.push(Update::WorkspaceCreated(session, id, anchors))
                        }
                        After::Strip => {
                            updates.push(Update::StripCreated(session, None, Vec::new()))
                        }
                        After::StripAt(anchors, split) => {
                            updates.push(Update::StripCreated(session, split, anchors))
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
                updates.push(Update::ExitSaved(
                    id,
                    checkpoint
                        .save_async(client)
                        .await
                        .map_err(|e| format!("{e:#}")),
                ));
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
                        let helper = std::env::current_exe()?.with_file_name("terminator-hook");
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
                    let mut completion = tokio::net::UnixStream::from_std(watched.completion)?;
                    let mut bytes = [0; 256];
                    loop {
                        tokio::select! {
                            _ = token.cancelled() => return Ok(Vec::new()),
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
fn editor_ids(job: &Job) -> Vec<String> {
    let mut ids = match job {
        Job::CloseEditors(_, ids, _, _) => ids.clone(),
        Job::Control(request, _) => match request.as_ref() {
            Request::EditorSave { session }
            | Request::EditorStatus { session }
            | Request::EditorCompare { session }
            | Request::Stop { session } => vec![session.clone()],
            _ => Vec::new(),
        },
        _ => Vec::new(),
    };
    ids.sort();
    ids.dedup();
    ids
}
fn rejection(job: &Job) -> Vec<Update> {
    let busy = || "Services are busy; the action was not accepted. Retry it.".to_string();
    match job {
        Job::SaveLayout(project, _, text) => vec![Update::LayoutSaved(
            project.clone(),
            text.clone(),
            Err(busy()),
        )],
        Job::CloseEditors(target, ids, _, _) => vec![Update::EditorsClosed(
            target.clone(),
            ids.clone(),
            Err(busy()),
        )],
        Job::CloseIdle(target, _, ids) => {
            vec![Update::IdleClosed(target.clone(), ids.clone(), Err(busy()))]
        }
        Job::Preferences(_) => vec![Update::PreferencesSaved(Box::new(Err(busy())))],
        Job::RepairInstallation(..) => vec![Update::InstallationRepaired(Err(busy()))],
        Job::RestartSessionService(_) => vec![Update::RestartFinished(busy())],
        Job::StartSessionService => vec![Update::ServiceStarted(Err(busy()))],
        Job::Diff(tab) => vec![Update::Diff(tab.key(), Err(busy()))],
        Job::ResolveTarget(key, _, _) => vec![Update::ResolvedTarget(key.clone(), None)],
        Job::ExitSave(id, _) => vec![Update::ExitSaved(*id, Err(busy()))],
        _ => Vec::new(),
    }
}

fn context(job: &Job) -> OperationContext {
    let (subsystem, resource, policy) = match job {
        Job::PrepareLayouts(_, _) => ("layout", "snapshot".into(), Policy::ReplaceableRead),
        Job::SaveLayout(project, _, _) => (
            "daemon",
            format!("layout:{project}"),
            Policy::OrderedMutation,
        ),
        Job::Control(request, _) => {
            let resource = match request.as_ref() {
                Request::EditorSave { session }
                | Request::EditorStatus { session }
                | Request::EditorCompare { session }
                | Request::Stop { session }
                | Request::Rename { session, .. }
                | Request::Focus { session } => format!("editor:{session}"),
                _ => "workspace".into(),
            };
            (
                "daemon",
                resource,
                if async_client::read_only(request) {
                    Policy::ReplaceableRead
                } else {
                    Policy::OrderedMutation
                },
            )
        }
        Job::CloseEditors(_, ids, _, _) => (
            "daemon",
            format!("editor:{}", ids.join(",")),
            Policy::OrderedMutation,
        ),
        Job::Diff(tab) => ("diff", tab.key(), Policy::ReplaceableRead),
        Job::ResolveTarget(key, _, _) => ("files", key.clone(), Policy::ReplaceableRead),
        Job::HookStatus => ("catalog", "hooks".into(), Policy::ReplaceableRead),
        Job::Preferences(_) => ("catalog", "preferences".into(), Policy::OrderedMutation),
        Job::SaveAppearance(..) => ("catalog", "appearance".into(), Policy::OrderedMutation),
        _ => ("daemon", "workspace".into(), Policy::OrderedMutation),
    };
    let mut context = OperationContext::new(subsystem, resource, policy);
    context.session = editor_ids(job).first().cloned();
    match job {
        Job::CloseEditors(editor_close::Target::Workspace(project, tab), _, _, _) => {
            context.project = Some(project.clone());
            context.tab = Some(tab.clone());
        }
        Job::Control(request, after) => {
            if let Request::Create { project, .. }
            | Request::CreateReview { project, .. }
            | Request::SaveLayout { project, .. }
            | Request::SelectProject { project } = request.as_ref()
            {
                context.project = Some(project.clone());
            }
            if let After::Workspace(tab, _) = after {
                context.tab = Some(tab.clone());
            }
        }
        Job::OpenProject(_, generation) => context.generation = *generation,
        _ => {}
    }
    context
}

#[derive(Clone)]
pub struct ImageJobs(pub Services);
impl ImageJobs {
    pub fn try_send(
        &self,
        (path, generation): (PathBuf, u64),
    ) -> Result<CancellationToken, async_service::Failure> {
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let service = self.0.clone();
        let mut context = OperationContext::new(
            "images",
            path.to_string_lossy().into_owned(),
            Policy::ReplaceableRead,
        );
        context.generation = generation;
        self.0
            .handle()
            .submit(context, cancel.clone(), async move {
                let result = image_preview::load(&service, path.clone(), &token)
                    .await
                    .map_err(|e| format!("{e:#}"));
                Ok(vec![Update::Image(path, generation, result)])
            })?;
        Ok(cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn stalled_radio_git_and_neovim_do_not_block_another_editor() {
        let directory = tempfile::Builder::new()
            .prefix("async-load-")
            .tempdir_in("/tmp")
            .unwrap();
        let (updates, _) = mpsc::channel();
        let (service, mut owner) = Services::new(
            Paths::at(directory.path().into()),
            egui::Context::default(),
            updates,
        )
        .unwrap();
        let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", http.local_addr().unwrap());
        let slow_path = directory.path().join("slow.nvim");
        let good_path = directory.path().join("good.nvim");
        let slow = tokio::net::UnixListener::bind(&slow_path).unwrap();
        let good = tokio::net::UnixListener::bind(&good_path).unwrap();
        let stop = CancellationToken::new();
        let (http_started, http_ready) = tokio::sync::oneshot::channel();
        let token = stop.clone();
        let http_server = tokio::spawn(async move {
            let (mut peer, _) = http.accept().await.unwrap();
            let mut bytes = [0; 4096];
            assert!(peer.read(&mut bytes).await.unwrap() > 0);
            let _ = http_started.send(());
            token.cancelled().await;
        });
        let (nvim_started, nvim_ready) = tokio::sync::oneshot::channel();
        let token = stop.clone();
        let slow_server = tokio::spawn(async move {
            let (mut peer, _) = slow.accept().await.unwrap();
            let mut bytes = [0; 4096];
            assert!(peer.read(&mut bytes).await.unwrap() > 0);
            let _ = nvim_started.send(());
            token.cancelled().await;
        });
        let good_server = tokio::spawn(async move {
            let (mut peer, _) = good.accept().await.unwrap();
            let mut bytes = [0; 4096];
            assert!(peer.read(&mut bytes).await.unwrap() > 0);
            peer.write_all(
                &rmp_serde::to_vec(&(
                    1,
                    1,
                    serde_json::Value::Null,
                    serde_json::json!({"blocking":false}),
                ))
                .unwrap(),
            )
            .await
            .unwrap();
        });
        let radio = CancellationToken::new();
        service
            .handle()
            .submit(
                OperationContext::new("fixture", "radio".into(), Policy::ReplaceableRead),
                radio.clone(),
                async move {
                    player::fixture_download(url).await?;
                    Ok(Vec::new())
                },
            )
            .unwrap();
        let nvim = CancellationToken::new();
        let cpu = service.cpu().clone();
        service
            .handle()
            .submit(
                OperationContext::new("fixture", "nvim".into(), Policy::ReplaceableRead),
                nvim.clone(),
                async move {
                    let mut connection =
                        nvim_rpc::AsyncConnection::connect(&slow_path, Duration::from_secs(3), cpu)
                            .await?;
                    connection
                        .call("nvim_get_mode", serde_json::json!([]), 4096)
                        .await?;
                    Ok(Vec::new())
                },
            )
            .unwrap();
        let git = CancellationToken::new();
        let processes = service.processes().clone();
        service
            .handle()
            .submit(
                OperationContext::new("fixture", "git".into(), Policy::ReplaceableRead),
                git.clone(),
                async move {
                    let mut command = std::process::Command::new("sh");
                    command.args(["-c", "sleep 10"]);
                    processes
                        .run(
                            command,
                            CommandOptions::default(),
                            Some("stalled-repo".into()),
                        )
                        .await?;
                    Ok(Vec::new())
                },
            )
            .unwrap();
        http_ready.await.unwrap();
        nvim_ready.await.unwrap();
        let cpu = service.cpu().clone();
        service
            .handle()
            .submit(
                OperationContext::new("fixture", "other-editor".into(), Policy::ReplaceableRead),
                CancellationToken::new(),
                async move {
                    let mut connection = nvim_rpc::AsyncConnection::connect(
                        &good_path,
                        Duration::from_millis(400),
                        cpu,
                    )
                    .await?;
                    let value = connection
                        .call("nvim_get_mode", serde_json::json!([]), 4096)
                        .await?;
                    anyhow::ensure!(
                        value["blocking"] == false,
                        "other editor returned invalid state"
                    );
                    Ok(vec![Update::Info("other editor progressed".into())])
                },
            )
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            assert!(
                Instant::now() < deadline,
                "Unrelated editor stalled behind radio/Git/Neovim"
            );
            if let Some(mut completion) = owner.supervisor.try_recv()
                && let Some(Ok(updates)) = completion.result.take()
                && updates
                    .iter()
                    .any(|u| matches!(u, Update::Info(s) if s == "other editor progressed"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        radio.cancel();
        nvim.cancel();
        git.cancel();
        stop.cancel();
        http_server.await.unwrap();
        slow_server.await.unwrap();
        good_server.await.unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            while owner.supervisor.try_recv().is_some() {}
            if service.processes().children() == 0 && service.handle().diagnostics().active == 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "Cancelled fixture work was not reaped"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[test]
    fn replaceable_read_admission_failure_does_not_emit_status_error() {
        let directory = tempfile::Builder::new()
            .prefix("busy-banner-")
            .tempdir_in("/tmp")
            .unwrap();
        let (updates, rx) = mpsc::channel();
        let (service, _owner) = Services::new(
            Paths::at(directory.path().into()),
            egui::Context::default(),
            updates,
        )
        .unwrap();
        service.handle().close_admission();
        while rx.try_recv().is_ok() {}
        assert!(
            service
                .send(Job::ResolveTarget(
                    "hover".into(),
                    "lib.rs".into(),
                    PathBuf::from("/tmp"),
                ))
                .is_err()
        );
        let received: Vec<_> = rx.try_iter().collect();
        assert!(
            received.iter().any(
                |update| matches!(update, Update::ResolvedTarget(key, None) if key == "hover")
            ),
            "rejected hover still resolves empty"
        );
        assert!(
            received
                .iter()
                .all(|update| !matches!(update, Update::Error(_))),
            "hover admission failure must not use the status banner"
        );
        assert!(
            service
                .send(Job::Preferences(UiPreferences::default()))
                .is_err()
        );
        assert!(
            rx.try_iter().any(
                |update| matches!(update, Update::Error(message) if message == "Services are closing")
            ),
            "mutations still report admission failure"
        );
    }

    #[test]
    fn rejected_service_start_reports_without_status_error() {
        let directory = tempfile::Builder::new()
            .prefix("busy-service-start-")
            .tempdir_in("/tmp")
            .unwrap();
        let (updates, rx) = mpsc::channel();
        let (service, _owner) = Services::new(
            Paths::at(directory.path().into()),
            egui::Context::default(),
            updates,
        )
        .unwrap();
        service.handle().close_admission();
        while rx.try_recv().is_ok() {}
        assert!(service.send(Job::StartSessionService).is_err());
        let received: Vec<_> = rx.try_iter().collect();
        assert!(
            received
                .iter()
                .any(|update| matches!(update, Update::ServiceStarted(Err(_)))),
            "rejected service start still reports completion"
        );
    }
}
