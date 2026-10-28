use anyhow::{Context, Result};
use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use terminator_core::*;

use super::super::super::shared::{Runtime, Shared, relock};
use super::Target;
use super::command::EnvInput;
use crate::{Launch, terminal_events};

pub(super) struct StartInput<'a> {
    pub command: CommandBuilder,
    pub target: &'a Target,
    pub launch: Launch,
    pub sid: &'a str,
    pub token: String,
}

/// A started session process and the runtime that owns its PTY.
pub(super) struct Spawned {
    pub child: Box<dyn Child + Send + Sync>,
    pub reader: Box<dyn Read + Send>,
    pub runtime: Arc<Mutex<Runtime>>,
    pub pid: Option<u32>,
}

pub(super) struct RecordInput<'a> {
    pub sid: &'a str,
    pub project: &'a str,
    pub launch: Launch,
    pub remote: Option<remote::RemoteSession>,
    pub cwd: PathBuf,
    pub file: Option<PathBuf>,
    pub generation: String,
    pub pid: Option<u32>,
}

impl Shared {
    pub(super) fn start_pty(&self, input: StartInput<'_>) -> Result<Spawned> {
        let StartInput {
            mut command,
            target,
            launch,
            sid,
            token,
        } = input;
        self.apply_session_environment(EnvInput {
            command: &mut command,
            target,
            sid,
            token: &token,
        });
        let pair = native_pty_system().openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let shell_executable = if matches!(launch, Launch::Shell) {
            command
                .get_argv()
                .first()
                .and_then(|path| Path::new(path).canonicalize().ok())
        } else {
            None
        };
        let child = pair
            .slave
            .spawn_command(command)
            .context("Could not start shell/editor")?;
        drop(pair.slave);
        let pid = child.process_id();
        let reader = pair.master.try_clone_reader()?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer()?));
        let parser = vt100::Parser::new_with_callbacks(
            24,
            80,
            target.settings.scrollback_lines,
            terminal_events::Events::default(),
        );
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
        Ok(Spawned {
            child,
            reader,
            runtime,
            pid,
        })
    }

    pub(super) fn session_record(&self, input: RecordInput<'_>) -> Session {
        let RecordInput {
            sid,
            project,
            launch,
            remote,
            cwd,
            file,
            generation,
            pid,
        } = input;
        let editor = !matches!(launch, Launch::Shell);
        Session {
            remote,
            review: matches!(launch, Launch::Review { .. }),
            id: sid.to_owned(),
            project_id: project.to_owned(),
            label: self.session_label(launch, file.as_deref(), project),
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
        }
    }

    fn session_label(&self, launch: Launch, file: Option<&Path>, project: &str) -> String {
        let file_name = || {
            file.and_then(|f| f.file_name())
                .unwrap_or_default()
                .to_string_lossy()
        };
        match launch {
            Launch::Review { staged } => format!(
                "{}: {}",
                if staged { "Staged" } else { "Diff" },
                file_name()
            ),
            Launch::Editor => file_name().into(),
            Launch::Shell => format!(
                "Terminal {}",
                relock(&self.state)
                    .sessions
                    .iter()
                    .filter(|s| s.project_id == project)
                    .count()
                    .saturating_add(1)
            ),
        }
    }

    /// Publishes the session to attach clients and the persisted state.
    pub(super) fn register_session(
        &self,
        sid: &str,
        runtime: &Arc<Mutex<Runtime>>,
        record: &Session,
    ) {
        relock(&self.sessions).insert(sid.to_owned(), Arc::clone(runtime));
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
    }
}
