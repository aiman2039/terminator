//! Nonblocking Unix subprocess I/O; no reader threads survive a deadline.
use anyhow::{Result, bail, ensure};
use std::{
    io::{Read, Write},
    os::{fd::AsFd, unix::process::CommandExt},
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
fn nonblocking(fd: impl AsFd) -> Result<()> {
    let flags =
        rustix::fs::fcntl_getfl(&fd).map_err(|_| anyhow::anyhow!("Set nonblocking pipe failed"))?;
    rustix::fs::fcntl_setfl(&fd, flags | rustix::fs::OFlags::NONBLOCK)
        .map_err(|_| anyhow::anyhow!("Set nonblocking pipe failed"))?;
    Ok(())
}
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
                bytes.extend_from_slice(&buf[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(false)
}
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
        let mut out = child.stdout.take().unwrap();
        let mut err = child.stderr.take().unwrap();
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
                let bytes = options.input.as_deref().unwrap_or_default();
                match stdin.write(&bytes[written..]) {
                    Ok(n) => written += n,
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

/// Double-fork + setsid so the process is not a child of the GUI.
pub fn spawn_session_leader(mut command: Command) -> Result<std::process::Child> {
    terminator_sys::double_fork_setsid(&mut command);
    Ok(command.spawn()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shell(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.args(["-c", script]);
        c
    }
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
    #[test]
    fn session_leader_pid_equals_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let ready = dir.path().join("ready");
        let staging = dir.path().join("ready.tmp");
        let mut command = Command::new("sh");
        command.arg("-c").arg(format!(
            "echo $$ > {staging} && mv {staging} {ready}; sleep 8",
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
}
