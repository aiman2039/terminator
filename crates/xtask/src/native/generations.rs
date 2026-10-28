//! Native generation diagnostics, restart warning, cancellation and reattachment.
use super::CaptureInput;
use super::{
    Context, Duration, Harness, Options, Path, Result, Value, capture, ensure, fs, id, json,
    output, session, sessions, thread, wait_child,
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
    first_migration(&h, o)?;
    let project = h.project("generation-workspace")?;
    let a = h.shell(&project)?;
    let file = h.root.join("generation-workspace/buffer.md");
    fs::write(&file, "# Saved\n")?;
    let editor = h.editor(&project, &file)?;
    change_editor_buffer(&h, &editor)?;
    h.layout(&project, &[a.clone(), editor.clone()])?;
    let original_generation = h
        .state()?
        .get("generation")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing generation"))?;
    h.env
        .insert("TERMINATOR_TEST_NEW_GENERATION".into(), "1".into());
    let b = updated_service_ready(&h, o, (&project, &editor, &original_generation))?;
    h.env.remove("TERMINATOR_TEST_NEW_GENERATION");
    h.assert_pids(&[a.clone(), b.clone(), editor.clone()])?;
    restart_warning_and_cancel(&h, o)?;
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
    refuse_late_session(&h, &project, &[a.clone(), b.clone(), editor.clone()])?;
    let before_recovery = h.state()?;
    drop(attached);
    confirmed_cleanup(&h, &file)?;
    after_confirmed_recovery(&h, o, &before_recovery)?;
    fs::write(
        o.output.join("generations.json"),
        serde_json::to_vec_pretty(&json!({
            "idle_migration":true,"generations":before_recovery.get("generations").unwrap_or(&Value::Null),
            "original_session":a,"new_session":b,"unsaved_editor":editor,"gui_relaunch_pids_preserved":true,
            "warning_counts_all_owners":true,"cancellation_preserves_sessions":true,
            "late_session_aborts_confirmed_cleanup":true,"confirmed_cleanup_stops_all_owners":true,"recovery_does_not_rerun_sessions":true
        }))?,
    )?;
    Ok(())
}

// The first GUI launch performs the real idle migration and candidate check.
fn first_migration(h: &Harness, o: &Options) -> Result<()> {
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "first-migration",
            actions: json!([]),
            after: 1800,
        },
        |_| {
            let deadline = Instant::now()
                .checked_add(Duration::from_secs(10))
                .ok_or_else(|| anyhow::anyhow!("generation deadline overflow"))?;
            while h.state().is_err() {
                ensure!(
                    Instant::now() < deadline,
                    "Initial generation did not start"
                );
                thread::sleep(Duration::from_millis(50));
            }
            Ok(())
        },
    )?;
    Ok(())
}

/// Wait for the editor's Neovim server, then change its buffer without saving.
fn change_editor_buffer(h: &Harness, editor: &Value) -> Result<()> {
    let owner = terminator_core::generations::owner_for(
        &Paths::at(h.root.clone()),
        &terminator_core::Request::History {
            session: id(editor).into(),
        },
    )?;
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(5))
        .ok_or_else(|| anyhow::anyhow!("editor deadline overflow"))?;
    let server = terminator_core::transport::nvim_listen_arg(&owner.editor_socket(id(editor)));
    #[cfg(unix)]
    while !owner.editor_socket(id(editor)).exists() {
        ensure!(Instant::now() < deadline, "Editor did not start");
        thread::sleep(Duration::from_millis(50));
    }
    // Windows Neovim listens on a named pipe with no filesystem record, so
    // poll the endpoint instead of waiting for a socket file.
    #[cfg(not(unix))]
    {
        let nvim = terminator_core::find_executable("nvim").context("Neovim required")?;
        loop {
            let mut probe = Command::new(&nvim);
            probe
                .arg("--server")
                .arg(&server)
                .args(["--remote-expr", "1"]);
            if output(probe).is_ok() {
                break;
            }
            ensure!(Instant::now() < deadline, "Editor did not start");
            thread::sleep(Duration::from_millis(50));
        }
    }
    let mut change =
        Command::new(terminator_core::find_executable("nvim").context("Neovim required")?);
    change
        .arg("--server")
        .arg(&server)
        .args(["--remote-expr", "setline(1, '# Unsaved across upgrades')"]);
    output(change)?;
    Ok(())
}

