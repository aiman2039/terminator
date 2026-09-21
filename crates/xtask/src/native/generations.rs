//! Native generation diagnostics, restart warning, cancellation and reattachment.
use super::{
    Context, Duration, Harness, Options, Result, Value, capture, ensure, fs, id, json, output,
    session, sessions, thread, wait_child,
};
use std::{process::Command, time::Instant};
use terminator_core::{Paths, ui_control};

pub fn run(o: &Options) -> Result<()> {
    legacy_crash_recovery(o)?;
    let mut h = Harness::new()?;
    h.setup()?;
    h.rpc(json!("ShutdownIfIdle"))?;
    wait_child(&mut h.daemon.as_mut().unwrap().0, Duration::from_secs(5))?;
    h.daemon.take();
    // The first GUI launch performs the real idle migration and candidate check.
    capture(&h, o, "first-migration", json!([]), 1800, |_| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while h.state().is_err() {
            ensure!(
                Instant::now() < deadline,
                "Initial generation did not start"
            );
            thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    })?;
    let project = h.project("generation-workspace")?;
    let a = h.shell(&project)?;
    let file = h.root.join("generation-workspace/buffer.md");
    fs::write(&file, "# Saved\n")?;
    let editor = h.editor(&project, &file)?;
    let owner = terminator_core::generations::owner_for(
        &Paths::at(h.root.clone()),
        &terminator_core::Request::History {
            session: id(&editor).into(),
        },
    )?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !owner.editor_socket(id(&editor)).exists() {
        ensure!(Instant::now() < deadline, "Editor did not start");
        thread::sleep(Duration::from_millis(50));
    }
    let mut change =
        Command::new(terminator_core::find_executable("nvim").context("Neovim required")?);
    change
        .arg("--server")
        .arg(owner.editor_socket(id(&editor)))
        .args(["--remote-expr", "setline(1, '# Unsaved across upgrades')"]);
    output(change)?;
    h.layout(&project, &[a.clone(), editor.clone()])?;
    let original_generation = h.state()?["generation"].clone();
    h.env
        .insert("TERMINATOR_TEST_NEW_GENERATION".into(), "1".into());
    let mut b = Value::Null;
    capture(
        &h,
        o,
        "updated-service-ready",
        json!([
            {"at_ms":700,"target":"settings"},
            {"at_ms":1100,"target":"settings-section:Updates"}
        ]),
        2800,
        |_| {
            h.wait(|s| s["generation"] != original_generation, 10)?;
            b = h.shell(&project)?;
            h.wait(|s| sessions(s).len() == 3, 5)?;
            h.wait(
                |_| {
                    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)
                        .is_ok_and(|s| {
                            s["markdown"][id(&editor)]["text"]
                                .as_str()
                                .is_some_and(|text| text.contains("Unsaved across upgrades"))
                        })
                },
                8,
            )?;
            Ok(())
        },
    )?;
    h.env.remove("TERMINATOR_TEST_NEW_GENERATION");
    h.assert_pids(&[a.clone(), b.clone(), editor.clone()])?;
    capture(
        &h,
        o,
        "all-owners-restart-warning",
        json!([
            {"at_ms":700,"target":"settings"},
            {"at_ms":1100,"target":"settings-section:Updates"},
            {"at_ms":1600,"target":"restart-session-service"}
        ]),
        2400,
        |_| {
            h.wait(
                |_| {
                    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)
                        .is_ok_and(|s| {
                            s["installation"]["restart_confirm"] == true
                                && s["installation"]["live_count"] == 3
                        })
                },
                10,
            )?;
            Ok(())
        },
    )?;
    capture(
        &h,
        o,
        "restart-cancelled",
        json!([
            {"at_ms":700,"target":"settings"},
            {"at_ms":1100,"target":"settings-section:Updates"},
            {"at_ms":1600,"target":"restart-session-service"},
            {"at_ms":2100,"target":"cancel-restart-session"}
        ]),
        2900,
        |_| Ok(()),
    )?;
    h.assert_pids(&[a.clone(), b.clone(), editor.clone()])?;
    ensure!(
        fs::read_to_string(&file)? == "# Saved\n",
        "Unsaved editor text reached disk unexpectedly"
    );
    let mut attached = h.attach(&a)?;
    h.write(&mut attached, "printf 'OLD_OWNER_AFTER_GUI_RELAUNCH\\n'\n")?;
    h.wait(
        |_| {
            h.history(id(&a))
                .is_ok_and(|s| s.contains("OLD_OWNER_AFTER_GUI_RELAUNCH"))
        },
        5,
    )?;
    let approved: terminator_core::State = serde_json::from_value(h.state()?)?;
    let inventory = terminator_core::recovery::RestartInventory::capture(&approved);
    let late = h.shell(&project)?;
    let refused = h
        .command("terminator-hook")
        .args(["ctl", "shutdown", "--stop-all", "--confirmed-inventory"])
        .arg(serde_json::to_string(&inventory)?)
        .output()?;
    ensure!(
        !refused.status.success()
            && String::from_utf8_lossy(&refused.stderr).contains("after confirmation"),
        "Changed confirmation inventory was accepted"
    );
    h.assert_pids(&[a.clone(), b.clone(), editor.clone(), late.clone()])?;
    h.rpc(json!({"Stop":{"session":id(&late)}}))?;
    h.wait(|state| session(state, id(&late))["lifecycle"] == "ended", 5)?;
    h.rpc(json!({"Remove":{"session":id(&late)}}))?;
    let before_recovery = h.state()?;
    drop(attached);
    let cleanup = h
        .command("terminator-hook")
        .args(["ctl", "shutdown", "--stop-all", "--timeout", "10"])
        .output()?;
    ensure!(
        cleanup.status.success(),
        "All-owner recovery failed: {}",
        String::from_utf8_lossy(&cleanup.stderr)
    );
    ensure!(
        fs::read_to_string(&file)? == "# Saved\n",
        "Recovery silently saved an unsaved buffer"
    );
    capture(&h, o, "after-confirmed-recovery", json!([]), 2000, |_| {
        h.wait(
            |state| {
                state["generation"] != before_recovery["generation"]
                    && sessions(state).iter().all(|s| s["lifecycle"] == "ended")
            },
            10,
        )?;
        Ok(())
    })?;
    fs::write(
        o.output.join("generations.json"),
        serde_json::to_vec_pretty(&json!({
            "idle_migration":true,"generations":before_recovery["generations"],
            "original_session":a,"new_session":b,"unsaved_editor":editor,"gui_relaunch_pids_preserved":true,
            "warning_counts_all_owners":true,"cancellation_preserves_sessions":true,
            "late_session_aborts_confirmed_cleanup":true,"confirmed_cleanup_stops_all_owners":true,"recovery_does_not_rerun_sessions":true
        }))?,
    )?;
    Ok(())
}

fn legacy_crash_recovery(o: &Options) -> Result<()> {
    let mut h = Harness::new()?;
    h.setup()?;
    let project = h.project("legacy-crash-recovery")?;
    let original = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&original))?;
    let daemon = &mut h.daemon.as_mut().unwrap().0;
    ensure!(
        terminator_core::signals::signal_process(
            daemon.id(),
            terminator_core::signals::ProcSignal::Term,
        )
        .is_ok(),
        "Cannot stop fixture daemon"
    );
    wait_child(daemon, Duration::from_secs(5))?;
    h.daemon.take();
    capture(&h, o, "legacy-crash-recovered", json!([]), 2200, |_| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(state) = h.state() {
                ensure!(
                    session(&state, id(&original))["lifecycle"] == "interrupted",
                    "Stale legacy record was not reconciled"
                );
                ensure!(
                    sessions(&state).iter().all(|s| s["lifecycle"] != "running"),
                    "Migration restarted a historical session"
                );
                break;
            }
            ensure!(Instant::now() < deadline, "Legacy crash blocked migration");
            thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    })?;
    Ok(())
}
