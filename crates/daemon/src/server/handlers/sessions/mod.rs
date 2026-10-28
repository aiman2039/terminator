mod command;
mod exit;
mod output;
mod spawn;

use anyhow::{Context, Result, ensure};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};
use terminator_core::*;

use super::super::shared::{Shared, relock};
use crate::{Launch, review};

pub(crate) struct CreateInput {
    pub project: String,
    pub cwd: Option<PathBuf>,
    pub file: Option<PathBuf>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub launch: Launch,
}

/// Where a new session runs, resolved under the worktree guard.
struct Target {
    remote_project: Option<remote::RemoteProject>,
    /// The cwd as the client asked for it, before local resolution.
    requested_cwd: Option<PathBuf>,
    settings: Settings,
    cwd: PathBuf,
    generation: String,
}

impl Target {
    fn absolute_file(&self, file: Option<PathBuf>) -> Option<PathBuf> {
        file.map(|file| {
            if self.remote_project.is_some() || file.is_absolute() {
                file
            } else {
                self.cwd.join(file)
            }
        })
    }
}

impl Shared {
    pub(crate) fn create(self: &Arc<Self>, input: CreateInput) -> Result<Session> {
        let CreateInput {
            project,
            cwd,
            file,
            line,
            column,
            launch,
        } = input;
        let _worktree_guard = relock(&self.worktree_operations);
        ensure!(
            !self.shutdown.load(Ordering::Acquire),
            "Daemon is shutting down"
        );
        let target = self.resolve_target(&project, cwd)?;
        let file = target.absolute_file(file);
        let sid = id();
        let review_files =
            matches!(launch, Launch::Review { .. }).then(|| review::Files::new(&self.paths, &sid));
        let token = id();
        self.ensure_helper_available()?;
        let prepared = self.launch_command(command::LaunchInput {
            target: &target,
            launch,
            sid: &sid,
            file,
            line,
            column,
        })?;
        let spawned = self.start_pty(spawn::StartInput {
            command: prepared.command,
            target: &target,
            launch,
            sid: &sid,
            token,
        })?;
        let record = self.session_record(spawn::RecordInput {
            sid: &sid,
            project: &project,
            launch,
            remote: prepared.remote_session,
            cwd: target.cwd,
            file: prepared.file,
            generation: target.generation,
            pid: spawned.pid,
        });
        self.register_session(&sid, &spawned.runtime, &record);
        let reader_finished = self.spawn_output_reader(&sid, &spawned.runtime, spawned.reader);
        self.spawn_exit_watcher(exit::ExitWatch {
            child: spawned.child,
            runtime: spawned.runtime,
            sid,
            reader_finished,
            review_files,
        });
        if record.remote.is_some() && record.kind == SessionKind::Shell {
            self.auto_start_forwards(&project);
        }
        Ok(record)
    }

    fn resolve_target(&self, project: &str, cwd: Option<PathBuf>) -> Result<Target> {
        let remote_project = relock(&self.state)
            .remote_projects
            .iter()
            .find(|p| p.id == project)
            .cloned();
        let requested_cwd = cwd.clone();
        let (settings, root, generation) = {
            let s = relock(&self.state);
            ensure!(
                !s.worktrees
                    .iter()
                    .any(|worktree| worktree.project_id == project && worktree.removed),
                "Worktree was removed; open an existing project before creating a session"
            );
            (
                s.settings.clone(),
                if let Some(remote) = &remote_project {
                    remote.identity_path()
                } else {
                    s.projects
                        .iter()
                        .find(|p| p.id == project)
                        .context("Unknown project")?
                        .path
                        .clone()
                },
                s.generation.clone(),
            )
        };
        let cwd = if remote_project.is_some() {
            root
        } else {
            Self::local_cwd(root, cwd)?
        };
        Ok(Target {
            remote_project,
            requested_cwd,
            settings,
            cwd,
            generation,
        })
    }

    fn local_cwd(root: PathBuf, cwd: Option<PathBuf>) -> Result<PathBuf> {
        ensure!(
            root.is_dir(),
            "Project directory unavailable: {}. If it was moved, restore access at this path or open the project at its new location",
            root.display()
        );
        let requested_cwd = cwd.unwrap_or(root);
        let cwd = requested_cwd.canonicalize().with_context(|| {
            format!(
                "Working directory unavailable: {}. If it was moved, restore access at this path or open the project at its new location",
                requested_cwd.display()
            )
        })?;
        ensure!(cwd.is_dir(), "Working directory is not a directory");
        Ok(cwd)
    }

    fn ensure_helper_available(&self) -> Result<()> {
        let helper = &self.helper.executable;
        ensure!(
            executable_available(helper),
            "Attachment helper unavailable: {}. Open Settings → Updates → Installation to repair Terminator. Existing sessions are preserved",
            helper.display()
        );
        Ok(())
    }
}
