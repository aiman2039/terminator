use anyhow::{Context, Result, bail, ensure};
use std::{
    sync::{Arc, atomic::Ordering, mpsc},
    time::{Duration, Instant},
};
use terminator_core::*;

use super::super::shared::{HistoryJob, Shared, relock};
use crate::Launch;
use crate::{notifications, ntfy, presence, signals, storage};

impl Shared {
    pub(crate) fn handle(self: &Arc<Self>, request: Request) -> Result<Response> {
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
            let mut state = relock(&self.state);
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
                let mut state = relock(&self.state);
                for session in &mut state.sessions {
                    if ids.contains(&session.id) {
                        session.truncated = true;
                    }
                }
                state.revision = state.revision.saturating_add(1);
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
                let path = relock(&self.state)
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
                let _worktree_guard = relock(&self.worktree_operations);
                let source = relock(&self.state)
                    .projects
                    .iter()
                    .find(|p| p.id == project)
                    .context("Unknown project")?
                    .path
                    .clone();
                let common_dir = worktrees::common_dir(&source)?;
                let revision = worktrees::resolve_start(&source, &start)?;
                let checkout = worktrees::add(&common_dir, &path, branch.as_deref(), &revision)?;
                let mut state = relock(&self.state);
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
                state.revision = state.revision.saturating_add(1);
            }
            Request::WorktreeRemove { project } => {
                let _worktree_guard = relock(&self.worktree_operations);
                let state = relock(&self.state);
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
                let mut state = relock(&self.state);
                if let Some(record) = state.worktrees.iter_mut().find(|w| w.project_id == project) {
                    record.removed = true;
                }
                if state.selected_project.as_ref() == Some(&project) {
                    state.selected_project = state
                        .projects
                        .iter()
                        .find(|candidate| {
                            candidate.id != project
                                && !state
                                    .worktrees
                                    .iter()
                                    .any(|w| w.project_id == candidate.id && w.removed)
                        })
                        .map(|candidate| candidate.id.clone());
                }
                state.revision = state.revision.saturating_add(1);
            }
            Request::Screen { session } => {
                let runtime = self.runtime(&session)?;
                let screen = relock(&runtime).parser.screen().contents();
                return Ok(Response::Text(screen));
            }
            Request::Snapshot => {
                let mut state = relock(&self.state);
                presence::expire_observations(&mut state, now());
                return Ok(Response::State(Box::new(state.clone())));
            }
            Request::Heartbeat { focused } => {
                *relock(&self.focused) = (focused, Instant::now());
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
                let mut s = relock(&self.state);
                if let Some(project) = s.projects.iter().find(|p| p.path == path) {
                    let project = project.id.clone();
                    let previous = s.worktrees.len();
                    s.worktrees
                        .retain(|w| w.project_id != project || !w.removed);
                    if previous != s.worktrees.len() {
                        s.revision = s.revision.saturating_add(1);
                    }
                    if s.selected_project.as_ref() != Some(&project) {
                        s.selected_project = Some(project);
                        s.revision = s.revision.saturating_add(1);
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
                    s.revision = s.revision.saturating_add(1);
                }
            }
            Request::SaveLayout { project, layout } => {
                ensure!(
                    serde_json::to_vec(&layout)?.len() < 512 * 1024,
                    "Layout too large"
                );
                let mut s = relock(&self.state);
                s.projects
                    .iter_mut()
                    .find(|p| p.id == project)
                    .context("Unknown project")?
                    .layout = layout;
                s.revision = s.revision.saturating_add(1);
            }
            Request::SelectProject { project } => {
                let mut s = relock(&self.state);
                ensure!(
                    !s.worktrees
                        .iter()
                        .any(|w| w.project_id == project && w.removed),
                    "Worktree was removed"
                );
                ensure!(
                    s.projects.iter().any(|p| p.id == project),
                    "Unknown project"
                );
                s.selected_project = Some(project);
                s.revision = s.revision.saturating_add(1);
            }
            Request::Cwd { session, path } => {
                let _worktree_guard = relock(&self.worktree_operations);
                ensure!(path.is_absolute(), "Expected absolute cwd");
                let mut s = relock(&self.state);
                let rec = s
                    .sessions
                    .iter_mut()
                    .find(|r| r.id == session && r.lifecycle.live())
                    .context("Unknown live session")?;
                rec.cwd = path;
                rec.cwd_confirmed = true;
                s.revision = s.revision.saturating_add(1);
            }
            Request::Focus { session } => relock(&self.state).focus(&session),
            Request::Notice { id, action } => {
                let mut s = relock(&self.state);
                let n = s
                    .notifications
                    .iter_mut()
                    .find(|n| n.id == id)
                    .context("Unknown notification")?;
                match action.as_str() {
                    "read" => n.read = true,
                    "dismiss" => n.dismissed = true,
                    "snooze" => n.snoozed_until = now().saturating_add(600),
                    _ => bail!("Unknown notification action"),
                }
                s.revision = s.revision.saturating_add(1);
            }
            Request::TerminalNotify {
                session,
                title,
                body,
            } => {
                let mut state = relock(&self.state);
                let os = state.settings.terminal_notifications_os;
                let id = state.terminal_notice(&session, &title, &body)?;
                drop(state);
                if os && let Some(id) = id {
                    let _ = self.alerts.try_send(id);
                }
            }
            Request::DismissTerminalNotice { id } => {
                let mut state = relock(&self.state);
                let notice = state
                    .terminal_notices
                    .iter_mut()
                    .find(|n| n.id == id)
                    .context("Unknown terminal notice")?;
                notice.dismissed = true;
                state.revision = state.revision.saturating_add(1);
            }
            Request::Settings(settings) => {
                settings.validate()?;
                let mut s = relock(&self.state);
                s.settings = settings;
                s.revision = s.revision.saturating_add(1);
            }
            Request::Hook(event) => {
                let _operation = relock(&self.terminal_operations);
                let should_os = relock(&self.state)
                    .settings
                    .os_events
                    .contains(&event.state);
                let mut state = relock(&self.state);
                let ping = ntfy::Ping::from_event(&state.settings, &event);
                let key = format!("{}:{}", event.agent_invocation_id, event.state as u8);
                let nid = state.apply_hook(event)?;
                drop(state);
                if nid.is_some()
                    && let Some(ping) = ping
                    && relock(&self.ntfy_cooldown).allow(&key, now())
                {
                    let _ = self.ntfy.try_send(ping);
                }
                let focused = *relock(&self.focused);
                if let Some(nid) = nid
                    && should_os
                    && (!focused.0 || focused.1.elapsed() > Duration::from_secs(4))
                {
                    let _ = self.alerts.try_send(nid);
                }
            }
            Request::Rename { session, label } => {
                ensure!(label.len() <= 256, "Label too long");
                let mut s = relock(&self.state);
                s.sessions
                    .iter_mut()
                    .find(|r| r.id == session)
                    .context("Unknown session")?
                    .label = label;
                s.revision = s.revision.saturating_add(1);
            }
            Request::Stop { session } => {
                let _operation = relock(&self.terminal_operations);
                let rt = self.runtime(&session)?;
                let mut s = relock(&self.state);
                let rec = s
                    .sessions
                    .iter_mut()
                    .find(|r| r.id == session)
                    .context("Unknown session")?;
                ensure!(!relock(&rt).ended, "Session ended");
                if let Some(pid) = rec.pid {
                    // Signal the PTY's foreground job as well as its owning shell.
                    #[cfg(unix)]
                    if let Ok(shell_pid) = i32::try_from(pid)
                        && let Some(foreground) = relock(&rt).master.process_group_leader()
                        && foreground > 1
                        && foreground != shell_pid
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
                s.revision = s.revision.saturating_add(1);
            }
            Request::Remove { session } => {
                let mut s = relock(&self.state);
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
                s.revision = s.revision.saturating_add(1);
                drop(s);
                self.history_clear(Some(session.clone()), true)?;
            }
            Request::History { session } => {
                let record = relock(&self.state)
                    .sessions
                    .iter()
                    .find(|s| s.id == session)
                    .cloned()
                    .context("Unknown session")?;
                let text = if let Ok(rt) = self.runtime(&session) {
                    storage::screen_text(relock(&rt).parser.screen(), 10_000)
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
                let mut s = relock(&self.state);
                for r in &mut s.sessions {
                    if session.as_ref().is_none_or(|id| id == &r.id) {
                        r.truncated = true;
                    }
                }
                s.revision = s.revision.saturating_add(1);
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
                }
                let _creation_guard = relock(&self.worktree_operations);
                ensure!(
                    !relock(&self.state)
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
                notifications::wake();
                if let Some(root) = &self.catalog_paths {
                    generations::Catalog::open(root)?.retire(&relock(&self.state).generation)?;
                }
                return Ok(Response::Ok);
            }
            _ => bail!("Request requires an attached stream"),
        }
        if shared_write && let Some(paths) = &self.catalog_paths {
            generations::Catalog::open(paths)?.save_workspace(&relock(&self.state))?;
        }
        self.persist()?;
        Ok(Response::Ok)
    }
    pub(crate) fn editor_rpc(&self, sid: &str, expression: &str) -> Result<Response> {
        let state = relock(&self.state);
        let program = if state.sessions.iter().any(|s| s.id == sid && s.review) {
            "nvim".to_owned()
        } else {
            state.settings.editor_program.clone()
        };
        drop(state);
        let socket = self.paths.editor_socket(sid);
        // Neovim listens on a named pipe on Windows, so the socket path has
        // no filesystem record there; the `--server` address must match the
        // `--listen` address from `editor::prepare` on every platform.
        let server = transport::nvim_listen_arg(&socket);
        #[cfg(unix)]
        ensure!(socket.exists(), "Embedded editor RPC unavailable");
        let editor = find_executable(&program).context("Editor executable not found")?;
        let mut command = std::process::Command::new(editor);
        command
            .args(["--server"])
            .arg(server)
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
