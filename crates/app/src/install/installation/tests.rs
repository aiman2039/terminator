use super::*;

#[test]
#[cfg(unix)]
fn detached_helper_receives_the_exact_confirmed_inventory() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let hook = dir.path().join("helper");
    std::fs::write(
        &hook,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$TERMINATOR_DATA_DIR/arguments\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
    let inventory = terminator_core::recovery::RestartInventory {
        generation: "confirmed-generation".into(),
        sessions: ["session-a".into(), "session-b".into()].into(),
    };
    let (tx, rx) = std::sync::mpsc::channel();
    spawn_restart(
        RestartInvocation {
            hook,
            gui: dir.path().join("gui"),
            data: dir.path().into(),
            runtime: dir.path().join("run"),
            config: dir.path().join("config.toml"),
        },
        inventory.clone(),
        move |_| {
            let _ = tx.send(());
        },
    )
    .unwrap();
    rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    let arguments = std::fs::read_to_string(dir.path().join("arguments")).unwrap();
    let arguments: Vec<_> = arguments.lines().collect();
    let index = arguments
        .iter()
        .position(|arg| *arg == "--confirmed-inventory")
        .unwrap();
    let received: terminator_core::recovery::RestartInventory =
        serde_json::from_str(arguments[index + 1]).unwrap();
    assert_eq!(received, inventory);
}
#[test]
#[cfg(unix)]
fn helper_continues_and_logs_after_gui_process_exits() {
    const CHILD: &str = "TERMINATOR_RESTART_TEST_PARENT_EXIT";
    if let Some(directory) = std::env::var_os(CHILD) {
        let data = PathBuf::from(directory);
        spawn_restart(
            RestartInvocation {
                hook: data.join("helper"),
                gui: data.join("gui"),
                runtime: data.join("run"),
                config: data.join("config.toml"),
                data,
            },
            terminator_core::recovery::RestartInventory::capture(
                &terminator_core::State::default(),
            ),
            |_| {},
        )
        .unwrap();
        std::process::exit(0);
    }
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let hook = dir.path().join("helper");
    std::fs::write(&hook, "#!/bin/sh\nset -e\nsleep 0.2\nprintf 'continuing after GUI exit\\n' >&2\nprintf ready > \"$TERMINATOR_DATA_DIR/continued\"\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "install::installation::tests::helper_continues_and_logs_after_gui_process_exits",
        ])
        .env(CHILD, dir.path())
        .status()
        .unwrap();
    assert!(status.success());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !dir.path().join("continued").exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "Helper aborted after GUI exit"
        );
        thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        std::fs::read_to_string(dir.path().join("restart.log"))
            .unwrap()
            .contains("continuing after GUI exit")
    );
}

#[test]
fn restart_failure_survives_reopening_and_is_consumed_once() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path().to_owned());
    std::fs::write(
        paths.data.join("restart-result.json"),
        br#"{"generation":"old","error":"Session 7 did not stop"}"#,
    )
    .unwrap();
    let error = restart_result(
        &paths,
        &terminator_core::State::default(),
        Path::new("unused"),
    )
    .unwrap_err();
    assert!(error.to_string().contains("Session 7 did not stop"));
    assert!(
        restart_result(
            &paths,
            &terminator_core::State::default(),
            Path::new("unused")
        )
        .is_ok()
    );
}

#[test]
fn restart_reconnecting_to_old_generation_is_not_success() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path().to_owned());
    std::fs::write(
        paths.data.join("restart-result.json"),
        br#"{"generation":"old","error":null}"#,
    )
    .unwrap();
    let state = terminator_core::State {
        generation: "old".into(),
        ..Default::default()
    };
    assert!(
        restart_result(&paths, &state, Path::new("unused"))
            .unwrap_err()
            .to_string()
            .contains("previous service")
    );
}

#[test]
fn replacement_must_match_bundled_path_version_and_helper() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("terminator");
    let daemon = dir.path().join(exe_name("terminator-daemon"));
    std::fs::write(&daemon, "fixture").unwrap();
    let mut state = terminator_core::State {
        daemon_executable: Some(daemon),
        daemon_version: Some(env!("CARGO_PKG_VERSION").into()),
        attachment_helper_available: Some(true),
        capabilities: vec![terminator_core::STABLE_HELPER_CAPABILITY.into()],
        ..Default::default()
    };
    assert!(verify_replacement(&state, &exe).is_ok());
    state.daemon_version = Some("0.0.1".into());
    assert!(verify_replacement(&state, &exe).is_err());
    state.daemon_version = Some(env!("CARGO_PKG_VERSION").into());
    state.attachment_helper_available = Some(false);
    assert!(verify_replacement(&state, &exe).is_err());
}

