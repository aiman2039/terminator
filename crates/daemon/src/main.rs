#![forbid(unsafe_code)]
mod editor;
mod helper;
mod idle_close;
mod notifications;
mod review;
mod terminal_env;
mod terminal_events;

#[derive(Clone, Copy)]
enum Launch {
    Shell,
    Editor,
    Review { staged: bool },
}
mod shell;
mod storage;
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use fs2::FileExt;
use portable_pty::{MasterPty, PtySize, native_pty_system};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};
use terminator_core::*;

struct Runtime {
    parser: vt100::Parser<terminal_events::Events>,
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    subscribers: Vec<SyncSender<Response>>,
    token: String,
    ended: bool,
    closing: bool,
    inputs_in_flight: usize,
    shell_executable: Option<std::path::PathBuf>,
}
/// A cloned input descriptor must not keep a failed output connection alive.
struct Disconnect(UnixStream);
impl Drop for Disconnect {
    fn drop(&mut self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}
enum HistoryJob {
    Append(String, Vec<u8>),
    Prune(u64, mpsc::Sender<Result<Vec<String>, String>>),
    Flush(mpsc::Sender<Result<(), String>>),
    Clear(Option<String>, bool, mpsc::Sender<Result<(), String>>),
}
struct Shared {
    helper: helper::Helper,
    catalog_paths: Option<Paths>,
    state: Mutex<State>,
    store: Mutex<storage::Store>,
    sessions: Mutex<HashMap<String, Arc<Mutex<Runtime>>>>,
    paths: Paths,
    auth: String,
    focused: Mutex<(bool, Instant)>,
    worktree_operations: Mutex<()>,
    terminal_operations: Mutex<()>,
    shutdown: AtomicBool,
    history: SyncSender<HistoryJob>,
    alerts: SyncSender<String>,
}
impl Shared {
    fn forward_input(&self, runtime: &Arc<Mutex<Runtime>>, bytes: &[u8]) -> Result<()> {
        let writer = {
            let _operation = self.terminal_operations.lock().unwrap();
            let mut runtime = runtime.lock().unwrap();
            ensure!(!runtime.ended && !runtime.closing, "Session is closing");
            runtime.inputs_in_flight += 1;
            runtime.writer.clone()
        };
        // A full PTY input buffer must not block other panes or Stop.
        // In-flight input makes idle-close ineligible until it finishes.
        let result = {
            let mut writer = writer.lock().unwrap();
            writer.write_all(bytes).and_then(|_| writer.flush())
        };
        {
            let _operation = self.terminal_operations.lock().unwrap();
            runtime.lock().unwrap().inputs_in_flight -= 1;
        }
        result.map_err(Into::into)
    }

    fn prune_global(self: &Arc<Self>, owners: &[generations::Generation]) -> Result<()> {
        let mut segments = Vec::new();
        let mut budgets = HashMap::<String, u64>::new();
        for owner in owners
            .iter()
            .filter(|g| g.status != generations::Status::Prepared)
        {
            budgets.insert(owner.id.clone(), 0);
            for path in storage::history_files(&owner.paths(), None) {
                let metadata = fs::metadata(&path)?;
                *budgets.get_mut(&owner.id).unwrap() += metadata.len();
                segments.push((metadata.modified()?, owner.id.clone(), metadata.len()));
            }
        }
        segments.sort_by_key(|s| s.0);
        let limit = self.state.lock().unwrap().settings.total_mib * 1024 * 1024;
        let original_budgets = budgets.clone();
        let mut total: u64 = budgets.values().sum();
        for (_, owner, bytes) in segments {
            if total <= limit {
                break;
            }
            total = total.saturating_sub(bytes);
            *budgets.get_mut(&owner).unwrap() -= bytes;
        }
        let mine = self.state.lock().unwrap().generation.clone();
        for owner in owners
            .iter()
            .filter(|g| g.status != generations::Status::Prepared)
        {
            let reduced = budgets[&owner.id] < original_budgets[&owner.id];
            if !reduced && owner.status != generations::Status::Retired {
                continue;
            }
            let request = Request::PruneHistory {
                budget: if reduced { budgets[&owner.id] } else { limit },
            };
            self.prune_owner_history(owner, request, &mine)?;
        }
        Ok(())
    }

    fn prune_owner_history(
        self: &Arc<Self>,
        owner: &generations::Generation,
        request: Request,
        mine: &str,
    ) -> Result<()> {
        if owner.status != generations::Status::Retired || owner.id == mine {
            if owner.id == mine {
                self.handle(request)?;
            } else {
                rpc(&owner.paths(), request)?;
            }
            return Ok(());
        }
        if generations::saved(&owner.paths())?
            .sessions
            .iter()
            .any(|s| s.lifecycle.live())
        {
            return Ok(());
        }
        self.handle(Request::Archived {
            generation: owner.id.clone(),
            request: Box::new(request),
        })?;
        Ok(())
    }

