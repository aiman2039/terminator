use super::*;

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
        .tempdir()
        .unwrap();
    let paths = Paths::at(dir.path().into());
    paths.init().unwrap();
    atomic_write(&paths.auth(), b"fixture").unwrap();
    (dir, paths)
}

fn options(stop_all: bool, relaunch: Option<PathBuf>) -> Options {
    Options {
        confirmed: None,
        stop_all,
        timeout: Duration::from_secs(1),
        relaunch,
    }
}

#[test]
fn held_lock_is_busy_and_can_be_acquired_after_release() {
    let (_dir, paths) = fixture();
    let path = paths.runtime.join("ui.lock");
    let owner = File::create(&path).unwrap();
    owner.lock_exclusive().unwrap();
    let contender = OpenOptions::new().write(true).open(&path).unwrap();
    assert!(!try_lock(&contender).unwrap());
    FileExt::unlock(&owner).unwrap();
    assert!(try_lock(&contender).unwrap());
}

#[test]
fn platform_lock_contention_is_busy() {
    assert!(!lock_result(Err(fs2::lock_contended_error())).unwrap());
}

#[test]
fn lock_permission_errors_are_reported_instead_of_treated_as_busy() {
    let error = lock_result(Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "fixture lock permission denied",
    )))
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::PermissionDenied
    );
}

#[cfg(unix)]
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

#[cfg(unix)]
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
fn gui_confirmation_rejects_late_session_before_closing_or_stopping_anything() {
    let (_dir, paths) = fixture();
    let daemon = transport::Listener::bind_ipc(&paths.socket()).unwrap();
    let gui = transport::Listener::bind_ipc(&paths.runtime.join("gui.sock")).unwrap();
    let mut state = State {
        generation: "confirmed-owner".into(),
        sessions: vec![live_session()],
        ..Default::default()
    };
    let confirmed = recovery::RestartInventory::capture(&state);
    let mut late = live_session();
    late.id = "created-after-confirmation".into();
    state.sessions.push(late);
    let mut options = options(true, None);
    options.confirmed = Some(confirmed);
    let error = run(&paths, state, options).unwrap_err();
    assert!(error.to_string().contains("after confirmation"));
    daemon.set_nonblocking(true).unwrap();
    gui.set_nonblocking(true).unwrap();
    assert!(daemon.accept().is_err(), "No Stop may be sent");
    assert!(gui.accept().is_err(), "The GUI must stay open");
}

#[test]
fn failed_gui_checkpoint_sends_no_session_stop_or_daemon_shutdown() {
    if isolated("shutdown::tests::failed_gui_checkpoint_sends_no_session_stop_or_daemon_shutdown") {
        return;
    }
    let (_dir, paths) = fixture();
    let lock = File::create(paths.runtime.join("ui.lock")).unwrap();
    lock.lock_exclusive().unwrap();
    let daemon = transport::Listener::bind_ipc(&paths.socket()).unwrap();
    let gui = transport::Listener::bind_ipc(&paths.runtime.join("gui.sock")).unwrap();
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
    assert!(
        format!("{error:#}").contains("Workspace save failed"),
        "Expected the GUI checkpoint error, got: {error:#}"
    );
    server.join().unwrap();
    daemon.set_nonblocking(true).unwrap();
    assert!(daemon.accept().is_err());
}

#[test]
fn changed_daemon_or_concurrent_session_aborts_before_shutdown() {
    if isolated("shutdown::tests::changed_daemon_or_concurrent_session_aborts_before_shutdown") {
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
        let listener = transport::Listener::bind_ipc(&paths.socket()).unwrap();
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
    let daemon = transport::Listener::bind_ipc(&paths.socket()).unwrap();
    let gui = transport::Listener::bind_ipc(&paths.runtime.join("gui.sock")).unwrap();
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

#[cfg(unix)]
#[cfg(unix)]
#[cfg(unix)]
#[test]
#[cfg(unix)]
fn failed_gui_close_does_not_relaunch() {
    if isolated("shutdown::tests::failed_gui_close_does_not_relaunch") {
        return;
    }
    let (dir, paths) = fixture();
    let stub = stub_exe(dir.path(), "relaunch");
    let lock = File::create(paths.runtime.join("ui.lock")).unwrap();
    lock.lock_exclusive().unwrap();
    let daemon = transport::Listener::bind_ipc(&paths.socket()).unwrap();
    let gui = transport::Listener::bind_ipc(&paths.runtime.join("gui.sock")).unwrap();
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

#[cfg(unix)]
#[cfg(unix)]
#[cfg(unix)]
#[test]
#[cfg(unix)]
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

#[cfg(unix)]
fn idle_state() -> State {
    State {
        generation: "fixture".into(),
        capabilities: vec![SHUTDOWN_IF_IDLE_CAPABILITY.into()],
        ..Default::default()
    }
}

#[cfg(unix)]
#[cfg(unix)]
#[cfg(unix)]
#[test]
#[cfg(unix)]
fn relaunch_runs_after_lock_release_with_isolated_env() {
    if isolated("shutdown::tests::relaunch_runs_after_lock_release_with_isolated_env") {
        return;
    }
    let (dir, paths) = fixture();
    File::create(paths.runtime.join("daemon.lock")).unwrap();
    let stub = stub_exe(dir.path(), "relaunch");
    let listener = transport::Listener::bind_ipc(&paths.socket()).unwrap();
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

#[cfg(unix)]
#[cfg(unix)]
#[cfg(unix)]
#[test]
#[cfg(unix)]
fn stop_timeout_relaunches_without_replacing_the_daemon() {
    if isolated("shutdown::tests::stop_timeout_relaunches_without_replacing_the_daemon") {
        return;
    }
    let (dir, paths) = fixture();
    let stub = stub_exe(dir.path(), "relaunch");
    let listener = transport::Listener::bind_ipc(&paths.socket()).unwrap();
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
    let report: Value =
        serde_json::from_slice(&std::fs::read(paths.data.join("restart-result.json")).unwrap())
            .unwrap();
    assert!(report["error"].as_str().unwrap().contains("did not stop"));
    let _ = wait_marker(&paths.data.join("relaunched"));
    assert!(paths.socket().exists());
    drop(server);
}

#[cfg(unix)]
#[cfg(unix)]
#[cfg(unix)]
#[test]
#[cfg(unix)]
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
