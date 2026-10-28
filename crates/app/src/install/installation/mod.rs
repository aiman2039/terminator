//! Validate the launch location before creating data or starting persistent PTYs.
#[cfg(test)]
mod tests;

use anyhow::{Context, Result, ensure};
#[cfg(test)]
use std::thread;
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use terminator_core::{
    Paths, bundled_exes, crash::CrashNotice, exe_name, executable_available, sibling_exe,
    spawn_session_leader, transport,
};

pub fn is_helper_error(error: &str) -> bool {
    error.starts_with("Attachment helper unavailable:")
}

/// Target this GUI's installation and service even from an unrelated terminal.
pub fn manual_shutdown_command(executable: &Path, paths: &Paths, stop_all: bool) -> Result<String> {
    use terminator_core::quote;
    let helper = sibling_exe(executable, "terminator-hook");
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
    let hook = sibling_exe(executable, "terminator-hook");
    let gui = std::path::absolute(executable)?;
    let daemon = sibling_exe(executable, "terminator-daemon");
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
    pub completion: transport::Stream,
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
    let (completion, lifetime) = transport::pair()?;
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
        .stdin(Stdio::from(lifetime.into_owned_fd()))
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
    if let Some(error) = report.get("error").and_then(serde_json::Value::as_str) {
        anyhow::bail!(
            "Session service restart failed: {error}. The app reopened for recovery; the old service may still be running."
        );
    }
    ensure!(
        report
            .get("generation")
            .and_then(serde_json::Value::as_str)
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
                    &active
                        .owner
                        .data
                        .join("bin")
                        .join(exe_name("terminator-daemon")),
                    &sibling_exe(executable, "terminator-daemon")
                )?,
            "Active generation does not match this installation"
        );
        return Ok(());
    }
    let expected = sibling_exe(executable, "terminator-daemon").canonicalize()?;
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
    Ok(sibling_exe(&std::env::current_exe()?, "terminator-hook"))
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
    for name in bundled_exes() {
        let helper = sibling_exe(&executable, &name);
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

pub fn show_crash_dialog(notice: &CrashNotice) {
    let _ = show_message("Terminator crashed", &notice.dialog_body(), false);
}

#[cfg(target_os = "macos")]
fn show_message(title: &str, description: &str, open_applications: bool) -> bool {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSAlert, NSAlertFirstButtonReturn, NSApplication, NSApplicationActivationPolicy,
    };
    use objc2_foundation::NSString;
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    #[expect(
        deprecated,
        reason = "activateIgnoringOtherApps is the only way to steal focus on older macOS"
    )]
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