    fn handle_archived(
        self: &Arc<Self>,
        generation: String,
        request: Box<Request>,
    ) -> Result<Response> {
        let root = self
            .catalog_paths
            .as_ref()
            .context("No generation catalog")?;
        let _coordination = generations::coordinate(root)?;
        let catalog = generations::Catalog::open(root)?;
        if catalog.active()?.as_deref() == Some(generation.as_str()) {
            drop(_coordination);
            return self.handle(*request);
        }
        let owner = catalog
            .generations()?
            .into_iter()
            .find(|g| g.id == generation && g.status == generations::Status::Retired)
            .context("Owner is not retired")?;
        let paths = owner.paths();
        let (store, mut state) = storage::Store::open(&paths)?;
        ensure!(
            !state.sessions.iter().any(|s| s.lifecycle.live()),
            "Retired owner still has live records"
        );
        match *request {
            Request::PruneHistory { budget } => {
                generations::Catalog::open(root)?.refresh(&mut state)?;
                let ids =
                    storage::History::new(paths)?.prune_with_budget(&state.settings, budget)?;
                for session in &mut state.sessions {
                    if ids.contains(&session.id) {
                        session.truncated = true;
                    }
                }
            }
            Request::History { session } => {
                let record = state
                    .sessions
                    .iter()
                    .find(|s| s.id == session)
                    .context("Unknown historical session")?;
                return Ok(Response::Text(storage::text(&paths, record)?));
            }
            Request::Rename { session, label } => {
                ensure!(label.len() <= 256, "Label too long");
                state
                    .sessions
                    .iter_mut()
                    .find(|s| s.id == session)
                    .context("Unknown session")?
                    .label = label;
            }
            Request::Focus { session } => state.focus(&session),
            Request::Notice { id, action } => {
                let n = state
                    .notifications
                    .iter_mut()
                    .find(|n| n.id == id)
                    .context("Unknown notification")?;
                match action.as_str() {
                    "read" => n.read = true,
                    "dismiss" => n.dismissed = true,
                    "snooze" => n.snoozed_until = now() + 600,
                    _ => bail!("Unknown notification action"),
                }
            }
            Request::DismissTerminalNotice { id } => {
                state
                    .terminal_notices
                    .iter_mut()
                    .find(|n| n.id == id)
                    .context("Unknown notice")?
                    .dismissed = true;
            }
            Request::Remove { session } => {
                storage::History::new(paths)?.clear(Some(&session), true)?;
                state.sessions.retain(|s| s.id != session);
                state.agents.retain(|a| a.session_id != session);
                state.notifications.retain(|n| n.session_id != session);
                state.terminal_notices.retain(|n| n.session_id != session);
            }
            Request::ClearHistory { session } => {
                storage::History::new(paths)?.clear(session.as_deref(), false)?;
                for s in &mut state.sessions {
                    if session.as_ref().is_none_or(|id| id == &s.id) {
                        s.truncated = true;
                    }
                }
            }
            _ => bail!("Operation requires a live session owner"),
        }
        state.revision += 1;
        store.save(&state)?;
        Ok(Response::Ok)
    }