#[test]
#[cfg(unix)]
fn detached_helper_failure_reports_completion_and_preserves_config() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let hook = dir.path().join("helper");
    std::fs::write(
        &hook,
        "#!/bin/sh\nprintf '%s' \"$TERMINATOR_CONFIG_DIR\" >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config_dir = dir.path().join("original config");
    let (tx, rx) = std::sync::mpsc::channel();
    spawn_restart(
        RestartInvocation {
            hook,
            gui: dir.path().join("gui"),
            data: dir.path().to_owned(),
            runtime: dir.path().join("run"),
            config: config_dir.join("config.toml"),
        },
        terminator_core::recovery::RestartInventory::capture(&terminator_core::State::default()),
        move |message| {
            let _ = tx.send(message);
        },
    )
    .unwrap();
    let message = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!(message, config_dir.to_string_lossy());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("restart.log")).unwrap(),
        config_dir.to_string_lossy()
    );
}

#[test]
#[cfg(unix)]
fn manual_shutdown_targets_this_installation_and_quotes_shell_metacharacters() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("Install ' $(false) `false`");
    std::fs::create_dir(&dir).unwrap();
    let helper = dir.join("terminator-hook");
    std::fs::write(
        &helper,
        "#!/bin/sh\nprintf '%s\\n' \"$TERMINATOR_DATA_DIR\" \"$TERMINATOR_RUNTIME_DIR\" \"$@\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
    let paths = terminator_core::Paths {
        data: dir.join("data ' $HOME"),
        runtime: dir.join("custom runtime"),
    };
    for (stop_all, arguments) in [
        (false, "rpc\n\"Shutdown\"\n"),
        (true, "ctl\nshutdown\n--stop-all\n--relaunch\n"),
    ] {
        let command = manual_shutdown_command(&dir.join("terminator"), &paths, stop_all).unwrap();
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &command])
            .env("TERMINATOR_DATA_DIR", "/wrong/data")
            .env("TERMINATOR_RUNTIME_DIR", "/wrong/runtime")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!(
                "{}\n{}\n{arguments}",
                paths.data.display(),
                paths.runtime.display()
            )
        );
    }
}

#[test]
fn restart_invocation_uses_gui_sibling_not_a_private_helper() {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let gui = dir.path().join("terminator");
    let hook = dir.path().join(exe_name("terminator-hook"));
    for path in [&gui, &hook, &dir.path().join(exe_name("terminator-daemon"))] {
        std::fs::write(path, b"#!/bin/sh\n").unwrap();
        // `executable_available` checks the exec bit on Unix only.
        #[cfg(unix)]
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let paths = terminator_core::Paths {
        data: dir.path().join("data"),
        runtime: dir.path().join("run"),
    };
    let invocation = restart_invocation(&gui, &paths).unwrap();
    assert_eq!(invocation.hook, hook);
    assert!(invocation.gui.ends_with("terminator"));
    assert_eq!(invocation.data, paths.data);
    assert_eq!(invocation.runtime, paths.runtime);
    std::fs::remove_file(&hook).unwrap();
    assert!(restart_invocation(&gui, &paths).is_err());
}

#[test]
fn gui_uses_only_an_advertised_available_private_helper() {
    use terminator_core::{STABLE_HELPER_CAPABILITY, State};
    let mut state = State {
        attachment_helper_executable: Some(crate::test_path("/private/pinned/terminator-hook")),
        attachment_helper_available: Some(true),
        ..State::default()
    };
    let bundled = sibling_exe(&std::env::current_exe().unwrap(), "terminator-hook");
    assert_eq!(attachment_helper(&state).unwrap(), bundled);
    state.capabilities.push(STABLE_HELPER_CAPABILITY.into());
    assert_eq!(
        attachment_helper(&state).unwrap(),
        state.attachment_helper_executable.clone().unwrap()
    );
    state.attachment_helper_available = Some(false);
    assert_eq!(attachment_helper(&state).unwrap(), bundled);
    state.attachment_helper_available = Some(true);
    state.attachment_helper_executable = Some("relative/path".into());
    assert_eq!(attachment_helper(&state).unwrap(), bundled);
}
#[test]
fn installer_downloaded_and_translocated_bundles_require_installation() {
    let home = Some(Path::new("/Users/test"));
    for path in [
        "/Volumes/Terminator/Terminator.app/Contents/MacOS/terminator",
        "/private/tmp/AppTranslocation/123/d/Terminator.app/Contents/MacOS/terminator",
        "/Users/test/Downloads/Terminator.app/Contents/MacOS/terminator",
        "/Applications-copy/Terminator.app/Contents/MacOS/terminator",
    ] {
        assert!(needs_installation(Path::new(path), home), "{path}");
    }
    for path in [
        "/Applications/Terminator.app/Contents/MacOS/terminator",
        "/Users/test/Applications/Terminator.app/Contents/MacOS/terminator",
        "/tmp/build/target/debug/terminator",
    ] {
        assert!(!needs_installation(Path::new(path), home), "{path}");
    }
    assert!(needs_installation(
        Path::new("/tmp/build/target/debug-local/Terminator.app/Contents/MacOS/terminator"),
        home
    ));
}
