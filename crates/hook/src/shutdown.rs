//! Explicit cleanup through daemon-owned session termination, never process-name kills.
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    thread,
    time::{Duration, Instant},
};
use terminator_core::*;

fn current(paths: &Paths, generation: &str) -> Result<State> {
    let Response::State(state) = rpc(paths, Request::Snapshot)? else {
        anyhow::bail!("Could not read session inventory");
    };
    ensure!(
        state.generation == generation,
        "The daemon changed during cleanup; retry against the current service"
    );
    Ok(*state)
}

fn check_scope(state: &State, targets: &HashSet<String>) -> Result<()> {
    ensure!(
        state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live())
            .all(|s| targets.contains(&s.id)),
        "A new session started during cleanup. It was preserved; stop other clients creating sessions and retry"
    );
    Ok(())
}

fn try_lock(file: &File) -> Result<bool> {
    match file.try_lock_exclusive() {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn close_gui(paths: &Paths, timeout: Duration) -> Result<File> {
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(paths.runtime.join("ui.lock"))?;
    if !try_lock(&lock)? {
        ui_control::rpc(paths, ui_control::Request::Window { action: "close".into() })
            .context("Could not request a normal GUI exit. If Terminator is hidden or minimized, restore its window, quit it completely and retry; sessions have not been stopped")?;
        let deadline = Instant::now() + timeout;
        while !try_lock(&lock)? {
            ensure!(
                Instant::now() < deadline,
                "GUI did not finish saving and closing. Sessions have not been stopped; finish or cancel pending dialogs and retry"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }
    // Keep the lock until cleanup completes, so another GUI cannot create sessions.
    Ok(lock)
}

pub fn run(paths: &Paths, state: State, stop_all: bool, timeout: Duration) -> Result<Value> {
    ensure!(
        std::env::var_os("TERMINATOR_SESSION_ID").is_none(),
        "Run shutdown from Terminal.app or another terminal outside Terminator"
    );
    let live = state.sessions.iter().filter(|s| s.lifecycle.live()).count();
    ensure!(
        stop_all || live == 0,
        "{live} live sessions remain. Save your work, then use --stop-all to terminate them (unsaved editor buffers will be lost)"
    );
    let ancestors = super::agent_parent().2;
    ensure!(
        !state
            .sessions
            .iter()
            .any(|s| s.lifecycle.live() && s.pid.is_some_and(|pid| ancestors.contains(&pid))),
        "Run shutdown from Terminal.app or another terminal outside Terminator"
    );
    let _gui_lock = close_gui(paths, timeout)?;
    let state = current(paths, &state.generation)?;
    let targets: HashSet<_> = state
        .sessions
        .iter()
        .filter(|s| s.lifecycle.live())
        .map(|s| s.id.clone())
        .collect();
    ensure!(
        stop_all || targets.is_empty(),
        "A session started while the GUI was closing. It was preserved; save your work and retry with --stop-all"
    );
    if !targets.is_empty() {
        eprintln!(
            "Stopping {} sessions; unsaved editor buffers will be discarded.",
            targets.len()
        );
    }
    for sid in &targets {
        let before = current(paths, &state.generation)?;
        check_scope(&before, &targets)?;
        if before
            .sessions
            .iter()
            .any(|s| s.id == *sid && s.lifecycle.live())
            && let Err(error) = rpc(
                paths,
                Request::Stop {
                    session: sid.clone(),
                },
            )
        {
            let after = current(paths, &state.generation)?;
            // Exiting independently between Snapshot and Stop is normal.
            if after
                .sessions
                .iter()
                .any(|s| s.id == *sid && s.lifecycle.live())
            {
                return Err(error.context(format!(
                    "Could not stop session {sid}; the daemon was left running"
                )));
            }
        }
    }
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = current(paths, &state.generation)?;
        check_scope(&remaining, &targets)?;
        let live = remaining
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live())
            .count();
        if live == 0 {
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "{live} sessions did not stop within the timeout. The daemon was left running; close the remaining jobs and retry"
        );
        thread::sleep(Duration::from_millis(50));
    }
    let request = if state
        .capabilities
        .iter()
        .any(|c| c == SHUTDOWN_IF_IDLE_CAPABILITY)
    {
        Request::ShutdownIfIdle
    } else {
        // Existing legacy request; never send the newer variant without its capability.
        Request::Shutdown
    };
    ensure!(
        matches!(rpc(paths, request)?, Response::Ok),
        "Daemon shutdown was not acknowledged"
    );
    let lock = OpenOptions::new()
        .write(true)
        .open(paths.runtime.join("daemon.lock"))?;
    let deadline = Instant::now() + timeout;
    loop {
        if !paths.socket().exists() && try_lock(&lock)? {
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "Shutdown was acknowledged but the daemon has not exited; do not relaunch until it finishes"
        );
        thread::sleep(Duration::from_millis(50));
    }
    Ok(json!({"shutdown":true,"stopped_sessions":targets.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    fn fixture() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::Builder::new()
            .prefix("cleanup-")
            .tempdir_in("/tmp")
            .unwrap();
        let paths = Paths::at(dir.path().into());
        paths.init().unwrap();
        atomic_write(&paths.auth(), b"fixture").unwrap();
        (dir, paths)
    }

    fn live_session() -> Session {
        serde_json::from_value(json!({
            "id":"new-session","project_id":"fixture","label":"Fixture",
            "cwd":"/tmp","kind":"shell","lifecycle":"running","created":0,
            "rows":24,"cols":80,"generation":"fixture","truncated":false,"cwd_confirmed":false
        }))
        .unwrap()
    }

    #[test]
    fn failed_gui_checkpoint_sends_no_session_stop_or_daemon_shutdown() {
        let (_dir, paths) = fixture();
        let lock = File::create(paths.runtime.join("ui.lock")).unwrap();
        lock.lock_exclusive().unwrap();
        let daemon = UnixListener::bind(paths.socket()).unwrap();
        let gui = UnixListener::bind(paths.runtime.join("gui.sock")).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = gui.accept().unwrap();
            let request: ui_control::Envelope = read_frame(&mut stream).unwrap();
            assert!(
                matches!(request.request, ui_control::Request::Window { action } if action == "close")
            );
            write_frame(
                &mut stream,
                &ui_control::Response {
                    result: None,
                    error: Some("Workspace save failed".into()),
                },
            )
            .unwrap();
        });
        let state = State {
            sessions: vec![live_session()],
            ..Default::default()
        };
        let error = run(&paths, state, true, Duration::from_secs(1)).unwrap_err();
        assert!(format!("{error:#}").contains("Workspace save failed"));
        server.join().unwrap();
        daemon.set_nonblocking(true).unwrap();
        assert!(daemon.accept().is_err());
    }

    #[test]
    fn changed_daemon_or_concurrent_session_aborts_before_shutdown() {
        for changed_generation in [false, true] {
            let (_dir, paths) = fixture();
            let initial = State::default();
            let mut changed = initial.clone();
            let mut replies = Vec::new();
            if changed_generation {
                changed.generation = "replacement".into();
            } else {
                replies.push(initial.clone());
                changed.sessions.push(live_session());
            }
            replies.push(changed);
            let listener = UnixListener::bind(paths.socket()).unwrap();
            let server = thread::spawn(move || {
                for state in replies {
                    let (mut stream, _) = listener.accept().unwrap();
                    let request: Envelope = read_frame(&mut stream).unwrap();
                    assert!(matches!(request.request, Request::Snapshot));
                    write_frame(&mut stream, &Response::State(Box::new(state))).unwrap();
                }
                listener
            });
            let error = run(&paths, initial, true, Duration::from_secs(1)).unwrap_err();
            assert!(error.to_string().contains(if changed_generation {
                "daemon changed"
            } else {
                "new session"
            }));
            let listener = server.join().unwrap();
            listener.set_nonblocking(true).unwrap();
            assert!(listener.accept().is_err());
        }
    }
}
