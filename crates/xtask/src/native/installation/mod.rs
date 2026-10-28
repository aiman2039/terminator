//! Exercise recovery in the native GUI, using only a disposable daemon.
mod cleanup;
mod recovery;
mod restart;

use super::CaptureInput;
use super::{
    Duration, Harness, Options, PathBuf, Result, Value, bin, capture, ensure, fs, id, json,
    session, thread,
};
use terminator_core::{Paths, ui_control};

fn gui(h: &Harness) -> Result<Value> {
    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)
}

/// True when the GUI's installation status has `key` equal to `expected`.
fn installation_is(h: &Harness, key: &str, expected: &Value) -> bool {
    gui(h).is_ok_and(|s| {
        s.get("installation")
            .and_then(|installation| installation.get(key))
            .is_some_and(|value| value == expected)
    })
}

/// Delete this installation's private attachment helper to simulate a broken install.
fn remove_attachment_helper(state: &Value) -> Result<()> {
    let helper = PathBuf::from(
        state
            .get("attachment_helper_executable")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing attachment helper"))?,
    );
    fs::remove_file(&helper)?;
    Ok(())
}

/// The generation recorded in a daemon snapshot.
fn generation(state: &Value) -> Result<Value> {
    state
        .get("generation")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing generation"))
}

/// A shell and an editor holding an unsaved buffer over `<name>/unsaved.txt`.
fn seed_dirty_editor(h: &Harness, name: &str) -> Result<PathBuf> {
    let project = h.project(name)?;
    let shell = h.shell(&project)?;
    let file = h.root.join(format!("{name}/unsaved.txt"));
    fs::create_dir_all(file.parent().unwrap())?;
    fs::write(&file, "saved file\n")?;
    let editor = h.editor(&project, &file)?;
    h.dirty_editor(&editor, "gg0Cunsaved buffer\u{1b}", 8)?;
    h.layout(&project, &[shell, editor])?;
    Ok(file)
}

pub fn run(o: &Options) -> Result<()> {
    restart::restart_failure_is_visible(o)?;
    cleanup::cleanup_closes_gui(o, false)?;
    cleanup::cleanup_closes_gui(o, true)?;
    restart::restart_cancel(o)?;
    restart::restart_relaunch(o)?;
    recovery::connection_recovery(o)?;
    recovery::manual_recovery(o)?;
    let mut h = Harness::new()?;
    freeze_binaries(&mut h)?;
    h.setup()?;
    let project = h.project("installation-recovery")?;
    let shell = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&shell))?;
    let state = h.state()?;
    let generation = generation(&state)?;
    remove_attachment_helper(&state)?;
    live_sessions_preserved(&h, o, (&shell, &generation))?;
    repaired(&h, o, (&shell, &generation))?;
    let repaired = h.state()?;
    ensure!(
        repaired.get("generation") != Some(&generation)
            && repaired
                .get("attachment_helper_available")
                .is_some_and(|available| available == &json!(true)),
        "Repair did not install a working private helper"
    );
    ensure!(
        session(&repaired, id(&shell))["lifecycle"] == "ended",
        "Repair revived an ended session"
    );
    let new = h.shell(&project)?;
    h.assert_pids(std::slice::from_ref(&new))?;
    fs::write(
        o.output.join("installation.json"),
        serde_json::to_vec_pretty(&json!({
            "live_session_preserved":true,"idle_repair_succeeded":true,
            "history_not_restarted":true,"new_session_after_repair":true
        }))?,
    )?;
    shut_down_repaired(&h, &new)
}

// Freeze the GUI and replacement daemon so other Cargo builds cannot change
// test-support features between the two native captures.
fn freeze_binaries(h: &mut Harness) -> Result<()> {
    let binaries = h.root.join("fixture-install");
    fs::create_dir(&binaries)?;
    for name in ["terminator", "terminator-daemon", "terminator-hook"] {
        fs::copy(bin().join(name), binaries.join(name))?;
    }
    h.env.insert(
        "TERMINATOR_FIXTURE_GUI".into(),
        binaries.join("terminator").to_string_lossy().into(),
    );
    Ok(())
}

fn live_sessions_preserved(
    h: &Harness,
    o: &Options,
    (shell, generation): (&Value, &Value),
) -> Result<()> {
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "live-sessions-preserved",
            actions: json!([
                {"at_ms":700,"target":"fix-installation"},
                {"at_ms":1100,"target":"show-stop-all-command"}
            ]),
            after: 2200,
        },
        |_| {
            h.wait(|_| installation_is(h, "settings_visible", &json!(true)), 5)?;
            let ui = gui(h)?;
            ensure!(
                ui.get("installation").is_some_and(|installation| {
                    installation
                        .get("problem")
                        .is_some_and(|problem| problem == &json!(true))
                        && installation
                            .get("can_repair")
                            .is_some_and(|repair| repair == &json!(false))
                }),
                "Recovery must remain disabled with a live session"
            );
            h.assert_pids(std::slice::from_ref(shell))?;
            ensure!(
                h.state()?
                    .get("generation")
                    .is_some_and(|current| current == generation),
                "Recovery retired a live daemon"
            );
            Ok(())
        },
    )?;
    Ok(())
}

fn repaired(h: &Harness, o: &Options, (shell, generation): (&Value, &Value)) -> Result<()> {
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "repaired",
            actions: json!([
                {"at_ms":700,"target":"fix-installation"}
            ]),
            after: 5500,
        },
        |_| {
            h.wait(|_| installation_is(h, "settings_visible", &json!(true)), 5)?;
            // The user finishes the last session while the recovery screen is open.
            h.rpc(json!({"Stop":{"session":id(shell)}}))?;
            wait_repair(h, generation)
        },
    )?;
    Ok(())
}

// Idle migration starts automatically; the repair button can disappear immediately.
// Daemon RPC is briefly unavailable during the acknowledged idle restart.
fn wait_repair(h: &Harness, generation: &Value) -> Result<()> {
    let deadline = std::time::Instant::now()
        .checked_add(Duration::from_secs(8))
        .ok_or_else(|| anyhow::anyhow!("repair deadline overflow"))?;
    loop {
        if gui(h).is_ok_and(|s| {
            s.get("installation").is_some_and(|installation| {
                installation.get("generation") != Some(generation)
                    && installation
                        .get("repair_pending")
                        .is_some_and(|pending| pending == &json!(false))
                    && installation
                        .get("problem")
                        .is_some_and(|problem| problem == &json!(false))
                    && installation.get("error").is_none_or(Value::is_null)
            })
        }) {
            return Ok(());
        }
        ensure!(
            std::time::Instant::now() < deadline,
            "Native repair did not finish successfully"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn shut_down_repaired(h: &Harness, new: &Value) -> Result<()> {
    h.rpc(json!({"Stop":{"session":id(new)}}))?;
    h.wait(|s| session(s, id(new))["lifecycle"] == "ended", 5)?;
    let endpoint = terminator_core::generations::owner_for(
        &Paths::at(h.root.clone()),
        &terminator_core::Request::Snapshot,
    )?;
    h.rpc(json!("ShutdownIfIdle"))?;
    let deadline = std::time::Instant::now()
        .checked_add(Duration::from_secs(5))
        .ok_or_else(|| anyhow::anyhow!("deadline overflow"))?;
    while endpoint.socket().exists() {
        ensure!(
            std::time::Instant::now() < deadline,
            "Repaired fixture service did not shut down"
        );
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