    fn history_clear(&self, session: Option<String>, remove: bool) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        self.history.send(HistoryJob::Clear(session, remove, tx))?;
        rx.recv_timeout(Duration::from_secs(3))?
            .map_err(anyhow::Error::msg)
    }
    fn persist(&self) -> Result<()> {
        let s = self.state.lock().unwrap().clone();
        self.store.lock().unwrap().save(&s)
    }
    fn runtime(&self, id: &str) -> Result<Arc<Mutex<Runtime>>> {
        self.sessions
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .context("Session is not live")
    }
    fn degraded(&self, message: &str) {
        let mut s = self.state.lock().unwrap();
        s.degraded = Some(message.into());
        s.revision += 1;
    }
    fn create(
        self: &Arc<Self>,
        project: String,
        cwd: Option<std::path::PathBuf>,
        file: Option<std::path::PathBuf>,
        line: Option<u32>,
        column: Option<u32>,
        launch: Launch,
    ) -> Result<Session> {
        let _worktree_guard = self.worktree_operations.lock().unwrap();
        ensure!(
            !self.shutdown.load(Ordering::Acquire),
            "Daemon is shutting down"
        );
        let editor = !matches!(launch, Launch::Shell);
        let is_review = matches!(launch, Launch::Review { .. });
        let (settings, root, generation) = {
            let s = self.state.lock().unwrap();
            (
                s.settings.clone(),
                s.projects
                    .iter()
                    .find(|p| p.id == project)
                    .context("Unknown project")?
                    .path
                    .clone(),
                s.generation.clone(),
            )
        };
        let requested_cwd = cwd.unwrap_or(root);
        let cwd = requested_cwd.canonicalize().with_context(|| {
            format!(
                "Working directory unavailable: {}. If it was moved, restore access at this path or open the project at its new location",
                requested_cwd.display()
            )
        })?;
        ensure!(cwd.is_dir(), "Working directory is not a directory");
        let file = file.map(|file| {
            if file.is_absolute() {
                file
            } else {
                cwd.join(file)
            }
        });
        let sid = id();
        let review_files = is_review.then(|| review::Files::new(&self.paths, &sid));
        let token = id();
        let helper = &self.helper.executable;
        ensure!(
            executable_available(helper),
            "Attachment helper unavailable: {}. Open Settings → Updates → Installation to repair Terminator. Existing sessions are preserved",
            helper.display()
        );
        let mut cmd = if let Launch::Review { staged } = launch {
            review::prepare(
                &self.paths,
                &sid,
                &cwd,
                file.as_ref().context("Review requires a file")?,
                staged,
            )?
        } else if editor {
            editor::prepare(
                &self.paths,
                &sid,
                &settings,
                file.as_ref().context("Editor requires a file")?,
                line,
                column,
            )?
        } else {
            let shell = if settings.shell.trim().is_empty() {
                default_shell()?
            } else {
                find_executable(&settings.shell).context(
                    "Configured shell not found; clear the override to use zsh → bash → sh",
                )?
            };
            shell::prepare(&self.paths, &shell.to_string_lossy(), helper)?
        };
        cmd.cwd(&cwd);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        terminal_env::restore_colors(&mut cmd);
        cmd.env("TERMINATOR_SESSION_ID", &sid);
        cmd.env("TERMINATOR_SESSION_TOKEN", &token);
        cmd.env("TERMINATOR_DATA_DIR", &self.paths.data);
        cmd.env("TERMINATOR_RUNTIME_DIR", &self.paths.runtime);
        let pair = native_pty_system().openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let shell_executable = if editor {
            None
        } else {
            cmd.get_argv()
                .first()
                .and_then(|path| std::path::Path::new(path).canonicalize().ok())
        };
        let mut child = pair
            .slave
            .spawn_command(cmd)
            .context("Could not start shell/editor")?;
        drop(pair.slave);
        let pid = child.process_id();
        let mut reader = pair.master.try_clone_reader()?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer()?));
        let runtime = Arc::new(Mutex::new(Runtime {
            parser: vt100::Parser::new_with_callbacks(
                24,
                80,
                settings.scrollback_lines,
                terminal_events::Events::default(),
            ),
            master: pair.master,
            writer,
            subscribers: vec![],
            token,
            ended: false,
            closing: false,
            inputs_in_flight: 0,
            shell_executable,
        }));
        let record = Session {
            review: is_review,
            id: sid.clone(),
            project_id: project.clone(),
            label: if is_review {
                format!(
                    "{}: {}",
                    if matches!(launch, Launch::Review { staged: true }) {
                        "Staged"
                    } else {
                        "Diff"
                    },
                    file.as_ref()
                        .and_then(|f| f.file_name())
                        .unwrap_or_default()
                        .to_string_lossy()
                )
            } else if editor {
                file.as_ref()
                    .and_then(|f| f.file_name())
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into()
            } else {
                format!(
                    "Terminal {}",
                    self.state
                        .lock()
                        .unwrap()
                        .sessions
                        .iter()
                        .filter(|s| s.project_id == project)
                        .count()
                        + 1
                )
            },
            cwd,
            kind: if editor {
                SessionKind::Editor
            } else {
                SessionKind::Shell
            },
            file,
            lifecycle: Lifecycle::Running,
            created: now(),
            exit_code: None,
            rows: 24,
            cols: 80,
            generation,
            pid,
            truncated: false,
            cwd_confirmed: false,
        };
        self.sessions
            .lock()
            .unwrap()
            .insert(sid.clone(), runtime.clone());
        {
            let mut s = self.state.lock().unwrap();
            s.sessions.push(record.clone());
            s.revision += 1;
        }
        if let Err(e) = self.persist() {
            self.degraded(&format!(
                "Session running, recovery persistence failed: {e}"
            ));
        }
        let shared = self.clone();
        let read_sid = sid.clone();
        let read_rt = runtime.clone();
        thread::spawn(move || {
            let mut bytes = [0u8; 8192];
            while let Ok(n) = reader.read(&mut bytes) {
                if n == 0 {
                    break;
                }
                let data = &bytes[..n];
                {
                    let mut rt = read_rt.lock().unwrap();
                    terminal_events::process(&mut rt.parser, data);
                    let replies = std::mem::take(&mut rt.parser.callbacks_mut().replies);
                    let notices = std::mem::take(&mut rt.parser.callbacks_mut().notices);
                    let frame = Response::Data(B64.encode(data));
                    rt.subscribers.retain(|s| s.try_send(frame.clone()).is_ok());
                    drop(rt);
                    for reply in replies {
                        let _ = shared.forward_input(&read_rt, reply.as_bytes());
                    }
                    for notice in notices {
                        let mut state = shared.state.lock().unwrap();
                        let os = state.settings.terminal_notifications_os;
                        let id = state
                            .terminal_notice(&read_sid, &notice.title, &notice.body)
                            .ok()
                            .flatten();
                        drop(state);
                        if os && let Some(id) = id {
                            let _ = shared.alerts.try_send(id);
                        }
                    }
                }
                // Bound memory without dropping bursts. No runtime/state lock is held
                // while storage applies backpressure to this PTY reader.
                if shared
                    .history
                    .send(HistoryJob::Append(read_sid.clone(), data.to_vec()))
                    .is_err()
                {
                    let mut s = shared.state.lock().unwrap();
                    if let Some(rec) = s.sessions.iter_mut().find(|s| s.id == read_sid) {
                        rec.truncated = true;
                    }
                    s.degraded =
                        Some("History storage worker stopped; terminal output continues but some history was not saved".into());
                    s.revision += 1;
                }
            }
        });
        let shared = self.clone();
        thread::spawn(move || {
            let exit = child.wait();
            {
                let mut rt = runtime.lock().unwrap();
                rt.ended = true;
                for tx in rt.subscribers.drain(..) {
                    let _ = tx.try_send(Response::End);
                }
            }
            // Sessions without an agent resume command (plain shells,
            // file editors) are not worth keeping: reopening them
            // restores nothing actionable.
            let removed = {
                let mut s = shared.state.lock().unwrap();
                if let Some(rec) = s.sessions.iter_mut().find(|r| r.id == sid) {
                    rec.lifecycle = Lifecycle::Ended;
                    rec.exit_code = exit.ok().map(|e| e.exit_code());
                    rec.pid = None;
                }
                for a in &mut s.agents {
                    if a.session_id == sid {
                        a.state = AgentState::Stopped;
                    }
                }
                s.revision += 1;
                s.prune_non_resumable_ended()
            };
            let _ = shared.persist();
            for id in &removed {
                let _ = shared.history_clear(Some(id.clone()), true);
            }
            shared.sessions.lock().unwrap().remove(&sid);
            drop(review_files);
        });
        Ok(record)
    }
    fn handle(self: &Arc<Self>, request: Request) -> Result<Response> {
        let shared_write = matches!(
            &request,
            Request::AddProject { .. }
                | Request::SaveLayout { .. }
                | Request::SelectProject { .. }
                | Request::Settings(_)
                | Request::WorktreeAdd { .. }
                | Request::WorktreeRemove { .. }
        );
        let admission = shared_write
            || matches!(
                &request,
                Request::Create { .. } | Request::CreateReview { .. }
            );
        let coordinated = admission
            || matches!(
                &request,
                Request::Cwd { .. }
                    | Request::Shutdown
                    | Request::ShutdownIfIdle
                    | Request::RetireIfDraining
            );
        let _coordination = if coordinated {
            self.catalog_paths
                .as_ref()
                .map(generations::coordinate)
                .transpose()?
        } else {
            None
        };
        if let Some(paths) = &self.catalog_paths {
            let catalog = generations::Catalog::open(paths)?;
            let mut state = self.state.lock().unwrap();
            if admission && catalog.active()?.as_deref() != Some(&state.generation) {
                return Ok(Response::Redirect {
                    generation: catalog.active()?.context("No active generation")?,
                });
            }
            if admission && !shared_write && !catalog.admitted(&state.generation)? {
                return Ok(Response::Error(
                    "Creation rejected before execution: recovery is in progress".into(),
                ));
            }
            if coordinated {
                catalog.refresh(&mut state)?;
            }
        }
        match request {
            Request::PruneHistory { budget } => {
                let (tx, rx) = mpsc::channel();
                self.history.send(HistoryJob::Prune(budget, tx))?;
                let ids = rx
                    .recv_timeout(Duration::from_secs(3))?
                    .map_err(anyhow::Error::msg)?;
                let mut state = self.state.lock().unwrap();
                for session in &mut state.sessions {
                    if ids.contains(&session.id) {
                        session.truncated = true;
                    }
                }
                state.revision += 1;
            }
            Request::Archived {
                generation,
                request,
            } => return self.handle_archived(generation, request),
            Request::CloseIdleSessions {
                generation,
                sessions,
            } => return self.close_idle(generation, sessions),
            Request::ShellCommand { session } => {
                // Older generated shells still emit prompt hooks; ignore them.
                let _ = self.runtime(&session)?;
                return Ok(Response::Text("0".into()));
            }
            Request::ShellPrompt { session, .. } => {
                let _ = self.runtime(&session)?;
            }
            Request::WorktreeList { project } => {
                let path = self
                    .state
                    .lock()
                    .unwrap()
                    .projects
                    .iter()
                    .find(|p| p.id == project)
                    .context("Unknown project")?
                    .path
                    .clone();
                return Ok(Response::Worktrees(worktrees::list(
                    &worktrees::common_dir(&path)?,
                )?));
            }
            Request::WorktreeAdd {
                project,
                path,
                branch,
                start,
            } => {
                let _worktree_guard = self.worktree_operations.lock().unwrap();
                let source = self
                    .state
                    .lock()
                    .unwrap()
                    .projects
                    .iter()
                    .find(|p| p.id == project)
                    .context("Unknown project")?
                    .path
                    .clone();
                let common_dir = worktrees::common_dir(&source)?;
                let revision = worktrees::resolve_start(&source, &start)?;
                let checkout = worktrees::add(&common_dir, &path, branch.as_deref(), &revision)?;
                let mut state = self.state.lock().unwrap();
                let project_id = id();
                let project = Project {
                    id: project_id.clone(),
                    name: checkout
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    path: checkout.path.clone(),
                    layout: serde_json::Value::Null,
                };
                state.projects.push(project);
                state.worktrees.push(worktrees::Registration {
                    project_id,
                    common_dir,
                    path: checkout.path,
                    created: now(),
                    removed: false,
                });
                state.revision += 1;
            }
            Request::WorktreeRemove { project } => {
                let _worktree_guard = self.worktree_operations.lock().unwrap();
                let state = self.state.lock().unwrap();
                let record = state
                    .worktrees
                    .iter()
                    .find(|w| w.project_id == project && !w.removed)
                    .context("Project is not an active managed worktree")?
                    .clone();
                let mut sessions = state.sessions.clone();
                drop(state);
                if let Some(paths) = &self.catalog_paths {
                    let inventory = generations::snapshot(paths)?;
                    ensure!(
                        inventory.generations.iter().all(|g| g.error.is_none()),
                        "Cannot establish worktree safety while an owner is unavailable"
                    );
                    sessions = inventory.sessions;
                }
                worktrees::ensure_unused(&record.path, &project, &sessions)?;
                worktrees::remove(&record.common_dir, &record.path)?;
                let mut state = self.state.lock().unwrap();
                if let Some(record) = state.worktrees.iter_mut().find(|w| w.project_id == project) {
                    record.removed = true;
                }
                state.revision += 1;
            }
            Request::Screen { session } => {
                let runtime = self.runtime(&session)?;
                let screen = runtime.lock().unwrap().parser.screen().contents();
                return Ok(Response::Text(screen));
            }
            Request::Snapshot => {
                return Ok(Response::State(Box::new(
                    self.state.lock().unwrap().clone(),
                )));
            }
            Request::Heartbeat { focused } => {
                *self.focused.lock().unwrap() = (focused, Instant::now());
                return Ok(Response::Ok);
            }
            Request::Create {
                project,
                cwd,
                file,
                line,
                column,
                editor,
            } => {
                return Ok(Response::Created(self.create(
                    project,
                    cwd,
                    file,
                    line,
                    column,
                    if editor {
                        Launch::Editor
                    } else {
                        Launch::Shell
                    },
                )?));
            }
            Request::CreateReview {
                project,
                cwd,
                path,
                staged,
            } => {
                return Ok(Response::Created(self.create(
                    project,
                    Some(cwd),
                    Some(path),
                    None,
                    None,
                    Launch::Review { staged },
                )?));
            }
            Request::AddProject { path } => {
                let path = path.canonicalize()?;
                ensure!(path.is_dir(), "Project must be a directory");
                let mut s = self.state.lock().unwrap();
                if let Some(project) = s.projects.iter().find(|p| p.path == path) {
                    let project = project.id.clone();
                    if s.selected_project.as_ref() != Some(&project) {
                        s.selected_project = Some(project);
                        s.revision += 1;
                    }
                } else {
                    let pid = id();
                    s.projects.push(Project {
                        id: pid.clone(),
                        name: path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into(),
                        path,
                        layout: serde_json::Value::Null,
                    });
                    s.selected_project = Some(pid);
                    s.revision += 1;
                }
            }
            Request::SaveLayout { project, layout } => {
                ensure!(
                    serde_json::to_vec(&layout)?.len() < 512 * 1024,
                    "Layout too large"
                );
                let mut s = self.state.lock().unwrap();
                s.projects
                    .iter_mut()
                    .find(|p| p.id == project)
                    .context("Unknown project")?
                    .layout = layout;
                s.revision += 1;
            }
            Request::SelectProject { project } => {
                let mut s = self.state.lock().unwrap();
                ensure!(
                    s.projects.iter().any(|p| p.id == project),
                    "Unknown project"
                );
                s.selected_project = Some(project);
                s.revision += 1;
            }
            Request::Cwd { session, path } => {
                let _worktree_guard = self.worktree_operations.lock().unwrap();
                ensure!(path.is_absolute(), "Expected absolute cwd");
                let mut s = self.state.lock().unwrap();
                let rec = s
                    .sessions
                    .iter_mut()
                    .find(|r| r.id == session && r.lifecycle.live())
                    .context("Unknown live session")?;
                rec.cwd = path;
                rec.cwd_confirmed = true;
                s.revision += 1;
            }
            Request::Focus { session } => self.state.lock().unwrap().focus(&session),
            Request::Notice { id, action } => {
                let mut s = self.state.lock().unwrap();
                let n = s
                    .notifications
                    .iter_mut()
                    .find(|n| n.id == id)
                    .context("Unknown notification")?;
                match action.as_str() {
                    "read" => n.read = true,
                    "dismiss" => n.dismissed = true,
                    "snooze" => n.snoozed_until = now() + 600,
                    _ => bail!("Unknown notification action"),
                };
                s.revision += 1;
            }
            Request::TerminalNotify {
                session,
                title,
                body,
            } => {
                let mut state = self.state.lock().unwrap();
                let os = state.settings.terminal_notifications_os;
                let id = state.terminal_notice(&session, &title, &body)?;
                drop(state);
                if os && let Some(id) = id {
                    let _ = self.alerts.try_send(id);
                }
            }
            Request::DismissTerminalNotice { id } => {
                let mut state = self.state.lock().unwrap();
                let notice = state
                    .terminal_notices
                    .iter_mut()
                    .find(|n| n.id == id)
                    .context("Unknown terminal notice")?;
                notice.dismissed = true;
                state.revision += 1;
            }
            Request::Settings(settings) => {
                settings.validate()?;
                let mut s = self.state.lock().unwrap();
                s.settings = settings;
                s.revision += 1;
            }
            Request::Hook(event) => {
                let _operation = self.terminal_operations.lock().unwrap();
                let should_os = self
                    .state
                    .lock()
                    .unwrap()
                    .settings
                    .os_events
                    .contains(&event.state);
                let nid = self.state.lock().unwrap().apply_hook(event)?;
                let focused = *self.focused.lock().unwrap();
                if let Some(nid) = nid
                    && should_os
                    && (!focused.0 || focused.1.elapsed() > Duration::from_secs(4))
                {
                    let _ = self.alerts.try_send(nid);
                }
            }
            Request::Rename { session, label } => {
                ensure!(label.len() <= 256, "Label too long");
                let mut s = self.state.lock().unwrap();
                s.sessions
                    .iter_mut()
                    .find(|r| r.id == session)
                    .context("Unknown session")?
                    .label = label;
                s.revision += 1;
            }
            Request::Stop { session } => {
                let _operation = self.terminal_operations.lock().unwrap();
                let rt = self.runtime(&session)?;
                let mut s = self.state.lock().unwrap();
                let rec = s
                    .sessions
                    .iter_mut()
                    .find(|r| r.id == session)
                    .context("Unknown session")?;
                ensure!(!rt.lock().unwrap().ended, "Session ended");
                if let Some(pid) = rec.pid {
                    // Signal the PTY's foreground job as well as its owning shell.
                    if let Some(foreground) = rt.lock().unwrap().master.process_group_leader()
                        && foreground > 1
                        && foreground != pid as i32
                        && let Ok(foreground) = u32::try_from(foreground)
                    {
                        let _ = signals::signal_group(foreground, signals::ProcSignal::Hangup);
                    }
                    // The child remains unreaped while live, preventing PID reuse here.
                    signals::signal_group(pid, signals::ProcSignal::Hangup).map_err(|error| {
                        anyhow::anyhow!("Could not signal session process group: {error}")
                    })?;
                }
                rec.lifecycle = Lifecycle::Stopping;
                s.revision += 1;
            }
            Request::Remove { session } => {
                let mut s = self.state.lock().unwrap();
                ensure!(
                    !s.sessions
                        .iter()
                        .any(|r| r.id == session && r.lifecycle.live()),
                    "Stop session before removing its record"
                );
                s.sessions.retain(|r| r.id != session);
                s.agents.retain(|a| a.session_id != session);
                s.notifications.retain(|n| n.session_id != session);
                s.terminal_notices.retain(|n| n.session_id != session);
                s.revision += 1;
                drop(s);
                self.history_clear(Some(session.clone()), true)?;
            }
            Request::History { session } => {
                let record = self
                    .state
                    .lock()
                    .unwrap()
                    .sessions
                    .iter()
                    .find(|s| s.id == session)
                    .cloned()
                    .context("Unknown session")?;
                let text = if let Ok(rt) = self.runtime(&session) {
                    storage::screen_text(rt.lock().unwrap().parser.screen(), 10_000)
                } else {
                    let (tx, rx) = mpsc::channel();
                    self.history.send(HistoryJob::Flush(tx))?;
                    rx.recv_timeout(Duration::from_secs(3))?
                        .map_err(anyhow::Error::msg)?;
                    storage::text(&self.paths, &record)?
                };
                return Ok(Response::Text(text));
            }
            Request::ClearHistory { session } => {
                self.history_clear(session.clone(), false)?;
                let mut s = self.state.lock().unwrap();
                for r in &mut s.sessions {
                    if session.as_ref().is_none_or(|id| id == &r.id) {
                        r.truncated = true;
                    }
                }
                s.revision += 1;
            }
            Request::EditorSave { session } => return self.editor_rpc(&session, "execute('wall')"),
            Request::EditorCompare { session } => {
                return self.editor_rpc(&session, "execute('TerminatorCompareDisk')");
            }
            Request::EditorStatus { session } => {
                return self.editor_rpc(
                    &session,
                    "string(len(filter(getbufinfo(), 'v:val.changed')))",
                );
            }
            Request::Shutdown | Request::ShutdownIfIdle | Request::RetireIfDraining => {
                if matches!(request, Request::RetireIfDraining) {
                    let root = self
                        .catalog_paths
                        .as_ref()
                        .context("No generation catalog")?;
                    let mine = self.state.lock().unwrap().generation.clone();
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
                }
                let _creation_guard = self.worktree_operations.lock().unwrap();
                ensure!(
                    !self
                        .state
                        .lock()
                        .unwrap()
                        .sessions
                        .iter()
                        .any(|s| s.lifecycle.live()),
                    "Stop running sessions before stopping daemon"
                );
                let (tx, rx) = mpsc::channel();
                self.history.send(HistoryJob::Flush(tx))?;
                rx.recv_timeout(Duration::from_secs(3))?
                    .map_err(anyhow::Error::msg)?;
                self.persist()?;
                self.shutdown.store(true, Ordering::Release);
                if let Some(root) = &self.catalog_paths {
                    generations::Catalog::open(root)?
                        .retire(&self.state.lock().unwrap().generation)?;
                }
                return Ok(Response::Ok);
            }
            _ => bail!("Request requires an attached stream"),
        }
        if shared_write && let Some(paths) = &self.catalog_paths {
            generations::Catalog::open(paths)?.save_workspace(&self.state.lock().unwrap())?;
        }
        self.persist()?;
        Ok(Response::Ok)
    }
    fn editor_rpc(&self, sid: &str, expression: &str) -> Result<Response> {
        let state = self.state.lock().unwrap();
        let program = if state.sessions.iter().any(|s| s.id == sid && s.review) {
            "nvim".to_owned()
        } else {
            state.settings.editor_program.clone()
        };
        drop(state);
        let socket = self.paths.editor_socket(sid);
        ensure!(socket.exists(), "Embedded editor RPC unavailable");
        let editor = find_executable(&program).context("Editor executable not found")?;
        let mut command = std::process::Command::new(editor);
        command
            .args(["--server"])
            .arg(socket)
            .args(["--remote-expr", expression]);
        let out = bounded_output(command, Duration::from_secs(2))?;
        ensure!(
            out.status.success(),
            "Editor request failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(Response::Text(
            String::from_utf8_lossy(&out.stdout).trim().into(),
        ))
    }
}
fn serve(mut stream: UnixStream, shared: Arc<Shared>) -> Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let env: Envelope = read_frame(&mut stream)?;
    ensure!(
        env.version == PROTOCOL_VERSION,
        "Unsupported protocol version"
    );
    let authenticated = if env.auth == shared.auth {
        true
    } else {
        match &env.request {
            Request::Hook(e) => shared
                .runtime(&e.terminal_session_id)
                .is_ok_and(|r| r.lock().unwrap().token == env.auth),
            Request::ShellCommand { session }
            | Request::ShellPrompt { session, .. }
            | Request::Cwd { session, .. }
            | Request::TerminalNotify { session, .. } => shared
                .runtime(session)
                .is_ok_and(|r| r.lock().unwrap().token == env.auth),
            _ => false,
        }
    };
    ensure!(authenticated, "Authentication failed");
    if matches!(env.request, Request::Snapshot) {
        let executable = std::env::current_exe().ok();
        let helper = &shared.helper.executable;
        let available = executable_available(helper);
        let mut state = shared.state.lock().unwrap();
        if state.daemon_executable != executable
            || state.attachment_helper_executable.as_ref() != Some(helper)
            || state.attachment_helper_available != Some(available)
        {
            state.daemon_executable = executable;
            state.attachment_helper_executable = Some(helper.clone());
            state.attachment_helper_available = Some(available);
            state.revision += 1;
        }
    }
    if matches!(env.request, Request::Snapshot)
        && let Some(hint) = &env.snapshot_hint
    {
        let state = shared.state.lock().unwrap();
        if hint == &state.snapshot_hint() {
            write_frame(&mut stream, &Response::Unchanged)?;
            return Ok(());
        }
    }
    if let Request::Attach {
        session,
        rows,
        cols,
    } = env.request
    {
        let _disconnect = Disconnect(stream.try_clone()?);
        ensure!(
            rows > 0 && cols > 0 && rows <= 500 && cols <= 1000,
            "Invalid terminal dimensions"
        );
        let rt = shared.runtime(&session)?;
        let (tx, rx) = mpsc::sync_channel(128);
        {
            let mut r = rt.lock().unwrap();
            r.master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })?;
            r.parser.screen_mut().set_size(rows, cols);
            // Formatted screen is generated by vt100, never replay arbitrary OSC effects.
            let mut snapshot = Vec::new();
            let mut history = r.parser.screen().clone();
            history.set_scrollback(10_000);
            for offset in (1..=history.scrollback()).rev() {
                history.set_scrollback(offset);
                if let Some(row) = history.rows(0, cols).next() {
                    snapshot.extend_from_slice(row.as_bytes());
                    snapshot.extend_from_slice(b"\r\n");
                }
            }
            if r.parser.screen().alternate_screen() {
                snapshot.extend_from_slice(b"\x1b[?1049h");
            }
            snapshot.extend(r.parser.screen().state_formatted());
            snapshot.extend(r.parser.callbacks().modes_formatted());
            write_frame(&mut stream, &Response::Data(B64.encode(snapshot)))?;
            r.subscribers.push(tx);
        }
        stream.set_read_timeout(None)?;
        let mut input = stream.try_clone()?;
        let input_disconnect = Disconnect(input.try_clone()?);
        let input_rt = rt.clone();
        let input_shared = shared.clone();
        let session_id = session.clone();
        thread::spawn(move || {
            let _disconnect = input_disconnect;
            while let Ok(req) = read_frame::<Request>(&mut input) {
                match req {
                    Request::Input { data } => {
                        if let Ok(bytes) = B64.decode(data)
                            && input_shared.forward_input(&input_rt, &bytes).is_err()
                        {
                            break;
                        }
                    }
                    Request::Resize { rows, cols }
                        if rows > 0 && cols > 0 && rows <= 500 && cols <= 1000 =>
                    {
                        let mut r = input_rt.lock().unwrap();
                        let _ = r.master.resize(PtySize {
                            rows,
                            cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        });
                        r.parser.screen_mut().set_size(rows, cols);
                        drop(r);
                        let mut s = input_shared.state.lock().unwrap();
                        if let Some(rec) = s.sessions.iter_mut().find(|s| s.id == session_id) {
                            rec.rows = rows;
                            rec.cols = cols;
                        }
                    }
                    _ => break,
                }
            }
        });
        while let Ok(frame) = rx.recv_timeout(Duration::from_secs(2)) {
            write_frame(&mut stream, &frame)?;
            if matches!(frame, Response::End) {
                break;
            }
        }
        // A quiet terminal must not time out: keep the stream alive until it disconnects or ends.
        if !rt.lock().unwrap().ended {
            loop {
                match rx.recv_timeout(Duration::from_secs(2)) {
                    Ok(frame) => {
                        write_frame(&mut stream, &frame)?;
                        if matches!(frame, Response::End) {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        write_frame(&mut stream, &Response::Ok)?;
                    }
                    Err(_) => break,
                }
            }
        }
        Ok(())
    } else {
        let response = shared
            .handle(env.request)
            .unwrap_or_else(|e| Response::Error(format!("{e:#}")));
        snapshot::write_response(&mut stream, &response, env.snapshot_chunks)
    }
}
fn main() -> Result<()> {
    notifications::initialize();
    let mut paths = Paths::discover()?;
    if let Some(p) = std::env::var_os("TERMINATOR_RUNTIME_DIR") {
        paths.runtime = p.into();
    }
    paths.init()?;
    crash::install(crash::CrashInstall {
        binary: "terminator-daemon",
        version: env!("CARGO_PKG_VERSION"),
        data_dir: paths.data.clone(),
        notify: None,
    });
    let catalog_paths = std::env::var_os("TERMINATOR_CATALOG_DATA").map(|data| Paths {
        data: data.into(),
        runtime: std::env::var_os("TERMINATOR_CATALOG_RUNTIME")
            .expect("Catalog runtime")
            .into(),
    });
    if let Some(root) = &catalog_paths {
        generations::validate_endpoint(
            root,
            &paths,
            &std::env::var("TERMINATOR_GENERATION").context("Missing generation identity")?,
        )?;
    }
    let _legacy_guard = if let Some(root) = &catalog_paths {
        let guard = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.runtime.join("daemon.lock"))?;
        FileExt::try_lock_shared(&guard).context("Legacy service has not retired")?;
        Some(guard)
    } else {
        None
    };
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.runtime.join("daemon.lock"))?;
    lock.try_lock_exclusive()
        .context("Daemon already running")?;
    helper::cleanup_abandoned(&paths.data)?;
    let helper = helper::Helper::stage(
        &std::env::current_exe()?.with_file_name("terminator-hook"),
        &paths.data,
    )?;
    let _ = fs::remove_file(paths.socket());
    let listener = UnixListener::bind(paths.socket())?;
    fs::set_permissions(paths.socket(), fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let auth = id();
    atomic_write(&paths.auth(), auth.as_bytes())?;
    let (mut store, mut state) = storage::Store::open(&paths)?;
    state.recover();
    // Migrate existing stores: drop historical sessions without an agent
    // resume command so old plain shells/editors stop filling History.
    let removed = state.prune_non_resumable_ended();
    if !removed.is_empty()
        && let Ok(mut history) = storage::History::new(paths.clone())
    {
        for id in &removed {
            let _ = history.clear(Some(id), true);
        }
        let _ = history.flush();
    }
    if let Some(root) = &catalog_paths {
        let owner = std::env::var("TERMINATOR_GENERATION")?;
        ensure!(
            state.sessions.is_empty(),
            "Generation directory must be new; never restart owned processes"
        );
        state.generation = owner.clone();
        store.bind_owner(owner);
        generations::Catalog::open(root)?.refresh(&mut state)?;
    }
    state.daemon_version = Some(env!("CARGO_PKG_VERSION").into());
    state.capabilities = vec![
        terminator_core::idle_close::CAPABILITY.into(),
        STABLE_HELPER_CAPABILITY.into(),
        SHUTDOWN_IF_IDLE_CAPABILITY.into(),
        snapshot::CAPABILITY.into(),
        NVIM_REVIEW_CAPABILITY.into(),
        TERMINAL_NOTICES_CAPABILITY.into(),
        NOTIFICATION_SOUND_CAPABILITY.into(),
        DIFF_CLOSE_SETTINGS_CAPABILITY.into(),
        WORKTREES_CAPABILITY.into(),
        SCREEN_CAPABILITY.into(),
        METADATA_SETTINGS_CAPABILITY.into(),
    ];
    if catalog_paths.is_some() {
        state.daemon_build = Some(generations::build_identity(
            std::env::current_exe()?
                .parent()
                .context("Missing executable directory")?,
        )?);
        state.daemon_catalog_version = Some(generations::CATALOG_VERSION);
        state.capabilities.push(generations::CAPABILITY.into());
    }
    store.save(&state)?;
    let (history, history_rx) = mpsc::sync_channel::<HistoryJob>(512);
    let (alerts, alert_rx) = mpsc::sync_channel::<String>(64);
    let shared = Arc::new(Shared {
        helper,
        catalog_paths,
        state: Mutex::new(state),
        store: Mutex::new(store),
        sessions: Mutex::new(HashMap::new()),
        paths,
        auth,
        focused: Mutex::new((false, Instant::now())),
        worktree_operations: Mutex::new(()),
        terminal_operations: Mutex::new(()),
        shutdown: AtomicBool::new(false),
        history,
        alerts,
    });
    let weak = Arc::downgrade(&shared);
    thread::spawn(move || {
        let Some(shared) = weak.upgrade() else { return };
        let mut history = match storage::History::new(shared.paths.clone()) {
            Ok(h) => h,
            Err(e) => {
                shared.degraded(&format!("History initialization failed: {e}"));
                return;
            }
        };
        drop(shared);
        let mut last_flush = Instant::now();
        let mut last_prune = Instant::now();
        let mut previous_settings = None;
        loop {
            let Some(s) = weak.upgrade() else { break };
            match history_rx.recv_timeout(Duration::from_millis(250)) {
                Ok(HistoryJob::Prune(budget, tx)) => {
                    let settings = s.state.lock().unwrap().settings.clone();
                    let _ = tx.send(
                        history
                            .prune_with_budget(&settings, budget)
                            .map_err(|e| e.to_string()),
                    );
                }
                Ok(HistoryJob::Append(sid, data)) => {
                    if let Err(e) = history.append(&sid, &data) {
                        let mut state = s.state.lock().unwrap();
                        if let Some(record) = state.sessions.iter_mut().find(|r| r.id == sid) {
                            record.truncated = true;
                        }
                        state.degraded = Some(format!(
                            "History write failed; some terminal history was not saved: {e}"
                        ));
                        state.revision += 1;
                    }
                }
                Ok(HistoryJob::Flush(tx)) => {
                    let _ = tx.send(history.flush().map_err(|e| e.to_string()));
                }
                Ok(HistoryJob::Clear(session, remove, tx)) => {
                    let _ = tx.send(
                        history
                            .clear(session.as_deref(), remove)
                            .map_err(|e| e.to_string()),
                    );
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                _ => {}
            }
            if last_flush.elapsed() >= Duration::from_millis(250) {
                if let Err(e) = history.flush() {
                    s.degraded(&format!("History flush failed: {e}"));
                }
                last_flush = Instant::now();
            }
            let settings = s.state.lock().unwrap().settings.clone();
            let limits = (
                settings.history_days,
                settings.session_mib,
                settings.total_mib,
            );
            if history.exceeds(&settings)
                || previous_settings != Some(limits)
                || last_prune.elapsed() >= Duration::from_secs(30)
            {
                match history.prune(&settings) {
                    Ok(ids) if !ids.is_empty() => {
                        let mut state = s.state.lock().unwrap();
                        for record in &mut state.sessions {
                            if ids.contains(&record.id) {
                                record.truncated = true;
                            }
                        }
                        state.revision += 1;
                        drop(state);
                        let _ = s.persist();
                    }
                    Ok(_) => {}
                    Err(e) => s.degraded(&format!("History cleanup failed: {e}")),
                }
                previous_settings = Some(limits);
                last_prune = Instant::now();
            }
            if s.shutdown.load(Ordering::Relaxed) {
                break;
            }
        }
        let _ = history.flush();
    });
    let weak = Arc::downgrade(&shared);
    thread::spawn(move || {
        let mut last = Instant::now() - Duration::from_secs(10);
        while let Ok(nid) = alert_rx.recv() {
            let Some(s) = weak.upgrade() else { break };
            if std::env::var_os("TERMINATOR_NO_NOTIFICATIONS").is_some()
                || last.elapsed() < Duration::from_secs(2)
            {
                continue;
            }
            let state = s.state.lock().unwrap();
            let summary = state
                .notifications
                .iter()
                .find(|n| n.id == nid)
                .map(|n| format!("Agent: {}", n.state.label()))
                .or_else(|| {
                    state
                        .terminal_notices
                        .iter()
                        .find(|n| n.id == nid && !n.dismissed)
                        .map(|n| format!("Terminal: {}", n.title))
                });
            let Some(summary) = summary else {
                continue;
            };
            let sound = state.settings.notification_sound;
            drop(state);
            notifications::send(notifications::DesktopAlert {
                paths: s.catalog_paths.clone().unwrap_or_else(|| s.paths.clone()),
                summary,
                notice: nid,
                sound,
            });
            last = Instant::now();
        }
    });
    let weak = Arc::downgrade(&shared);
    thread::spawn(move || {
        let born = Instant::now();
        let mut global_prune = Instant::now() - Duration::from_secs(30);
        loop {
            thread::sleep(Duration::from_secs(1));
            let Some(shared) = weak.upgrade() else {
                break;
            };
            if shared.shutdown.load(Ordering::Acquire) {
                break;
            }
            let result = (|| -> Result<()> {
                if let Some(root) = &shared.catalog_paths {
                    let _coordination = generations::coordinate(root)?;
                    let catalog = generations::Catalog::open(root)?;
                    let owner = shared.state.lock().unwrap().generation.clone();
                    catalog.refresh(&mut shared.state.lock().unwrap())?;
                    if catalog.active()?.as_deref() == Some(&owner)
                        && !shared.shutdown.load(Ordering::Acquire)
                    {
                        let _ = catalog.restore_serving();
                    }
                    let owners = catalog.generations()?;
                    let draining = owners.iter().any(|g| {
                        g.id == owner
                            && (g.status == generations::Status::Draining
                                || (g.status == generations::Status::Prepared
                                    && born.elapsed() > Duration::from_secs(30)))
                    });
                    let active_owner = catalog.active()?.as_deref() == Some(&owner);
                    drop(_coordination);
                    if active_owner {
                        if global_prune.elapsed() >= Duration::from_secs(30) {
                            if let Err(error) = shared.prune_global(&owners) {
                                shared.degraded(&format!("Global history retention: {error:#}"));
                            }
                            global_prune = Instant::now();
                        }
                        for other in owners
                            .iter()
                            .filter(|g| g.id != owner && g.status != generations::Status::Retired)
                        {
                            let _ = generations::recover_exited(root, other);
                        }
                    }
                    if draining
                        && !shared
                            .state
                            .lock()
                            .unwrap()
                            .sessions
                            .iter()
                            .any(|s| s.lifecycle.live())
                    {
                        let _ = shared.handle(Request::RetireIfDraining);
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                shared.degraded(&format!("Generation maintenance: {error:#}"));
            }
        }
    });
    let active = Arc::new(AtomicUsize::new(0));
    while !shared.shutdown.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                if active.load(Ordering::Relaxed) > 256 {
                    continue;
                }
                let s = shared.clone();
                let count = active.clone();
                count.fetch_add(1, Ordering::Relaxed);
                thread::spawn(move || {
                    let _ = serve(stream, s);
                    count.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => notifications::idle(),
            Err(e) => return Err(e.into()),
        }
    }
    shared.persist()?;
    let _ = fs::remove_file(shared.paths.socket());
    shared.helper.cleanup()?;
    if shared.catalog_paths.is_some() {
        let _ = fs::remove_file(shared.paths.auth());
        let bin = shared.paths.data.join("bin");
        if bin.is_dir() {
            fs::remove_dir_all(bin)?;
        }
    }
    if let Some(root) = &shared.catalog_paths {
        let live = shared
            .state
            .lock()
            .unwrap()
            .sessions
            .iter()
            .any(|s| s.lifecycle.live());
        if !live {
            let _coordination = generations::coordinate(root)?;
            generations::Catalog::open(root)?.retire(&shared.state.lock().unwrap().generation)?;
        }
    }
    Ok(())
}
