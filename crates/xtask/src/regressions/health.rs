use crate::harness::{Harness, bin, id, session, session_present};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::{Path, PathBuf},
};
use terminator_core::quote;

pub(super) fn missing_helper_health() -> Result<()> {
    let mut h = Harness::new()?;
    let install = isolated_install(&mut h)?;
    h.setup()?;
    let project = h.project("helper-health")?;
    let shell = h.shell(&project)?;
    let initial = h.state()?;
    let pinned = pinned_helper(&h, &initial, &install)?;
    let second = h.shell(&project)?;
    let _stream = pinned_helper_runs(&h, (&shell, &second), &pinned)?;
    ensure!(
        h.state()?.get("generation").unwrap_or(&Value::Null)
            == initial.get("generation").unwrap_or(&Value::Null),
        "App removal replaced the daemon"
    );
    broken_helper_detected(&h, &pinned)?;
    h.assert_pids(std::slice::from_ref(&shell))?;
    ensure!(
        h.rpc(json!("ShutdownIfIdle")).is_err(),
        "Broken helper allowed shutdown with a live session"
    );
    ensure!(
        h.shell(&project).is_err(),
        "Missing helper should give an installation error"
    );
    h.rpc(json!({"Stop":{"session":id(&shell)}}))?;
    h.rpc(json!({"Stop":{"session":id(&second)}}))?;
    h.wait(
        |s| !session_present(s, id(&shell)) && !session_present(s, id(&second)),
        5,
    )?;
    h.rpc(json!("ShutdownIfIdle"))?;
    println!(
        "{}",
        json!({"app_removal_preserves_helper":true,"new_session_after_removal":true,"pinned_hook_executes":true,"missing_helper_detected":true,"permission_change_invalidates_snapshot":true,"live_sessions_preserved":true})
    );
    Ok(())
}

/// Run the daemon from a copied installation the fixture can later delete.
fn isolated_install(h: &mut Harness) -> Result<PathBuf> {
    let install = h.root.join("isolated-install");
    fs::create_dir(&install)?;
    for name in ["terminator-daemon", "terminator-hook"] {
        fs::copy(bin().join(name), install.join(name))?;
    }
    h.env.insert(
        "TERMINATOR_FIXTURE_DAEMON".into(),
        install
            .join("terminator-daemon")
            .to_string_lossy()
            .into_owned(),
    );
    h.restart()?;
    h.restart()?;
    Ok(install)
}

/// The daemon pins a private helper copy that survives replacing and removing
/// the installation; returns its path.
fn pinned_helper(h: &Harness, initial: &Value, install: &Path) -> Result<PathBuf> {
    ensure!(
        initial
            .get("attachment_helper_available")
            .is_some_and(|available| available.as_bool() == Some(true)),
        "Installed helper was not available"
    );
    let pinned = std::path::PathBuf::from(
        initial
            .get("attachment_helper_executable")
            .ok_or_else(|| anyhow!("missing attachment helper"))?
            .as_str()
            .unwrap(),
    );
    ensure!(
        pinned.starts_with(&h.root) && !pinned.starts_with(install),
        "Helper was not pinned outside the installation"
    );
    let original_helper = fs::read(&pinned)?;
    // Model both in-place replacement and removal of an entire old app bundle.
    fs::write(
        install.join("terminator-hook"),
        b"replaced application helper",
    )?;
    #[cfg(unix)]
    fs::set_permissions(
        install.join("terminator-hook"),
        fs::Permissions::from_mode(0o600),
    )?;
    fs::remove_dir_all(install)?;
    ensure!(
        h.state()?
            .get("attachment_helper_available")
            .is_some_and(|available| available.as_bool() == Some(true))
            && fs::read(&pinned)? == original_helper,
        "App replacement or removal changed the pinned helper"
    );
    Ok(pinned)
}

/// The pinned helper still reports a cwd; returns the attached stream.
fn pinned_helper_runs(
    h: &Harness,
    (shell, second): (&Value, &Value),
    pinned: &Path,
) -> Result<terminator_core::transport::Stream> {
    let new_cwd = h.root.join("after-app-removal");
    fs::create_dir(&new_cwd)?;
    let mut stream = h.attach(shell)?;
    h.write(
        &mut stream,
        &format!(
            "cd {}; {} cwd \"$PWD\"; printf 'PINNED_HELPER_STILL_WORKS\\n'\n",
            quote(&new_cwd.to_string_lossy()),
            quote(&pinned.to_string_lossy())
        ),
    )?;
    h.wait(
        |st| session(st, id(shell))["cwd"] == new_cwd.to_string_lossy().as_ref(),
        5,
    )?;
    h.assert_pids(&[shell.clone(), second.clone()])?;
    Ok(stream)
}

// Health now refers to the actual private helper, not a discarded bundle.
fn broken_helper_detected(h: &Harness, pinned: &Path) -> Result<()> {
    let hint = serde_json::from_value::<terminator_core::State>(h.state()?)?.snapshot_hint();
    #[cfg(unix)]
    fs::set_permissions(pinned, fs::Permissions::from_mode(0o600))?;
    let paths = terminator_core::Paths::at(h.root.clone());
    let response = terminator_core::conditional_snapshot(&paths, Some(hint))?;
    ensure!(
        matches!(response, terminator_core::Response::State(ref s) if s.attachment_helper_available == Some(false)),
        "Permission change did not invalidate conditional snapshot"
    );
    fs::remove_file(pinned)?;
    ensure!(
        h.state()?
            .get("attachment_helper_available")
            .is_some_and(|available| available.as_bool() == Some(false)),
        "Removed helper was not detected"
    );
    Ok(())
}
