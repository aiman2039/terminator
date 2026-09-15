//! Explicit cleanup through daemon-owned session termination, never process-name kills.
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    fs::{File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use terminator_core::*;

pub struct Options {
    pub stop_all: bool,
    pub timeout: Duration,
    pub relaunch: Option<PathBuf>,
}

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

fn refuse_managed_session(state: &State) -> Result<()> {
    ensure!(
        std::env::var_os("TERMINATOR_SESSION_ID").is_none(),
        "Run shutdown from Terminal.app or another terminal outside Terminator"
    );
    let ancestors = super::agent_parent().2;
    ensure!(
        !state
            .sessions
            .iter()
            .any(|s| s.lifecycle.live() && s.pid.is_some_and(|pid| ancestors.contains(&pid))),
        "Run shutdown from Terminal.app or another terminal outside Terminator"
    );
    Ok(())
}

fn validate_relaunch(exe: Option<&Path>) -> Result<()> {
    let Some(exe) = exe else {
        return Ok(());
    };
    ensure!(
        executable_available(exe),
        "Cannot reopen Terminator: {} is unavailable",
        exe.display()
    );
    Ok(())
}

fn prepare_relaunch_command(paths: &Paths, exe: &Path) -> Result<Command> {
    let config = terminator_core::appearance::config_path(paths)?;
    let mut command = Command::new(exe);
    command
        .env("TERMINATOR_DATA_DIR", &paths.data)
        .env("TERMINATOR_RUNTIME_DIR", &paths.runtime)
        .env(
            "TERMINATOR_CONFIG_DIR",
            config.parent().context("Missing config directory")?,
        )
        .env_remove("TERMINATOR_SESSION_ID")
        .env_remove("TERMINATOR_SESSION_TOKEN")
        .stdin(Stdio::null());
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.starts_with("TERMINATOR_TEST_ACTIONS") || name.starts_with("TERMINATOR_CAPTURE_") {
            command.env_remove(key);
        }
    }
    Ok(command)
}

fn spawn_relaunch(paths: &Paths, exe: &Path) -> Result<()> {
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths.data.join("restart.log"))?;
    let mut command = prepare_relaunch_command(paths, exe)?;
    command.stdout(log.try_clone()?).stderr(log);
    let mut child = spawn_session_leader(command)?;
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn stop_session(
    paths: &Paths,
    generation: &str,
    targets: &HashSet<String>,
    sid: &str,
) -> Result<()> {
    let before = current(paths, generation)?;
    check_scope(&before, targets)?;
    if before
        .sessions
        .iter()
        .any(|s| s.id == sid && s.lifecycle.live())
        && let Err(error) = rpc(
            paths,
            Request::Stop {
                session: sid.to_owned(),
            },
        )
    {
        let after = current(paths, generation)?;
        if after
            .sessions
            .iter()
            .any(|s| s.id == sid && s.lifecycle.live())
        {
            return Err(error.context(format!(
                "Could not stop session {sid}; the daemon was left running"
            )));
        }
    }
    Ok(())
}