/// Wait for the new generation, create a shell in it, and check the unsaved
/// editor survived. Returns the new shell.
fn updated_service_ready(
    h: &Harness,
    o: &Options,
    (project, editor, original_generation): (&Value, &Value, &Value),
) -> Result<Value> {
    let mut b = Value::Null;
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "updated-service-ready",
            actions: json!([
                {"at_ms":700,"target":"settings"},
                {"at_ms":1100,"target":"settings-section:Updates"}
            ]),
            after: 2800,
        },
        |_| {
            h.wait(|s| s["generation"] != *original_generation, 10)?;
            b = h.shell(project)?;
            h.wait(|s| sessions(s).len() == 3, 5)?;
            h.wait(
                |_| {
                    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)
                        .is_ok_and(|s| {
                            s.get("markdown")
                                .and_then(|markdown| markdown.get(id(editor)))
                                .and_then(|editor| editor.get("text"))
                                .and_then(Value::as_str)
                                .is_some_and(|text| text.contains("Unsaved across upgrades"))
                        })
                },
                8,
            )?;
            Ok(())
        },
    )?;
    Ok(b)
}

fn restart_warning_and_cancel(h: &Harness, o: &Options) -> Result<()> {
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "all-owners-restart-warning",
            actions: json!([
                {"at_ms":700,"target":"settings"},
                {"at_ms":1100,"target":"settings-section:Updates"},
                {"at_ms":1600,"target":"restart-session-service"}
            ]),
            after: 2400,
        },
        |_| {
            h.wait(
                |_| {
                    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)
                        .is_ok_and(|s| {
                            s.get("installation").is_some_and(|installation| {
                                installation
                                    .get("restart_confirm")
                                    .is_some_and(|confirm| confirm == &json!(true))
                                    && installation
                                        .get("live_count")
                                        .is_some_and(|count| count == &json!(3))
                            })
                        })
                },
                10,
            )?;
            Ok(())
        },
    )?;
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "restart-cancelled",
            actions: json!([
                {"at_ms":700,"target":"settings"},
                {"at_ms":1100,"target":"settings-section:Updates"},
                {"at_ms":1600,"target":"restart-session-service"},
                {"at_ms":2100,"target":"cancel-restart-session"}
            ]),
            after: 2900,
        },
        |_| Ok(()),
    )?;
    Ok(())
}

/// A session created after the inventory was approved must abort confirmed cleanup.
fn refuse_late_session(h: &Harness, project: &Value, live: &[Value]) -> Result<()> {
    let approved: terminator_core::State = serde_json::from_value(h.state()?)?;
    let inventory = terminator_core::recovery::RestartInventory::capture(&approved);
    let late = h.shell(project)?;
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
    let mut expected = live.to_vec();
    expected.push(late.clone());
    h.assert_pids(&expected)?;
    h.rpc(json!({"Stop":{"session":id(&late)}}))?;
    h.wait(|state| session(state, id(&late))["lifecycle"] == "ended", 5)?;
    h.rpc(json!({"Remove":{"session":id(&late)}}))?;
    Ok(())
}

fn confirmed_cleanup(h: &Harness, file: &Path) -> Result<()> {
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
        fs::read_to_string(file)? == "# Saved\n",
        "Recovery silently saved an unsaved buffer"
    );
    Ok(())
}

fn after_confirmed_recovery(h: &Harness, o: &Options, before_recovery: &Value) -> Result<()> {
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "after-confirmed-recovery",
            actions: json!([]),
            after: 2000,
        },
        |_| {
            h.wait(
                |state| {
                    state.get("generation").unwrap_or(&Value::Null)
                        != before_recovery.get("generation").unwrap_or(&Value::Null)
                        && sessions(state).iter().all(|s| s["lifecycle"] == "ended")
                },
                10,
            )?;
            Ok(())
        },
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
    capture(
        CaptureInput {
            h: &h,
            opts: o,
            name: "legacy-crash-recovered",
            actions: json!([]),
            after: 2200,
        },
        |_| {
            let deadline = Instant::now()
                .checked_add(Duration::from_secs(10))
                .ok_or_else(|| anyhow::anyhow!("generation deadline overflow"))?;
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
        },
    )?;
    Ok(())
}
