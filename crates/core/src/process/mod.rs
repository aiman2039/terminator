//! Bounded subprocess I/O; no reader threads survive a deadline.
//!
//! Unix uses nonblocking pipes drained on the calling thread. Windows pipes
//! cannot be polled from std, so bounded reader threads drain them instead.
#[cfg(unix)]
use std::os::{fd::AsFd, unix::process::CommandExt};
use std::{
    io::{Read, Write},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};
/// Why a bounded subprocess failed, timed out, or produced too much output.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("{pipe} pipe missing")]
    MissingPipe { pipe: &'static str },
    #[cfg(unix)]
    #[error("Set nonblocking pipe failed")]
    Nonblocking { errno: rustix::io::Errno },
    #[cfg(unix)]
    #[error("Command output exceeds {limit} bytes")]
    OutputTooLarge { limit: usize },
    #[cfg(not(unix))]
    #[error("Command output exceeds the configured limit")]
    OutputOverLimit,
    #[error("Command failed ({status}): {stderr}")]
    Failed {
        status: std::process::ExitStatus,
        stderr: String,
    },
    #[error("Command timed out after {timeout:?}")]
    TimedOut { timeout: Duration },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
type Result<T, E = ProcessError> = std::result::Result<T, E>;

/// How long a timed-out or cancelled command may clean up after Ctrl-C
/// (Git removes its lock files) before its process group is killed.
pub const INTERRUPT_GRACE: Duration = Duration::from_secs(5);

/// How long the pipes are still read after the child exited. A background
/// process (for example one a Git hook started) can hold them open; the
/// command still finished.
pub const EXIT_DRAIN: Duration = Duration::from_millis(500);

/// `Err` unless `status` is one of `accepted` (any status when `None`).
fn check_status(
    status: std::process::ExitStatus,
    accepted: Option<&[i32]>,
    stderr: &[u8],
) -> Result<()> {
    if let Some(accepted) = accepted
        && !status.code().is_some_and(|c| accepted.contains(&c))
    {
        return Err(ProcessError::Failed {
            status,
            stderr: String::from_utf8_lossy(stderr).into_owned(),
        });
    }
    Ok(())
}

pub struct CommandOptions {
    pub timeout: Duration,
    pub input: Option<Vec<u8>>,
    pub stdout_limit: usize,
    pub stderr_limit: usize,
    pub accepted_exit_codes: Option<Vec<i32>>,
}
impl Default for CommandOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            input: None,
            stdout_limit: 1024 * 1024,
            stderr_limit: 65536,
            accepted_exit_codes: Some(vec![0]),
        }
    }
}
#[cfg(unix)]
fn nonblocking(fd: impl AsFd) -> Result<()> {
    let failed = |errno| ProcessError::Nonblocking { errno };
    let flags = rustix::fs::fcntl_getfl(&fd).map_err(failed)?;
    rustix::fs::fcntl_setfl(&fd, flags | rustix::fs::OFlags::NONBLOCK).map_err(failed)?;
    Ok(())
}
#[cfg(unix)]
fn drain(pipe: &mut impl Read, bytes: &mut Vec<u8>, limit: usize) -> Result<bool> {
    let mut buf = [0; 8192];
    for _ in 0..32 {
        match pipe.read(&mut buf) {
            Ok(0) => return Ok(true),
            Ok(n) => {
                if bytes.len().saturating_add(n) > limit {
                    return Err(ProcessError::OutputTooLarge { limit });
                }
                if let Some(chunk) = buf.get(..n) {
                    bytes.extend_from_slice(chunk);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(false)
}
/// Writes what the pipe takes of `bytes` past `written`, closing `input`
/// once all is written.
#[cfg(unix)]
fn pump_input(
    input: &mut Option<std::process::ChildStdin>,
    bytes: &[u8],
    written: &mut usize,
) -> Result<()> {
    let Some(stdin) = input else {
        return Ok(());
    };
    let rest = bytes.get(*written..).unwrap_or_default();
    match stdin.write(rest) {
        Ok(n) => *written = written.saturating_add(n),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => *written = bytes.len(),
        Err(e) => return Err(e.into()),
    }
    if *written == bytes.len() {
        *input = None;
    }
    Ok(())
}
#[cfg(unix)]
pub fn run_command(mut cmd: Command, options: CommandOptions) -> Result<Output> {
    cmd.process_group(0)
        .stdin(if options.input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let result = (|| {
        let mut out = child
            .stdout
            .take()
            .ok_or(ProcessError::MissingPipe { pipe: "stdout" })?;
        let mut err = child
            .stderr
            .take()
            .ok_or(ProcessError::MissingPipe { pipe: "stderr" })?;
        let mut input = child.stdin.take();
        nonblocking(&out)?;
        nonblocking(&err)?;
        if let Some(stdin) = &input {
            nonblocking(stdin)?;
        }
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut written = 0;
        let started = Instant::now();
        let mut status = None;
        let mut exited = None;
        loop {
            let out_done = drain(&mut out, &mut stdout, options.stdout_limit)?;
            let err_done = drain(&mut err, &mut stderr, options.stderr_limit)?;
            let bytes = options.input.as_deref().unwrap_or(&[]);
            pump_input(&mut input, bytes, &mut written)?;
            if status.is_none() {
                status = child.try_wait()?;
                if status.is_some() {
                    exited = Some(Instant::now());
                }
            }
            let drained = (out_done && err_done)
                || exited.is_some_and(|exited: Instant| exited.elapsed() >= EXIT_DRAIN);
            if let Some(status) = status
                && drained
            {
                check_status(status, options.accepted_exit_codes.as_deref(), &stderr)?;
                return Ok(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            if started.elapsed() >= options.timeout {
                return Err(ProcessError::TimedOut {
                    timeout: options.timeout,
                });
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    if matches!(result, Err(ProcessError::TimedOut { .. })) {
        interrupt_and_wait(&mut child);
    }
    if result.is_err() {
        let _ = crate::signals::signal_group(child.id(), crate::signals::ProcSignal::Kill);
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

/// Ctrl-C to the process group, as in a terminal, so Git removes its lock
/// files; waits up to [`INTERRUPT_GRACE`] for the child to exit.
#[cfg(unix)]
fn interrupt_and_wait(child: &mut std::process::Child) {
    if crate::signals::signal_group(child.id(), crate::signals::ProcSignal::Interrupt).is_err() {
        return;
    }
    let started = Instant::now();
    while started.elapsed() < INTERRUPT_GRACE {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Bounded reader threads drain stdout/stderr so a verbose child cannot wedge
/// the caller; each stops past its limit, which surfaces as an overflow error.
#[cfg(not(unix))]
pub fn run_command(mut cmd: Command, options: CommandOptions) -> Result<Output> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    cmd.stdin(if options.input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let done = Arc::new(AtomicBool::new(false));
    let readers = spawn_output_readers(&mut child, &options, &done);
    // Start the timeout clock before touching stdin: a child that never reads
    // must not wedge the caller in a blocking write. The writer thread owns
    // stdin and exits when the write completes or the timed-out child dies.
    let started = Instant::now();
    spawn_stdin_writer(&mut child, &options);
    let (status, timed_out) = wait_for_child(&mut child, &options, started)?;
    done.store(true, Ordering::Relaxed);
    let (stdout, stderr, overflow) = collect_output(readers);
    if overflow {
        return Err(ProcessError::OutputOverLimit);
    }
    if timed_out {
        return Err(ProcessError::TimedOut {
            timeout: options.timeout,
        });
    }
    check_status(status, options.accepted_exit_codes.as_deref(), &stderr)?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

#[cfg(not(unix))]
fn spawn_stdin_writer(child: &mut std::process::Child, options: &CommandOptions) {
    if let Some(mut stdin) = child.stdin.take()
        && let Some(input) = options.input.clone()
    {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
}

#[cfg(not(unix))]
fn spawn_output_readers(
    child: &mut std::process::Child,
    options: &CommandOptions,
    done: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Vec<std::sync::mpsc::Receiver<(Vec<u8>, bool)>> {
    use std::sync::{Arc, atomic::Ordering};
    let mut readers = Vec::new();
    for (pipe, limit) in [
        (
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
            options.stdout_limit,
        ),
        (
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
            options.stderr_limit,
        ),
    ] {
        let Some(mut pipe) = pipe else { continue };
        let done = Arc::clone(done);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut buf = [0; 8192];
            let mut overflow = false;
            while !done.load(Ordering::Relaxed) {
                match pipe.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if bytes.len().saturating_add(n) > limit {
                            overflow = true;
                            break;
                        }
                        if let Some(chunk) = buf.get(..n) {
                            bytes.extend_from_slice(chunk);
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tx.send((bytes, overflow));
        });
        readers.push(rx);
    }
    readers
}

#[cfg(not(unix))]
fn wait_for_child(
    child: &mut std::process::Child,
    options: &CommandOptions,
    started: Instant,
) -> Result<(std::process::ExitStatus, bool)> {
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= options.timeout {
            timed_out = true;
            break loop {
                let _ = child.kill();
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if started.elapsed()
                    >= options
                        .timeout
                        .checked_add(Duration::from_secs(2))
                        .unwrap_or(Duration::MAX)
                {
                    return Err(ProcessError::TimedOut {
                        timeout: options.timeout,
                    });
                }
                std::thread::sleep(Duration::from_millis(5));
            };
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    Ok((status, timed_out))
}

#[cfg(not(unix))]
fn collect_output(
    readers: Vec<std::sync::mpsc::Receiver<(Vec<u8>, bool)>>,
) -> (Vec<u8>, Vec<u8>, bool) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut overflow = false;
    // A grandchild can inherit the pipes and hold them open past the kill;
    // don't let its output wedge the caller: take what arrived, then move on.
    // The orphaned reader exits when the last pipe handle closes.
    for (i, rx) in readers.into_iter().enumerate() {
        let (bytes, limited) = rx.recv_timeout(Duration::from_secs(2)).unwrap_or_default();
        overflow |= limited;
        if i == 0 {
            stdout = bytes;
        } else {
            stderr = bytes;
        }
    }
    (stdout, stderr, overflow)
}

/// Detach so the process is not a child of the GUI (double-fork + setsid on
/// Unix; a new process group on Windows, which has no fork).
pub fn spawn_session_leader(mut command: Command) -> Result<std::process::Child> {
    terminator_sys::double_fork_setsid(&mut command);
    Ok(command.spawn()?)
}

#[cfg(test)]
mod tests;
