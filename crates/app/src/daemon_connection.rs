//! Starting/replacing a daemon always preserves live sessions and held locks.
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use terminator_core::*;

pub fn is_connection_error(error: &str) -> bool {
    error.starts_with("Reconnecting:") || error.starts_with("Session daemon unavailable")
}

pub fn can_replace_daemon(state: &State) -> bool {
    state.daemon_version.as_deref().is_some_and(|version| {
        semver::Version::parse(version)
            .ok()
            .zip(semver::Version::parse(env!("CARGO_PKG_VERSION")).ok())
            .is_some_and(|(running, bundled)| {
                running < bundled
                    || (running == bundled
                        && (state.attachment_helper_available != Some(true)
                            || !state
                                .capabilities
                                .iter()
                                .any(|c| c == STABLE_HELPER_CAPABILITY)))
            })
    })
}

pub fn can_retire_daemon(state: &State) -> bool {
    can_replace_daemon(state)
        && state
            .capabilities
            .iter()
            .any(|c| c == SHUTDOWN_IF_IDLE_CAPABILITY)
        && !state.sessions.iter().any(|s| s.lifecycle.live())
}

pub fn can_restart_service(state: &State) -> bool {
    can_replace_daemon(state) && state.sessions.iter().any(|s| s.lifecycle.live())
}

