//! Runs the host bridge over the project's SSH master and moves messages
//! between it and the session's [`Runtime`].

use std::io::{BufReader, BufWriter, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, mpsc};
use std::thread;

use terminator_core::remote::RemoteProject;
use terminator_core::remote_apps::{
    ClientMessage, DEFAULT_COMMAND, ErrorKind, HostMessage, RemoteAppsStatus, protocol,
};

use super::Handle;

/// Most stderr kept for error messages.
const STDERR_LIMIT: usize = 16 * 1024;

/// Starts the bridge and the threads that serve it. Errors end the session
/// at once with a message the GUI shows.
pub(super) fn start(handle: &Arc<Handle>, project: &RemoteProject, control: Option<PathBuf>) {
    let (tx, rx) = mpsc::channel();
    let child = spawn_bridge(project, control);
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            handle.update(|r| {
                r.end(
                    RemoteAppsStatus::Failed,
                    Some((ErrorKind::Other, format!("Cannot run ssh: {error:#}"))),
                );
            });
            return;
        }
    };
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.kill();
        handle.update(|r| {
            r.end(
                RemoteAppsStatus::Failed,
                Some((ErrorKind::Other, "ssh has no pipes".into())),
            );
        });
        return;
    };
    handle.update(|r| r.input = Some(tx));
    thread::spawn(move || write_input(stdin, &rx));
    let stderr = thread::spawn(move || read_stderr(stderr));
    let handle = Arc::clone(handle);
    thread::spawn(move || {
        let mut input = BufReader::new(stdout);
        while let Ok(framed) = protocol::read_frame::<HostMessage>(&mut input) {
            handle.update(|r| {
                r.apply(framed);
            });
        }
        let status = child.wait();
        let stderr = stderr.join().unwrap_or_default();
        handle.update(|r| {
            if r.input.is_none() && r.status != RemoteAppsStatus::Failed {
                // The user disconnected.
                r.end(RemoteAppsStatus::Disconnected, None);
            } else {
                let failure = classify(status.ok().and_then(|s| s.code()), &stderr);
                r.end(RemoteAppsStatus::Failed, Some(failure));
            }
        });
    });
}

fn spawn_bridge(project: &RemoteProject, control: Option<PathBuf>) -> anyhow::Result<Child> {
    project.validate()?;
    let mut command = crate::remote::command(&project.ssh, control.as_deref())?;
    let host_command = project.apps_command.as_deref().unwrap_or(DEFAULT_COMMAND);
    // No login shell: profile output on stdout would corrupt the stream.
    command.arg(format!(
        "exec sh -c {}",
        terminator_core::quote(&format!("exec {host_command}"))
    ));
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command.spawn()?)
}

/// Sends our hello, then input until the session drops its sender; closing
/// stdin then makes the bridge exit.
fn write_input(stdin: ChildStdin, rx: &mpsc::Receiver<ClientMessage>) {
    let mut out = BufWriter::new(stdin);
    let hello = ClientMessage::Hello {
        protocol: protocol::PROTOCOL_VERSION,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        capabilities: Vec::new(),
    };
    if protocol::write_frame(&mut out, &hello, &[]).is_err() {
        return;
    }
    while let Ok(message) = rx.recv() {
        if protocol::write_frame(&mut out, &message, &[]).is_err() {
            return;
        }
    }
}

fn read_stderr(stderr: impl Read) -> Vec<u8> {
    let mut kept = Vec::new();
    let _ = stderr.take(STDERR_LIMIT as u64).read_to_end(&mut kept);
    kept
}

/// Why the bridge stopped, from its exit status and stderr.
pub(super) fn classify(code: Option<i32>, stderr: &[u8]) -> (ErrorKind, String) {
    let text = String::from_utf8_lossy(stderr);
    let tail: Vec<&str> = text.lines().rev().take(4).collect();
    let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
    if code == Some(255)
        && let Some(error) = terminator_core::ssh::SshError::from_stderr(stderr)
    {
        return (ErrorKind::Other, error.to_string());
    }
    if code == Some(127) || text.contains("not found") {
        return (
            ErrorKind::WprsdMissing,
            format!(
                "The remote-apps bridge is not installed on the host. Install or update \
                 Terminator there, or set the host's apps command. {tail}"
            ),
        );
    }
    if text.contains("server disconnected") {
        return (
            ErrorKind::WprsdStopped,
            format!(
                "wprsd stopped on the host; apps that were open there are gone. Connect again to start it. {tail}"
            ),
        );
    }
    (
        ErrorKind::Other,
        format!(
            "The remote-apps bridge stopped (exit {}). {tail}",
            code.map_or_else(|| "signal".to_owned(), |c| c.to_string())
        ),
    )
}
