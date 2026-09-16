//! Starting/replacing a daemon always preserves live sessions and held locks.
use anyhow::{Result, ensure};
use fs2::FileExt;
use std::{
    fs,
    path::Path,
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
                (running <= bundled
                    && state.generations.iter().any(|g| {
                        g.owner.id == state.generation
                            && (g.error.is_some() || g.owner.status == generations::Status::Retired)
                    }))
                    || running < bundled
                    || (running == bundled
                        && (!state
                            .capabilities
                            .iter()
                            .any(|c| c == generations::CAPABILITY)
                            || state.attachment_helper_available != Some(true)
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
        && (!state.generations.is_empty() || !state.sessions.iter().any(|s| s.lifecycle.live()))
}

pub fn can_restart_service(state: &State) -> bool {
    (can_replace_daemon(state) || !state.generations.is_empty())
        && state.sessions.iter().any(|s| s.lifecycle.live())
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
    if generations::exists(paths) {
        match crate::daemon_upgrade::activate(paths, executable) {
            Ok(()) => {
                let _ = fs::remove_file(paths.data.join("service-upgrade-error.txt"));
                return Ok(());
            }
            Err(error) if rpc(paths, Request::Snapshot).is_ok() => {
                atomic_write(&paths.data.join("service-upgrade-error.txt"), format!("Service upgrade pending: {error:#}. Existing sessions were preserved; retry in Settings → Updates.").as_bytes())?;
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }
    if let Ok(Response::State(state)) = rpc(paths, Request::Snapshot)
        && can_retire_daemon(&state)
        // A concurrent creation can make this fail; keep that daemon.
        && matches!(rpc(paths, Request::ShutdownIfIdle), Ok(Response::Ok))
    {
        wait_for_retirement(paths)?;
    }
    if rpc(paths, Request::Snapshot).is_ok() {
        return Ok(());
    }
    generations::migrate_after_legacy_exit(paths)?;
    crate::daemon_upgrade::activate(paths, executable)
}

pub fn repair(paths: &Paths, executable: &Path, generation: &str) -> Result<Box<State>> {
    if generations::exists(paths) {
        crate::daemon_upgrade::activate(paths, executable)?;
        let _ = fs::remove_file(paths.data.join("service-upgrade-error.txt"));
        if let Response::State(state) = rpc(paths, Request::Snapshot)? {
            return Ok(state);
        }
        anyhow::bail!("Could not verify the active service");
    }
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
    ensure_running(paths, executable)?;
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
            if state.generations.is_empty() {
                crate::installation::verify_replacement(&state, executable)?;
            }
            Ok(state)
        }
        _ => anyhow::bail!("Could not verify the repaired session service"),
    }
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
