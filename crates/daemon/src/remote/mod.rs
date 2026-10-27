//! SSH project services. No remote path is passed to the local filesystem.
mod files;
mod forwards;
mod git;
mod host_keys;
pub(crate) mod terminal;
pub(crate) mod transfer;
mod transfers;
use crate::server::{Shared, relock};
use anyhow::{Context, Result, ensure};
pub(crate) use forwards::Forwards;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use terminator_core::{
    CommandOptions,
    remote::{Operation, RemoteProject, Reply},
    run_command,
};
pub(crate) use transfers::Transfers;

pub(super) fn command(
    ssh: &terminator_core::ssh::SshOptions,
    control: Option<&Path>,
) -> Result<Command> {
    let mut command = Command::new("ssh");
    command.args(terminator_core::ssh::arguments(ssh, false, control)?);
    for key in terminator_core::ssh::private_environment_keys() {
        command.env_remove(key);
    }
    command.arg("--").arg(&ssh.host);
    Ok(command)
}
pub(super) fn execute(
    project: &RemoteProject,
    control: Option<&Path>,
    script: &str,
    limit: usize,
) -> Result<Vec<u8>> {
    project.validate()?;
    let mut command = command(&project.ssh, control)?;
    command.arg(format!("exec sh -c {}", terminator_core::quote(script)));
    let output = run_command(
        command,
        CommandOptions {
            timeout: Duration::from_secs(5),
            stdout_limit: limit,
            stderr_limit: STDERR_LIMIT,
            accepted_exit_codes: None,
            ..CommandOptions::default()
        },
    )?;
    checked(output.status, &output.stderr)?;
    Ok(output.stdout)
}
pub(crate) const STDERR_LIMIT: usize = 16 * 1024;
/// OpenSSH exits 255 for its own failures; those become a classified
/// [`SshError`](terminator_core::ssh::SshError) the GUI can act on.
pub(crate) fn checked(status: std::process::ExitStatus, stderr: &[u8]) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    if status.code() == Some(255)
        && let Some(error) = terminator_core::ssh::SshError::from_stderr(stderr)
    {
        return Err(error.into());
    }
    anyhow::bail!(
        "Command failed ({status}): {}",
        String::from_utf8_lossy(stderr)
    )
}
/// Reads at most [`STDERR_LIMIT`] bytes of an SFTP child's stderr and
/// discards the rest, so OpenSSH never blocks on a full pipe.
pub(crate) fn capture_stderr(
    child: &mut tokio::process::Child,
) -> tokio::task::JoinHandle<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let stderr = child.stderr.take();
    tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(mut stderr) = stderr {
            let limit = u64::try_from(STDERR_LIMIT).unwrap_or(u64::MAX);
            let _ = (&mut stderr).take(limit).read_to_end(&mut bytes).await;
            let _ = tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await;
        }
        bytes
    })
}
/// After the SFTP child exited: a classified SSH failure replaces `error`,
/// otherwise stderr is appended to it for context.
pub(crate) async fn sftp_failure(
    error: anyhow::Error,
    stderr: tokio::task::JoinHandle<Vec<u8>>,
) -> anyhow::Error {
    let Ok(Ok(stderr)) = tokio::time::timeout(Duration::from_secs(1), stderr).await else {
        return error;
    };
    if let Some(ssh) = terminator_core::ssh::SshError::from_stderr(&stderr) {
        return ssh.into();
    }
    let text = String::from_utf8_lossy(&stderr);
    let text = text.trim();
    if text.is_empty() {
        error
    } else {
        anyhow::anyhow!("{error:#}. SSH reported: {text}")
    }
}
/// Stops every OpenSSH multiplexing master under `runtime`. Best-effort and
/// bounded: a hung master must not hold up daemon exit.
pub(crate) fn exit_controls(runtime: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        let Ok(entries) = std::fs::read_dir(runtime) else {
            return;
        };
        let mut children: Vec<_> = entries
            .flatten()
            .filter(|entry| {
                entry.file_name().to_str().is_some_and(|name| {
                    name.strip_prefix("ssh-").is_some_and(|hex| {
                        hex.len() == 8 && hex.chars().all(|c| c.is_ascii_hexdigit())
                    })
                }) && entry.file_type().is_ok_and(|t| t.is_socket())
            })
            .filter_map(|entry| {
                terminator_core::ssh::exit_control(&entry.path())
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .ok()
            })
            .collect();
        let deadline = std::time::Instant::now()
            .checked_add(Duration::from_secs(2))
            .unwrap_or_else(std::time::Instant::now);
        while !children.is_empty() && std::time::Instant::now() < deadline {
            children.retain_mut(|child| matches!(child.try_wait(), Ok(None)));
            std::thread::sleep(Duration::from_millis(20));
        }
        for mut child in children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    #[cfg(not(unix))]
    let _ = runtime;
}
/// Whether a remaining SSH project, live terminal or live SSH-tunneled VNC
/// session resolves to `control`.
pub(crate) fn control_in_use(
    state: &terminator_core::State,
    control: &Path,
    resolve: impl Fn(&terminator_core::ssh::SshOptions) -> Option<PathBuf>,
) -> bool {
    let uses = |ssh: &terminator_core::ssh::SshOptions| resolve(ssh).as_deref() == Some(control);
    state.remote_projects.iter().any(|p| uses(&p.ssh))
        || state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live())
            .filter_map(|s| s.remote.as_ref())
            .any(|r| uses(&r.ssh))
        || state.vnc_sessions.iter().any(|s| {
            s.status.live()
                && s.options.transport == terminator_core::vnc::Transport::Ssh
                && uses(&s.options.ssh)
        })
}
/// Best-effort `ssh -O exit` that never blocks the request: a detached
/// thread reaps it, killing a master that does not answer in 2 s.
fn stop_control(control: &Path) {
    spawn_bounded(
        terminator_core::ssh::exit_control(control),
        "ssh-control-exit",
    );
}
/// Runs a short OpenSSH control command without waiting for it: a detached
/// thread reaps it and kills it after 2 s.
fn spawn_bounded(mut command: Command, thread: &str) {
    #[cfg(unix)]
    {
        let Ok(mut child) = command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        else {
            return;
        };
        let _ = std::thread::Builder::new()
            .name(thread.into())
            .spawn(move || {
                let deadline = std::time::Instant::now()
                    .checked_add(Duration::from_secs(2))
                    .unwrap_or_else(std::time::Instant::now);
                while matches!(child.try_wait(), Ok(None)) {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                let _ = child.wait();
            });
    }
    #[cfg(not(unix))]
    let _ = (&mut command, thread);
}
impl Shared {
    /// Masters are shared across generations; only the last serving
    /// generation may stop them.
    pub(crate) fn exit_ssh_controls(&self) {
        let root = self.catalog_paths.as_ref().unwrap_or(&self.paths);
        if let Some(catalog) = &self.catalog_paths {
            let mine = relock(&self.state).generation.clone();
            let last = terminator_core::generations::Catalog::open(catalog)
                .and_then(|c| c.generations())
                .is_ok_and(|all| {
                    all.iter().all(|g| {
                        g.id == mine || g.status == terminator_core::generations::Status::Retired
                    })
                });
            if !last {
                return;
            }
        }
        exit_controls(&root.runtime);
    }
    /// Control sockets live in the catalog runtime so generation handoffs
    /// keep reusing them.
    pub(crate) fn ssh_control(&self, ssh: &terminator_core::ssh::SshOptions) -> Option<PathBuf> {
        self.catalog_paths
            .as_ref()
            .unwrap_or(&self.paths)
            .ssh_control(ssh)
    }
    pub(crate) fn remote_project_operation(
        self: &std::sync::Arc<Self>,
        id: &str,
        operation: Operation,
    ) -> Result<Reply> {
        let project = relock(&self.state)
            .remote_projects
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .context("Unknown SSH project")?;
        if let Operation::TransferStatus { id } = &operation {
            return self.remote_transfer_status(&project.id, id);
        }
        if operation.starts_transfer() {
            return self.start_remote_transfer(project, operation);
        }
        let control = self.ssh_control(&project.ssh);
        let control = control.as_deref();
        match operation {
            Operation::GitStatus => git::status(&project, control),
            Operation::GitDiff { path, staged } => git::diff(&project, control, &path, staged),
            operation => files::operation(&project, control, operation),
        }
    }
    /// Forgets `id`. Refused while a terminal, editor or VNC session still
    /// belongs to it. Its OpenSSH master stops unless something else that
    /// remains resolves to the same control socket.
    pub(crate) fn remove_remote_project(&self, id: &str) -> Result<()> {
        let control = {
            let mut state = relock(&self.state);
            let index = state
                .remote_projects
                .iter()
                .position(|p| p.id == id)
                .context("Unknown SSH project")?;
            ensure!(
                !state
                    .sessions
                    .iter()
                    .any(|s| s.project_id == id && s.lifecycle.live())
                    && !state
                        .vnc_sessions
                        .iter()
                        .any(|s| s.project_id == id && s.status.live()),
                "Close this SSH project's terminals and VNC sessions before removing it"
            );
            ensure!(
                relock(&self.remote_transfers).active(id) == 0,
                "Wait for this SSH project's file transfers to finish before removing it"
            );
            if let Some(paths) = &self.catalog_paths {
                terminator_core::generations::Catalog::open(paths)?.remove_remote_project(id)?;
            }
            // State, then forwards: the established lock order.
            self.drop_project_forwards(id);
            let project = state.remote_projects.remove(index);
            if state.selected_project.as_deref() == Some(id) {
                state.selected_project = None;
            }
            state.revision = state.revision.saturating_add(1);
            self.ssh_control(&project.ssh)
                .filter(|control| !control_in_use(&state, control, |ssh| self.ssh_control(ssh)))
        };
        if let Some(control) = control {
            stop_control(&control);
        }
        Ok(())
    }
    pub(crate) fn scan_host_key(
        &self,
        ssh: &terminator_core::ssh::SshOptions,
    ) -> Result<terminator_core::ssh::HostKeyScan> {
        host_keys::scan(ssh)
    }
    pub(crate) fn trust_host_key(
        &self,
        ssh: &terminator_core::ssh::SshOptions,
        keys: &[terminator_core::ssh::HostKey],
    ) -> Result<()> {
        host_keys::trust(ssh, keys)
    }
    pub(crate) fn probe_remote_project(&self, project: &RemoteProject) -> Result<Reply> {
        // `HOME_ROOT` asks for the login directory: `cd` with no operand.
        let (project, script) = if project.root == terminator_core::remote::HOME_ROOT {
            let mut home = project.clone();
            home.root = "/".into();
            (home, "cd && pwd -P".to_owned())
        } else {
            let script = format!("cd -- {} && pwd -P", terminator_core::quote(&project.root));
            (project.clone(), script)
        };
        let project = &project;
        project.validate()?;
        let bytes = execute(
            project,
            self.ssh_control(&project.ssh).as_deref(),
            &script,
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

#[cfg(test)]
mod tests {
    use super::*;
    use terminator_core::ssh::SshOptions;
    fn project(id: &str, host: &str) -> RemoteProject {
        RemoteProject {
            id: id.into(),
            name: id.into(),
            ssh: SshOptions {
                host: host.into(),
                ..SshOptions::default()
            },
            root: "/".into(),
            layout: serde_json::Value::Null,
            group: None,
            forwards: Vec::new(),
        }
    }
    #[test]
    fn a_shared_control_socket_stays_up_while_another_project_uses_it() {
        let paths = terminator_core::Paths {
            data: "/unused".into(),
            runtime: "/tmp/terminator-fixture".into(),
        };
        let resolve = |ssh: &SshOptions| paths.ssh_control(ssh);
        let a = project("a", "alpha");
        let control = paths.ssh_control(&a.ssh).unwrap();
        let mut state = terminator_core::State {
            remote_projects: vec![project("b", "beta")],
            ..terminator_core::State::default()
        };
        assert!(!control_in_use(&state, &control, resolve));
        state.remote_projects.push(project("c", "alpha"));
        assert!(control_in_use(&state, &control, resolve));
    }
}
