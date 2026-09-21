//! Validate the launch location before creating data or starting persistent PTYs.
use anyhow::{Context, Result, ensure};
#[cfg(test)]
use std::thread;
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use terminator_core::{Paths, crash::CrashNotice, executable_available, spawn_session_leader};

pub fn is_helper_error(error: &str) -> bool {
    error.starts_with("Attachment helper unavailable:")
}

/// Target this GUI's installation and service even from an unrelated terminal.
pub fn manual_shutdown_command(executable: &Path, paths: &Paths, stop_all: bool) -> Result<String> {
    use terminator_core::quote;
    let helper = executable.with_file_name("terminator-hook");
    let path = |p: &Path| {
        let p = std::path::absolute(p)?;
        p.to_str()
            .map(quote)
            .context("Installation path cannot be represented as a shell command")
    };
    let arguments = if stop_all {
        "ctl shutdown --stop-all --relaunch"
    } else {
        "rpc '\"Shutdown\"'"
    };
    let config = terminator_core::appearance::config_path(paths)?;
    let config_dir = config.parent().context("Missing config directory")?;
    Ok(format!(
        "env TERMINATOR_CONFIG_DIR={} TERMINATOR_DATA_DIR={} \\\n  TERMINATOR_RUNTIME_DIR={} \\\n  {} {arguments}",
        path(config_dir)?,
        path(&paths.data)?,
        path(&paths.runtime)?,
        path(&helper)?,
    ))
}

#[derive(Debug, PartialEq, Eq)]
pub struct RestartInvocation {
    pub hook: PathBuf,
    pub gui: PathBuf,
    pub data: PathBuf,
    pub runtime: PathBuf,
    pub config: PathBuf,
}

pub fn restart_invocation(executable: &Path, paths: &Paths) -> Result<RestartInvocation> {
    let hook = executable.with_file_name("terminator-hook");
    let gui = std::path::absolute(executable)?;
    let daemon = executable.with_file_name("terminator-daemon");
    ensure!(
        executable_available(&daemon),
        "Cannot restart: {} is unavailable. Reinstall the complete app.",
        daemon.display()
    );
    ensure!(
        executable_available(&hook),
        "Cannot restart: {} is unavailable",
        hook.display()
    );
    ensure!(
        executable_available(&gui),
        "Cannot restart: {} is unavailable",
        gui.display()
    );
    Ok(RestartInvocation {
        hook,
        gui,
        data: paths.data.clone(),
        runtime: paths.runtime.clone(),
        config: terminator_core::appearance::config_path(paths)?,
    })
}

pub struct RestartWatch {
    pub completion: std::os::unix::net::UnixStream,
    pub log_path: PathBuf,
    pub log_start: u64,
}
pub fn begin_restart(
    invocation: RestartInvocation,
    inventory: terminator_core::recovery::RestartInventory,
) -> Result<RestartWatch> {
    let RestartInvocation {
        hook,
        gui,
        data,
        runtime,
        config,
    } = invocation;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(data.join("restart.log"))
        .context("Cannot open restart log")?;
    let log_start = log.metadata()?.len();
    let log_path = data.join("restart.log");
    // A silent inherited socket tracks helper lifetime without routing its output
    // through the GUI. Losing the GUI must not give the helper a broken pipe.
    let (completion, lifetime) = std::os::unix::net::UnixStream::pair()?;
    let mut command = Command::new(&hook);
    command
        .args(["ctl", "shutdown", "--stop-all", "--relaunch", "--exe"])
        .arg(&gui)
        .arg("--confirmed-inventory")
        .arg(serde_json::to_string(&inventory)?)
        .env("TERMINATOR_DATA_DIR", &data)
        .env("TERMINATOR_RUNTIME_DIR", &runtime)
        .env(
            "TERMINATOR_CONFIG_DIR",
            config.parent().context("Missing config directory")?,
        )
        .env_remove("TERMINATOR_SESSION_ID")
        .env_remove("TERMINATOR_SESSION_TOKEN")
        .stdin(Stdio::from(std::os::fd::OwnedFd::from(lifetime)))
        .stdout(Stdio::null())
        .stderr(log);
    let mut child = spawn_session_leader(command)?;
    // This is only the short-lived intermediate fork; the detached helper retains
    // the completion socket and must outlive GUI shutdown.
    let _ = child.wait();
    completion.set_nonblocking(true)?;
    Ok(RestartWatch {
        completion,
        log_path,
        log_start,
    })
}
pub fn restart_message(log_path: PathBuf, log_start: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    (|| -> std::io::Result<String> {
        let mut file = std::fs::File::open(log_path)?;
        let start = log_start.max(file.metadata()?.len().saturating_sub(16 * 1024));
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        file.take(16 * 1024).read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
    })()
    .unwrap_or_default()
}
#[cfg(test)]
pub fn spawn_restart(
    invocation: RestartInvocation,
    inventory: terminator_core::recovery::RestartInventory,
    finished: impl FnOnce(String) + Send + 'static,
) -> Result<()> {
    let RestartWatch {
        mut completion,
        log_path,
        log_start,
    } = begin_restart(invocation, inventory)?;
    completion.set_nonblocking(false)?;
    thread::spawn(move || {
        let _ = std::io::copy(&mut completion, &mut std::io::sink());
        finished(restart_message(log_path, log_start));
    });
    Ok(())
}