fn wait_for_retirement(paths: &Paths) -> Result<()> {
    let start = Instant::now();
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.runtime.join("daemon.lock"))?;
    loop {
        if !paths.socket().exists() && lock.try_lock_exclusive().is_ok() {
            FileExt::unlock(&lock)?;
            return Ok(());
        }
        ensure!(
            start.elapsed() < Duration::from_secs(5),
            "Idle daemon did not shut down; retry launch"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

pub fn ensure_running(paths: &Paths, executable: &Path) -> Result<()> {
    if let Ok(Response::State(state)) = rpc(paths, Request::Snapshot)
        && can_retire_daemon(&state)
        // A concurrent creation can make this fail; keep that daemon.
        && matches!(rpc(paths, Request::ShutdownIfIdle), Ok(Response::Ok))
    {
        wait_for_retirement(paths)?;
    }
    start_if_needed(paths, executable)
}

pub fn repair(paths: &Paths, executable: &Path, generation: &str) -> Result<Box<State>> {
    // Check the installed replacement before retiring even an idle daemon.
    for name in ["terminator-daemon", "terminator-hook"] {
        ensure!(
            executable_available(&executable.with_file_name(name)),
            "Reinstall the complete Terminator app before repairing: {name} is unavailable"
        );
    }
    let Response::State(state) = rpc(paths, Request::Snapshot)? else {
        anyhow::bail!("Could not read the running session service");
    };
    ensure!(
        state.generation == generation,
        "The session service changed. Review its current status before retrying"
    );
    ensure!(
        can_retire_daemon(&state),
        "Repair requires no live sessions and a compatible service with safe restart support. Existing sessions have been preserved"
    );
    // The daemon checks again under its creation lock; GUI snapshots are not authority.
    ensure!(
        matches!(rpc(paths, Request::ShutdownIfIdle)?, Response::Ok),
        "Safe restart was not acknowledged"
    );
    wait_for_retirement(paths)?;
    start_if_needed(paths, executable)?;
    match rpc(paths, Request::Snapshot)? {
        Response::State(state) => {
            ensure!(
                state.generation != generation
                    && state.attachment_helper_available == Some(true)
                    && state.attachment_helper_executable.is_some()
                    && state
                        .capabilities
                        .iter()
                        .any(|c| c == STABLE_HELPER_CAPABILITY),
                "The replacement service does not report a working private helper. Reopen the latest installed Terminator app"
            );
            Ok(state)
        }
        _ => anyhow::bail!("Could not verify the repaired session service"),
    }
}

fn start_if_needed(paths: &Paths, executable: &Path) -> Result<()> {
    if rpc(paths, Request::Snapshot).is_ok() {
        return Ok(());
    }
    // A failed RPC is never evidence that a live daemon may be replaced.
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.runtime.join("daemon.lock"))?;
    lock.try_lock_exclusive().context(
        "Running daemon is unresponsive or incompatible; its sessions have been preserved",
    )?;
    FileExt::unlock(&lock)?;
    let daemon = executable.with_file_name("terminator-daemon");
    ensure!(
        executable_available(&daemon),
        "Build or reinstall the complete Terminator app"
    );
    let log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.data.join("daemon.log"))?;
    let mut command = Command::new(daemon);
    command
        .env("TERMINATOR_DATA_DIR", &paths.data)
        .env("TERMINATOR_RUNTIME_DIR", &paths.runtime)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    let start = Instant::now();
    while rpc(paths, Request::Snapshot).is_err() {
        ensure!(
            start.elapsed() < Duration::from_secs(5),
            "Daemon did not start; inspect daemon.log and reopen Terminator to retry"
        );
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};

    fn fixture() -> (tempfile::TempDir, Paths, std::path::PathBuf) {
        let dir = tempfile::Builder::new()
            .prefix("tr-")
            .tempdir_in("/tmp")
            .unwrap();
        let paths = Paths::at(dir.path().to_path_buf());
        paths.init().unwrap();
        atomic_write(&paths.auth(), b"fixture").unwrap();
        for name in ["terminator-daemon", "terminator-hook"] {
            let path = dir.path().join(name);
            fs::write(&path, b"must never be executed").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let exe = dir.path().join("terminator");
        (dir, paths, exe)
    }

    #[test]
    fn repair_rechecks_identity_and_capability_before_sending_shutdown() {
        for changed_generation in [false, true] {
            let (_dir, paths, exe) = fixture();
            let listener = UnixListener::bind(paths.socket()).unwrap();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let request: Envelope = read_frame(&mut stream).unwrap();
                assert!(matches!(request.request, Request::Snapshot));
                let state = State {
                    daemon_version: Some("0.0.1".into()),
                    generation: if changed_generation {
                        "different"
                    } else {
                        "expected"
                    }
                    .into(),
                    capabilities: if changed_generation {
                        vec![SHUTDOWN_IF_IDLE_CAPABILITY.into()]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                };
                write_frame(&mut stream, &Response::State(Box::new(state))).unwrap();
                listener
            });
            assert!(repair(&paths, &exe, "expected").is_err());
            let listener = server.join().unwrap();
            listener.set_nonblocking(true).unwrap();
            assert!(
                listener.accept().is_err(),
                "Repair sent a request after an incompatible snapshot"
            );
        }
    }

    #[test]
    fn refused_idle_shutdown_does_not_fall_back_to_replacing_the_daemon() {
        let (_dir, paths, exe) = fixture();
        let listener = UnixListener::bind(paths.socket()).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request: Envelope = read_frame(&mut stream).unwrap();
            assert!(matches!(request.request, Request::Snapshot));
            write_frame(
                &mut stream,
                &Response::State(Box::new(State {
                    daemon_version: Some("0.0.1".into()),
                    generation: "expected".into(),
                    capabilities: vec![SHUTDOWN_IF_IDLE_CAPABILITY.into()],
                    ..Default::default()
                })),
            )
            .unwrap();
            let (mut stream, _) = listener.accept().unwrap();
            let request: Envelope = read_frame(&mut stream).unwrap();
            assert!(matches!(request.request, Request::ShutdownIfIdle));
            write_frame(
                &mut stream,
                &Response::Error("A session started concurrently".into()),
            )
            .unwrap();
            listener
        });
        assert!(
            repair(&paths, &exe, "expected")
                .unwrap_err()
                .to_string()
                .contains("concurrently")
        );
        let listener = server.join().unwrap();
        listener.set_nonblocking(true).unwrap();
        assert!(listener.accept().is_err());
    }

    #[test]
    fn unresponsive_locked_daemon_is_preserved() {
        let (_dir, paths, exe) = fixture();
        let lock = fs::File::create(paths.runtime.join("daemon.lock")).unwrap();
        lock.lock_exclusive().unwrap();
        assert!(
            ensure_running(&paths, &exe)
                .unwrap_err()
                .to_string()
                .contains("preserved")
        );
    }
}
