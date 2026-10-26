//! SSH project services. No remote path is passed to the local filesystem.
mod files;
mod git;
pub(crate) mod terminal;
use crate::server::{Shared, relock};
use anyhow::{Context, Result, ensure};
use std::{process::Command, time::Duration};
use terminator_core::{
    CommandOptions,
    remote::{Operation, RemoteProject, Reply},
    run_command,
};

pub(super) fn command(ssh: &terminator_core::ssh::SshOptions) -> Result<Command> {
    let mut command = Command::new("ssh");
    command.args(terminator_core::ssh::arguments(ssh, false)?);
    for key in terminator_core::ssh::private_environment_keys() {
        command.env_remove(key);
    }
    command.arg("--").arg(&ssh.host);
    Ok(command)
}
pub(super) fn execute(project: &RemoteProject, script: &str, limit: usize) -> Result<Vec<u8>> {
    project.validate()?;
    let mut command = command(&project.ssh)?;
    command.arg(format!("exec sh -c {}", terminator_core::quote(script)));
    Ok(run_command(
        command,
        CommandOptions {
            timeout: Duration::from_secs(5),
            stdout_limit: limit,
            stderr_limit: 16 * 1024,
            ..CommandOptions::default()
        },
    )?
    .stdout)
}
impl Shared {
    pub(crate) fn remote_project_operation(&self, id: &str, operation: Operation) -> Result<Reply> {
        let project = relock(&self.state)
            .remote_projects
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .context("Unknown SSH project")?;
        match operation {
            Operation::GitStatus => git::status(&project),
            Operation::GitDiff { path, staged } => git::diff(&project, &path, staged),
            operation => files::operation(&project, operation),
        }
    }
    pub(crate) fn probe_remote_project(&self, project: &RemoteProject) -> Result<Reply> {
        project.validate()?;
        let bytes = execute(
            project,
            &format!("cd -- {} && pwd -P", terminator_core::quote(&project.root)),
            8192,
        )?;
        let root = std::str::from_utf8(&bytes)?
            .trim_end_matches('\n')
            .to_owned();
        ensure!(
            root.starts_with('/') && !root.contains('\n'),
            "SSH server returned an invalid project root"
        );
        Ok(Reply::Probe { root })
    }
}
