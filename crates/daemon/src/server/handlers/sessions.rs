use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use portable_pty::{PtySize, native_pty_system};
use std::{
    io::Read,
    sync::{Arc, Mutex, atomic::Ordering},
    thread,
};
use terminator_core::*;

use super::super::shared::{HistoryJob, Runtime, Shared, relock};
use crate::Launch;
use crate::{editor, review, shell, terminal_env, terminal_events};

impl Shared {
    pub(crate) fn create(
        self: &Arc<Self>,
        project: String,
        cwd: Option<std::path::PathBuf>,
        file: Option<std::path::PathBuf>,
        line: Option<u32>,
        column: Option<u32>,
        launch: Launch,
    ) -> Result<Session> {
        let _worktree_guard = relock(&self.worktree_operations);
        ensure!(
            !self.shutdown.load(Ordering::Acquire),
            "Daemon is shutting down"
        );
        let editor = !matches!(launch, Launch::Shell);
        let is_review = matches!(launch, Launch::Review { .. });
        let (settings, root, generation) = {
            let s = relock(&self.state);
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
        let mut parser = vt100::Parser::new_with_callbacks(
            24,
            80,
            settings.scrollback_lines,
            terminal_events::Events::default(),
        );
        parser
            .callbacks_mut()
            .set_scrollback_cap(settings.scrollback_lines);
        let runtime = Arc::new(Mutex::new(Runtime {
            parser,
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
                    relock(&self.state)
                        .sessions
                        .iter()
                        .filter(|s| s.project_id == project)
                        .count()
                        .saturating_add(1)
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
        relock(&self.sessions).insert(sid.clone(), Arc::clone(&runtime));
        {
            let mut s = relock(&self.state);
            s.sessions.push(record.clone());
            s.revision = s.revision.saturating_add(1);
        }
        if let Err(e) = self.persist() {
            self.degraded(&format!(
                "Session running, recovery persistence failed: {e}"
            ));
        }
        let shared = Arc::clone(self);
        let read_sid = sid.clone();
        let read_rt = Arc::clone(&runtime);
        thread::spawn(move || {
            let mut bytes = [0u8; 8192];
            while let Ok(n) = reader.read(&mut bytes) {
                if n == 0 {
                    break;
                }
                let Some(data) = bytes.get(..n) else {
                    break;
                };
                {
                    let mut rt = relock(&read_rt);
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
                        let mut state = relock(&shared.state);
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
                    let mut s = relock(&shared.state);
                    if let Some(rec) = s.sessions.iter_mut().find(|s| s.id == read_sid) {
                        rec.truncated = true;
                    }
                    s.degraded =
                        Some("History storage worker stopped; terminal output continues but some history was not saved".into());
                    s.revision = s.revision.saturating_add(1);
                }
            }
        });
        let shared = Arc::clone(self);
        thread::spawn(move || {
            let exit = child.wait();
            {
                let mut rt = relock(&runtime);
                rt.ended = true;
                for tx in rt.subscribers.drain(..) {
                    let _ = tx.try_send(Response::End);
                }
            }
            {
                let mut s = relock(&shared.state);
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
                s.revision = s.revision.saturating_add(1);
                // Sessions without an agent resume command (plain shells,
                // file editors) are not worth keeping: reopening them
                // restores nothing actionable.
                if !s.session_has_resume(&sid) {
                    s.sessions.retain(|r| r.id != sid);
                    s.agents.retain(|a| a.session_id != sid);
                    s.notifications.retain(|n| n.session_id != sid);
                    s.terminal_notices.retain(|n| n.session_id != sid);
                    s.revision = s.revision.saturating_add(1);
                }
            }
            let _ = shared.persist();
            if !relock(&shared.state).sessions.iter().any(|r| r.id == sid) {
                let _ = shared.history_clear(Some(sid.clone()), true);
            }
            relock(&shared.sessions).remove(&sid);
            drop(review_files);
        });
        Ok(record)
    }
}