/// Consume the detached restart result only after a service has answered.
pub fn restart_result(
    paths: &Paths,
    state: &terminator_core::State,
    executable: &Path,
) -> Result<()> {
    let path = paths.data.join("restart-result.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    std::fs::remove_file(path)?;
    let report: serde_json::Value = serde_json::from_slice(&bytes)?;
    if let Some(error) = report["error"].as_str() {
        anyhow::bail!(
            "Session service restart failed: {error}. The app reopened for recovery; the old service may still be running."
        );
    }
    ensure!(
        report["generation"]
            .as_str()
            .is_some_and(|old| old != state.generation),
        "Session service restart failed: reconnected to the previous service"
    );
    verify_replacement(state, executable)
}

pub fn verify_replacement(state: &terminator_core::State, executable: &Path) -> Result<()> {
    if let Some(active) = state
        .generations
        .iter()
        .find(|g| g.owner.id == state.generation)
    {
        ensure!(
            active.error.is_none()
                && state.attachment_helper_available == Some(true)
                && crate::daemon_upgrade::same_file_contents(
                    &active.owner.data.join("bin/terminator-daemon"),
                    &executable.with_file_name("terminator-daemon")
                )?,
            "Active generation does not match this installation"
        );
        return Ok(());
    }
    let expected = executable
        .with_file_name("terminator-daemon")
        .canonicalize()?;
    ensure!(
        state
            .daemon_executable
            .as_ref()
            .and_then(|path| path.canonicalize().ok())
            .as_ref()
            == Some(&expected)
            && state.daemon_version.as_deref() == Some(env!("CARGO_PKG_VERSION"))
            && state
                .capabilities
                .iter()
                .any(|c| c == terminator_core::STABLE_HELPER_CAPABILITY)
            && state.attachment_helper_available == Some(true),
        "Replacement session service does not match this app or lacks a working helper. Reinstall the complete app; review installation details for the running service path and version."
    );
    Ok(())
}

pub fn attachment_helper(state: &terminator_core::State) -> anyhow::Result<std::path::PathBuf> {
    if state
        .capabilities
        .iter()
        .any(|c| c == terminator_core::STABLE_HELPER_CAPABILITY)
        && state.attachment_helper_available == Some(true)
        && let Some(path) = &state.attachment_helper_executable
        && path.is_absolute()
    {
        return Ok(path.clone());
    }
    // Old daemons don't advertise a private helper. A newly installed GUI's
    // bridge can still attach to their existing PTYs without replacing them.
    Ok(std::env::current_exe()?.with_file_name("terminator-hook"))
}

pub fn needs_installation(executable: &Path, home: Option<&Path>) -> bool {
    let Some(bundle) = executable
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
    else {
        return false; // Unbundled development builds.
    };
    executable.starts_with("/Volumes")
        || executable
            .components()
            .any(|p| p.as_os_str() == "AppTranslocation")
        || !(bundle.starts_with("/Applications")
            || home.is_some_and(|h| bundle.starts_with(h.join("Applications"))))
}

pub fn preflight() -> anyhow::Result<bool> {
    let executable = std::env::current_exe()?;
    // Isolated data-dir launches (debug-local.sh) are not a disk-image install.
    // Bundles under target/ still look uninstalled to Launch Services.
    let isolated = std::env::var_os("TERMINATOR_DATA_DIR").is_some();
    if cfg!(target_os = "macos")
        && !isolated
        && needs_installation(
            &executable,
            std::env::var_os("HOME").as_deref().map(Path::new),
        )
    {
        if show_message(
            "Install Terminator before opening",
            "Drag Terminator from the disk image into Applications, eject the disk image, then open Terminator from Applications.\n\nThis keeps terminal sessions attached to a permanent installation. No daemon has been started by this launch.",
            true,
        ) {
            open::that("/Applications")?;
        }
        return Ok(false);
    }
    for name in ["terminator-daemon", "terminator-hook"] {
        let helper = executable.with_file_name(name);
        if !terminator_core::executable_available(&helper) {
            show_message(
                "Terminator installation needs repair",
                &format!(
                    "A required executable is missing or cannot be executed:\n{}\n\nReinstall the complete Terminator app into Applications. For a development build, run cargo build --workspace --locked. Existing terminal sessions have been preserved.",
                    helper.display()
                ),
                false,
            );
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn show_crash_dialog(notice: &CrashNotice) {
    let _ = show_message("Terminator crashed", &notice.dialog_body(), false);
}

#[cfg(target_os = "macos")]
fn show_message(title: &str, description: &str, open_applications: bool) -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationPolicy,
    };
    use objc2_foundation::NSString;
    let mtm = MainThreadMarker::new().expect("Startup dialogs run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    // An app-owned alert remains visible and associated with Terminator even
    // before winit has created its main window.
    {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(title));
        alert.setInformativeText(&NSString::from_str(description));
        alert.addButtonWithTitle(&NSString::from_str(if open_applications {
            "Open Applications"
        } else {
            "Quit"
        }));
        if open_applications {
            alert.addButtonWithTitle(&NSString::from_str("Quit"));
        }
        alert.runModal() == NSAlertFirstButtonReturn && open_applications
    }
}

#[cfg(not(target_os = "macos"))]
fn show_message(title: &str, description: &str, _open_applications: bool) -> bool {
    rfd::MessageDialog::new()
        .set_title(title)
        .set_description(description)
        .set_level(rfd::MessageLevel::Error)
        .show();
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
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
                terminator_core::recovery::RestartInventory::capture(&Default::default()),
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
                "installation::tests::helper_continues_and_logs_after_gui_process_exits",
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
        let error = restart_result(&paths, &Default::default(), Path::new("unused")).unwrap_err();
        assert!(error.to_string().contains("Session 7 did not stop"));
        assert!(restart_result(&paths, &Default::default(), Path::new("unused")).is_ok());
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
        let daemon = dir.path().join("terminator-daemon");
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
            terminator_core::recovery::RestartInventory::capture(&Default::default()),
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
            let command =
                manual_shutdown_command(&dir.join("terminator"), &paths, stop_all).unwrap();
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
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let gui = dir.path().join("terminator");
        let hook = dir.path().join("terminator-hook");
        for path in [&gui, &hook, &dir.path().join("terminator-daemon")] {
            std::fs::write(path, b"#!/bin/sh\n").unwrap();
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
            attachment_helper_executable: Some("/private/pinned/terminator-hook".into()),
            attachment_helper_available: Some(true),
            ..State::default()
        };
        let bundled = std::env::current_exe()
            .unwrap()
            .with_file_name("terminator-hook");
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
}