fn wait_until_idle(
    paths: &Paths,
    generation: &str,
    targets: &HashSet<String>,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = current(paths, generation)?;
        check_scope(&remaining, targets)?;
        let live = remaining
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live())
            .count();
        if live == 0 {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "{live} sessions did not stop within the timeout. The daemon was left running; close the remaining jobs and retry"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_daemon_exit(paths: &Paths, timeout: Duration) -> Result<()> {
    let lock = OpenOptions::new()
        .write(true)
        .open(paths.runtime.join("daemon.lock"))?;
    let deadline = Instant::now() + timeout;
    loop {
        if !paths.socket().exists() && try_lock(&lock)? {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Shutdown was acknowledged but the daemon has not exited; do not relaunch until it finishes"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn finish_cleanup(paths: &Paths, state: State, stop_all: bool, timeout: Duration) -> Result<usize> {
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
        stop_session(paths, &state.generation, &targets, sid)?;
    }
    wait_until_idle(paths, &state.generation, &targets, timeout)?;
    let request = if state
        .capabilities
        .iter()
        .any(|c| c == SHUTDOWN_IF_IDLE_CAPABILITY)
    {
        Request::ShutdownIfIdle
    } else {
        Request::Shutdown
    };
    ensure!(
        matches!(rpc(paths, request)?, Response::Ok),
        "Daemon shutdown was not acknowledged"
    );
    wait_daemon_exit(paths, timeout)?;
    Ok(targets.len())
}

pub fn run(paths: &Paths, state: State, options: Options) -> Result<Value> {
    let Options {
        stop_all,
        timeout,
        relaunch,
    } = options;
    refuse_managed_session(&state)?;
    let live = state.sessions.iter().filter(|s| s.lifecycle.live()).count();
    ensure!(
        stop_all || live == 0,
        "{live} live sessions remain. Save your work, then use --stop-all to terminate them (unsaved editor buffers will be lost)"
    );
    validate_relaunch(relaunch.as_deref())?;
    let lock = close_gui(paths, timeout)?;
    let result = finish_cleanup(paths, state, stop_all, timeout);
    drop(lock);
    let relaunch_error = relaunch
        .as_ref()
        .and_then(|exe| spawn_relaunch(paths, exe).err());
    match (result, relaunch_error) {
        (Ok(stopped), None) => Ok(json!({
            "shutdown": true,
            "stopped_sessions": stopped,
            "relaunched": relaunch.is_some()
        })),
        (Ok(_), Some(error)) => {
            Err(error).context("Sessions stopped but Terminator could not reopen")
        }
        (Err(error), _) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    // Run socket fixtures in a child rather than mutating process-wide environment
    // while other tests are running. Keep the production managed-session guard.
    fn isolated(test: &str) -> bool {
        const CHILD: &str = "TERMINATOR_SHUTDOWN_TEST_CHILD";
        if std::env::var(CHILD).as_deref() == Ok(test) {
            return false;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture"])
            .env_remove("TERMINATOR_SESSION_ID")
            .env(CHILD, test)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        true
    }

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

    fn options(stop_all: bool, relaunch: Option<PathBuf>) -> Options {
        Options {
            stop_all,
            timeout: Duration::from_secs(1),
            relaunch,
        }
    }

    fn stub_exe(dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(
            &path,
            "#!/bin/sh\n{\n  printf 'data=%s\\n' \"$TERMINATOR_DATA_DIR\"\n  printf 'runtime=%s\\n' \"$TERMINATOR_RUNTIME_DIR\"\n  printf 'session=%s\\n' \"${TERMINATOR_SESSION_ID-}\"\n} > \"$TERMINATOR_DATA_DIR/relaunched.tmp\"\nmv \"$TERMINATOR_DATA_DIR/relaunched.tmp\" \"$TERMINATOR_DATA_DIR/relaunched\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn wait_marker(path: &Path) -> String {
        let start = Instant::now();
        while !path.exists() {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "relaunch stub did not run"
            );
            thread::sleep(Duration::from_millis(20));
        }
        std::fs::read_to_string(path).unwrap()
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
        if isolated(
            "shutdown::tests::failed_gui_checkpoint_sends_no_session_stop_or_daemon_shutdown",
        ) {
            return;
        }
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
        let error = run(&paths, state, options(true, None)).unwrap_err();
        assert!(format!("{error:#}").contains("Workspace save failed"));
        server.join().unwrap();
        daemon.set_nonblocking(true).unwrap();
        assert!(daemon.accept().is_err());
    }

    #[test]
    fn changed_daemon_or_concurrent_session_aborts_before_shutdown() {
        if isolated("shutdown::tests::changed_daemon_or_concurrent_session_aborts_before_shutdown")
        {
            return;
        }
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
            let error = run(&paths, initial, options(true, None)).unwrap_err();
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

    #[test]
    fn missing_relaunch_exe_does_not_close_gui() {
        if isolated("shutdown::tests::missing_relaunch_exe_does_not_close_gui") {
            return;
        }
        let (_dir, paths) = fixture();
        let lock = File::create(paths.runtime.join("ui.lock")).unwrap();
        lock.lock_exclusive().unwrap();
        let daemon = UnixListener::bind(paths.socket()).unwrap();
        let gui = UnixListener::bind(paths.runtime.join("gui.sock")).unwrap();
        let error = run(
            &paths,
            State::default(),
            options(true, Some(paths.data.join("missing-terminator"))),
        )
        .unwrap_err();
        assert!(error.to_string().contains("unavailable"));
        gui.set_nonblocking(true).unwrap();
        daemon.set_nonblocking(true).unwrap();
        assert!(gui.accept().is_err());
        assert!(daemon.accept().is_err());
        assert!(!paths.data.join("relaunched").exists());
    }

    #[test]
    fn failed_gui_close_does_not_relaunch() {
        if isolated("shutdown::tests::failed_gui_close_does_not_relaunch") {
            return;
        }
        let (dir, paths) = fixture();
        let stub = stub_exe(dir.path(), "relaunch");
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
        let error = run(
            &paths,
            State {
                sessions: vec![live_session()],
                ..Default::default()
            },
            options(true, Some(stub)),
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("Workspace save failed"));
        server.join().unwrap();
        daemon.set_nonblocking(true).unwrap();
        assert!(daemon.accept().is_err());
        thread::sleep(Duration::from_millis(200));
        assert!(!paths.data.join("relaunched").exists());
    }

    #[test]
    fn relaunch_pins_the_original_appearance_directory() {
        let (_dir, paths) = fixture();
        let expected = terminator_core::appearance::config_path(&paths).unwrap();
        let command = prepare_relaunch_command(&paths, Path::new("terminator")).unwrap();
        let configured = command
            .get_envs()
            .find(|(key, _)| *key == "TERMINATOR_CONFIG_DIR")
            .unwrap()
            .1
            .unwrap();
        assert_eq!(Path::new(configured), expected.parent().unwrap());
    }

    fn idle_state() -> State {
        State {
            generation: "fixture".into(),
            capabilities: vec![SHUTDOWN_IF_IDLE_CAPABILITY.into()],
            ..Default::default()
        }
    }

    #[test]
    fn relaunch_runs_after_lock_release_with_isolated_env() {
        if isolated("shutdown::tests::relaunch_runs_after_lock_release_with_isolated_env") {
            return;
        }
        let (dir, paths) = fixture();
        File::create(paths.runtime.join("daemon.lock")).unwrap();
        let stub = stub_exe(dir.path(), "relaunch");
        let listener = UnixListener::bind(paths.socket()).unwrap();
        let socket = paths.socket();
        let server = thread::spawn(move || {
            loop {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let request: Envelope = read_frame(&mut stream).unwrap();
                match request.request {
                    Request::Snapshot => {
                        write_frame(&mut stream, &Response::State(Box::new(idle_state()))).unwrap();
                    }
                    Request::ShutdownIfIdle => {
                        write_frame(&mut stream, &Response::Ok).unwrap();
                        let _ = std::fs::remove_file(&socket);
                        break;
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
        });
        let result = run(&paths, idle_state(), options(true, Some(stub))).unwrap();
        assert_eq!(result["shutdown"], true);
        assert_eq!(result["relaunched"], true);
        server.join().unwrap();
        let marker = wait_marker(&paths.data.join("relaunched"));
        assert!(marker.contains(&format!("data={}", paths.data.display())));
        assert!(marker.contains(&format!("runtime={}", paths.runtime.display())));
        assert!(marker.contains("session=\n"));
        let lock = File::create(paths.runtime.join("ui.lock")).unwrap();
        assert!(try_lock(&lock).unwrap());
    }

    #[test]
    fn stop_timeout_relaunches_without_replacing_the_daemon() {
        if isolated("shutdown::tests::stop_timeout_relaunches_without_replacing_the_daemon") {
            return;
        }
        let (dir, paths) = fixture();
        let stub = stub_exe(dir.path(), "relaunch");
        let listener = UnixListener::bind(paths.socket()).unwrap();
        let server = thread::spawn(move || {
            let mut live = idle_state();
            live.sessions.push(live_session());
            for _ in 0..64 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let request: Envelope = read_frame(&mut stream).unwrap();
                match request.request {
                    Request::Snapshot => {
                        write_frame(&mut stream, &Response::State(Box::new(live.clone()))).unwrap();
                    }
                    Request::Stop { .. } => {
                        write_frame(&mut stream, &Response::Ok).unwrap();
                    }
                    Request::ShutdownIfIdle | Request::Shutdown => {
                        panic!("timeout must not shut down the daemon");
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
        });
        let error = run(
            &paths,
            {
                let mut state = idle_state();
                state.sessions.push(live_session());
                state
            },
            options(true, Some(stub)),
        )
        .unwrap_err();
        assert!(error.to_string().contains("did not stop"));
        let _ = wait_marker(&paths.data.join("relaunched"));
        assert!(paths.socket().exists());
        drop(server);
    }

    #[test]
    fn managed_session_refuses_relaunch() {
        const CHILD: &str = "TERMINATOR_SHUTDOWN_TEST_CHILD";
        let name = "shutdown::tests::managed_session_refuses_relaunch";
        if std::env::var(CHILD).as_deref() != Ok(name) {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", name, "--nocapture"])
                .env(CHILD, name)
                .env("TERMINATOR_SESSION_ID", "session")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let (dir, paths) = fixture();
        let stub = stub_exe(dir.path(), "relaunch");
        let error = run(&paths, State::default(), options(true, Some(stub))).unwrap_err();
        assert!(error.to_string().contains("outside Terminator"));
        assert!(!paths.data.join("relaunched").exists());
    }
}
