//! Bounded subprocess I/O; no reader threads survive a deadline.
//!
//! Unix uses nonblocking pipes drained on the calling thread. Windows pipes
//! cannot be polled from std, so bounded reader threads drain them instead.
#[cfg(unix)]
use anyhow::Context;
use anyhow::{Result, bail, ensure};
#[cfg(unix)]
use std::os::{fd::AsFd, unix::process::CommandExt};
use std::{
    io::{Read, Write},
    process::{Command, Output, Stdio},
    time::{Duration, Instant},
};
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
    let flags =
        rustix::fs::fcntl_getfl(&fd).map_err(|_| anyhow::anyhow!("Set nonblocking pipe failed"))?;
    rustix::fs::fcntl_setfl(&fd, flags | rustix::fs::OFlags::NONBLOCK)
        .map_err(|_| anyhow::anyhow!("Set nonblocking pipe failed"))?;
    Ok(())
}
#[cfg(unix)]
fn drain(pipe: &mut impl Read, bytes: &mut Vec<u8>, limit: usize) -> Result<bool> {
    let mut buf = [0; 8192];
    for _ in 0..32 {
        match pipe.read(&mut buf) {
            Ok(0) => return Ok(true),
            Ok(n) => {
                ensure!(
                    bytes.len().saturating_add(n) <= limit,
                    "Command output exceeds {limit} bytes"
                );
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
        let mut out = child.stdout.take().context("stdout pipe missing")?;
        let mut err = child.stderr.take().context("stderr pipe missing")?;
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
        loop {
            let out_done = drain(&mut out, &mut stdout, options.stdout_limit)?;
            let err_done = drain(&mut err, &mut stderr, options.stderr_limit)?;
            if let Some(stdin) = &mut input {
                let bytes = options.input.as_deref().unwrap_or(&[]);
                let Some(rest) = bytes.get(written..) else {
                    input = None;
                    continue;
                };
                match stdin.write(rest) {
                    Ok(n) => written = written.saturating_add(n),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => written = bytes.len(),
                    Err(e) => return Err(e.into()),
                }
                if written == bytes.len() {
                    input = None;
                }
            }
            if status.is_none() {
                status = child.try_wait()?;
            }
            if let Some(status) = status
                && out_done
                && err_done
            {
                if let Some(accepted) = &options.accepted_exit_codes {
                    ensure!(
                        status.code().is_some_and(|c| accepted.contains(&c)),
                        "Command failed ({status}): {}",
                        String::from_utf8_lossy(&stderr)
                    );
                }
                return Ok(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            if started.elapsed() >= options.timeout {
                bail!("Command timed out after {:?}", options.timeout);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    if result.is_err() {
        let _ = crate::signals::signal_group(child.id(), crate::signals::ProcSignal::Kill);
        let _ = child.kill();
        let _ = child.wait();
    }
    result
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
        let done = Arc::clone(&done);
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
    // Start the timeout clock before touching stdin: a child that never reads
    // must not wedge the caller in a blocking write. The writer thread owns
    // stdin and exits when the write completes or the timed-out child dies.
    let started = Instant::now();
    if let Some(mut stdin) = child.stdin.take()
        && let Some(input) = options.input
    {
        std::thread::spawn(move || {
            let _ = stdin.write_all(&input);
        });
    }
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
                    bail!("Command timed out after {:?}", options.timeout);
                }
                std::thread::sleep(Duration::from_millis(5));
            };
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    done.store(true, Ordering::Relaxed);
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
    ensure!(!overflow, "Command output exceeds the configured limit");
    if timed_out {
        bail!("Command timed out after {:?}", options.timeout);
    }
    if let Some(accepted) = &options.accepted_exit_codes {
        ensure!(
            status.code().is_some_and(|c| accepted.contains(&c)),
            "Command failed ({status}): {}",
            String::from_utf8_lossy(&stderr)
        );
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Detach so the process is not a child of the GUI (double-fork + setsid on
/// Unix; a new process group on Windows, which has no fork).
pub fn spawn_session_leader(mut command: Command) -> Result<std::process::Child> {
    terminator_sys::double_fork_setsid(&mut command);
    Ok(command.spawn()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    fn shell(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.args(["-c", script]);
        c
    }
    #[cfg(not(unix))]
    fn shell(script: &str) -> Command {
        let mut c = Command::new("cmd");
        c.args(["/c", script]);
        c
    }
    #[cfg(unix)]
    #[test]
    fn timeout_includes_inherited_pipes() {
        let start = Instant::now();
        assert!(
            run_command(
                shell("sleep 10 & exit 0"),
                CommandOptions {
                    timeout: Duration::from_millis(80),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .to_string()
            .contains("timed out")
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    /// A child that never reads stdin must not wedge the caller in a blocking
    /// write: the timeout clock starts before stdin is touched.
    #[cfg(windows)]
    #[test]
    fn blocked_stdin_still_times_out() {
        let start = Instant::now();
        let error = run_command(
            shell("ping -n 30 127.0.0.1 > nul"),
            CommandOptions {
                timeout: Duration::from_secs(3),
                input: Some(vec![b'x'; 4 * 1024 * 1024]),
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("timed out"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(20));
    }

    #[cfg(unix)]
    #[test]
    fn overflow_is_explicit_for_both_streams() {
        for script in ["yes x", "yes x >&2"] {
            assert!(
                run_command(
                    shell(script),
                    CommandOptions {
                        stdout_limit: 100,
                        stderr_limit: 100,
                        ..Default::default()
                    }
                )
                .unwrap_err()
                .to_string()
                .contains("exceeds")
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn session_leader_pid_equals_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let ready = dir.path().join("ready");
        let staging = dir.path().join("ready.tmp");
        let mut command = Command::new("sh");
        command.arg("-c").arg(format!(
            "echo $$ > {staging} && mv {staging} {ready}; exec sleep 8",
            staging = staging.display(),
            ready = ready.display()
        ));
        let mut child = spawn_session_leader(command).unwrap();
        assert!(child.wait().unwrap().success());
        let start = Instant::now();
        let pid = loop {
            assert!(start.elapsed() < Duration::from_secs(3));
            if let Ok(contents) = std::fs::read_to_string(&ready)
                && let Ok(pid) = contents.trim().parse::<i32>()
            {
                break pid;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let tracked = rustix::process::Pid::from_raw(pid).unwrap();
        let sid = rustix::process::getsid(Some(tracked)).unwrap();
        assert_eq!(sid.as_raw_pid(), pid);
        crate::signals::signal_process(
            u32::try_from(pid).unwrap(),
            crate::signals::ProcSignal::Kill,
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stdin_and_accepted_exit_codes() {
        let o = run_command(
            shell("cat; exit 1"),
            CommandOptions {
                input: Some(b"hello".to_vec()),
                accepted_exit_codes: Some(vec![0, 1]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(o.stdout, b"hello");
    }

    #[cfg(not(unix))]
    #[test]
    fn timeout_kills_a_sleeping_child() {
        let start = Instant::now();
        assert!(
            run_command(
                shell("ping -n 10 127.0.0.1 >nul"),
                CommandOptions {
                    timeout: Duration::from_millis(300),
                    ..Default::default()
                }
            )
            .unwrap_err()
            .to_string()
            .contains("timed out")
        );
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[cfg(not(unix))]
    #[test]
    fn echo_returns_its_output() {
        let o = run_command(
            shell("echo hello"),
            CommandOptions {
                ..Default::default()
            },
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&o.stdout).contains("hello"));
    }
}
