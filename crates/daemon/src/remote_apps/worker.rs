//! Runs the host bridge over the project's SSH master and moves messages
//! between it and the session's [`Runtime`].

use std::io::{BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use terminator_core::remote::RemoteProject;
use terminator_core::remote_apps::{
    ClientMessage, DEFAULT_COMMAND, ErrorKind, HostMessage, RemoteAppsStatus, permanent, protocol,
    retry_delay_seconds,
};

use super::Handle;

/// Most stderr kept for error messages.
const STDERR_LIMIT: usize = 16 * 1024;
/// How often the detach timeout is checked.
const DETACH_CHECK: Duration = Duration::from_secs(10);

/// The detach timeout now; `None` is never. Read on every check so a
/// Settings change applies to live sessions.
pub(super) type DetachLimit = Box<dyn Fn() -> Option<Duration> + Send>;

/// How one bridge run ended.
enum RunEnd {
    /// ssh could not start: retrying will not help.
    NoSsh(String),
    Exited {
        code: Option<i32>,
        stderr: Vec<u8>,
    },
}

/// Runs the bridge until the user disconnects, the detach timeout passes,
/// or a failure that a retry cannot fix. Temporary failures (network, SSH,
/// wprsd stopped) retry with backoff; windows come back with the same IDs.
pub(super) fn start(
    handle: Arc<Handle>,
    project: RemoteProject,
    control: Option<PathBuf>,
    detach_limit: DetachLimit,
) {
    let watcher = Arc::clone(&handle);
    thread::spawn(move || watch_detach(&watcher, &detach_limit, DETACH_CHECK));
    thread::spawn(move || {
        let mut attempt = 0_u32;
        loop {
            let end = run_once(&handle, &project, control.as_deref());
            let (stop, bridge_error, reached_wprsd) =
                handle.read(|r| (r.stop_requested, r.bridge_error(), r.hello_seen));
            if stop {
                handle.update(|r| r.end(RemoteAppsStatus::Disconnected, None));
                return;
            }
            let failure = match (bridge_error, end) {
                (Some(error), _) => error,
                (None, RunEnd::NoSsh(message)) => {
                    handle.update(|r| {
                        r.end(RemoteAppsStatus::Failed, Some((ErrorKind::Other, message)));
                    });
                    return;
                }
                (None, RunEnd::Exited { code, stderr }) => classify(code, &stderr),
            };
            if permanent(failure.0) {
                handle.update(|r| r.end(RemoteAppsStatus::Failed, Some(failure)));
                return;
            }
            attempt = if reached_wprsd {
                1
            } else {
                attempt.saturating_add(1)
            };
            let delay = retry_delay_seconds(attempt);
            handle.update(|r| r.retrying(failure, attempt, delay));
            if handle.wait_stop(Duration::from_secs(delay)) {
                handle.update(|r| r.end(RemoteAppsStatus::Disconnected, None));
                return;
            }
            handle.update(super::Runtime::retry_started);
        }
    });
}

/// Stops the session when no viewer asked for its state within the limit.
/// The apps keep running in the host's wprsd; connecting again shows them.
pub(super) fn watch_detach(handle: &Handle, limit: &DetachLimit, every: Duration) {
    loop {
        if handle.wait_stop(every) || handle.read(|r| !r.status.live()) {
            return;
        }
        let idle = handle.read(|r| r.last_viewed.elapsed());
        if limit().is_some_and(|limit| idle > limit) {
            handle.request_stop();
            return;
        }
    }
}

/// One bridge process: starts it, serves it, and waits for it to exit.
fn run_once(handle: &Handle, project: &RemoteProject, control: Option<&Path>) -> RunEnd {
    let mut child = match spawn_bridge(project, control) {
        Ok(child) => child,
        Err(error) => return RunEnd::NoSsh(format!("Cannot run ssh: {error:#}")),
    };
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = child.kill();
        let _ = child.wait();
        return RunEnd::NoSsh("ssh has no pipes".into());
    };
    let (tx, rx) = mpsc::channel();
    let stopped = handle.update_with(|r| {
        if !r.stop_requested {
            r.input = Some(tx);
        }
        r.stop_requested
    });
    if stopped {
        // Disconnect came while ssh started: close stdin at once.
        drop(stdin);
    } else {
        thread::spawn(move || write_input(stdin, &rx));
    }
    let stderr = thread::spawn(move || read_stderr(stderr));
    let mut input = BufReader::new(stdout);
    while let Ok(framed) = protocol::read_frame::<HostMessage>(&mut input) {
        handle.update(|r| {
            r.apply(framed);
        });
    }
    let code = child.wait().ok().and_then(|status| status.code());
    RunEnd::Exited {
        code,
        stderr: stderr.join().unwrap_or_default(),
    }
}

fn spawn_bridge(project: &RemoteProject, control: Option<&Path>) -> anyhow::Result<Child> {
    project.validate()?;
    let mut command = crate::remote::command(&project.ssh, control)?;
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
fn write_input(stdin: ChildStdin, rx: &mpsc::Receiver<super::runtime::Input>) {
    let mut out = BufWriter::new(stdin);
    let hello = ClientMessage::Hello {
        protocol: protocol::PROTOCOL_VERSION,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        capabilities: Vec::new(),
    };
    if protocol::write_frame(&mut out, &hello, &[]).is_err() {
        return;
    }
    while let Ok((message, payload)) = rx.recv() {
        if protocol::write_frame(&mut out, &message, &payload).is_err() {
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
