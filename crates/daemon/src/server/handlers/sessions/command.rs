use anyhow::{Context, Result, ensure};
use portable_pty::CommandBuilder;
use std::path::PathBuf;
use terminator_core::*;

use super::super::super::shared::Shared;
use super::Target;
use crate::{Launch, editor, review, shell, terminal_env};

pub(super) struct LaunchInput<'a> {
    pub target: &'a Target,
    pub launch: Launch,
    pub sid: &'a str,
    pub file: Option<PathBuf>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

/// The program a session runs, plus what preparing it resolved.
pub(super) struct PreparedCommand {
    pub command: CommandBuilder,
    pub remote_session: Option<remote::RemoteSession>,
    /// The session file; remote preparation resolves it against the project root.
    pub file: Option<PathBuf>,
}

pub(super) struct EnvInput<'a> {
    pub command: &'a mut CommandBuilder,
    pub target: &'a Target,
    pub sid: &'a str,
    pub token: &'a str,
}

impl Shared {
    pub(super) fn launch_command(&self, input: LaunchInput<'_>) -> Result<PreparedCommand> {
        let LaunchInput {
            target,
            launch,
            sid,
            file,
            line,
            column,
        } = input;
        if let Some(remote) = &target.remote_project {
            ensure!(
                !matches!(launch, Launch::Review { .. }),
                "Remote Git reviews use the native read-only viewer"
            );
            let (command, metadata, resolved) =
                crate::remote::terminal::prepare(crate::remote::terminal::PrepareInput {
                    project: remote,
                    control: self.ssh_control(&remote.ssh).as_deref(),
                    sid,
                    cwd: target.requested_cwd.as_deref(),
                    file: file.as_deref(),
                    line,
                    column,
                })?;
            return Ok(PreparedCommand {
                command,
                remote_session: Some(metadata),
                file: resolved,
            });
        }
        let command = match launch {
            Launch::Review { staged } => review::prepare(review::PrepareInput {
                paths: &self.paths,
                sid,
                cwd: &target.cwd,
                file: file.as_ref().context("Review requires a file")?,
                staged,
            })?,
            Launch::Editor => editor::prepare(editor::PrepareInput {
                paths: &self.paths,
                sid,
                settings: &target.settings,
                file: file.as_ref().context("Editor requires a file")?,
                line,
                column,
            })?,
            Launch::Shell => self.shell_command(&target.settings)?,
        };
        Ok(PreparedCommand {
            command,
            remote_session: None,
            file,
        })
    }

    fn shell_command(&self, settings: &Settings) -> Result<CommandBuilder> {
        let shell = if settings.shell.trim().is_empty() {
            default_shell()?
        } else {
            find_executable(&settings.shell)
                .context("Configured shell not found; clear the override to use zsh → bash → sh")?
        };
        shell::prepare(
            &self.paths,
            &shell.to_string_lossy(),
            &self.helper.executable,
        )
    }

    /// Sets the working directory and environment every session starts with.
    pub(super) fn apply_session_environment(&self, input: EnvInput<'_>) {
        let EnvInput {
            command,
            target,
            sid,
            token,
        } = input;
        command.cwd(if target.remote_project.is_some() {
            &self.paths.data
        } else {
            &target.cwd
        });
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        terminal_env::restore_colors(command);
        if target.remote_project.is_none() {
            command.env("TERMINATOR_SESSION_ID", sid);
            command.env("TERMINATOR_SESSION_TOKEN", token);
            command.env("TERMINATOR_DATA_DIR", &self.paths.data);
            command.env("TERMINATOR_RUNTIME_DIR", &self.paths.runtime);
        } else {
            for key in ssh::private_environment_keys() {
                command.env_remove(key);
            }
        }
    }
}
