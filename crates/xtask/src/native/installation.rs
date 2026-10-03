//! Exercise recovery in the native GUI, using only a disposable daemon.
use super::{
    Duration, Harness, Options, PathBuf, Process, Result, Value, bin, capture, ensure, fs, id,
    json, reviews, save_prefs, session, sessions, thread, wait_child,
};
use terminator_core::{Paths, ui_control};

fn gui(h: &Harness) -> Result<Value> {
    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)
}

fn restart_failure_is_visible(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("restart-failure")?;
    let shell = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&shell))?;
    fs::write(
        h.root.join("restart-result.json"),
        serde_json::to_vec(&json!({
            "generation":h.state()?.get("generation").unwrap_or(&Value::Null), "error":"Fixture session did not stop"
        }))?,
    )?;
    capture(&h, o, "restart-failure-visible", json!([]), 2300, |_| {
        h.wait(
            |_| {
                gui(&h).is_ok_and(|s| {
                    s.get("installation")
                        .and_then(|installation| installation.get("error"))
                        .and_then(Value::as_str)
                        .is_some_and(|error| error.contains("Fixture session did not stop"))
                })
            },
            8,
        )?;
        Ok(())
    })?;
    h.assert_pids(&[shell])
}

pub fn run(o: &Options) -> Result<()> {
    restart_failure_is_visible(o)?;
    cleanup_closes_gui(o, false)?;
    cleanup_closes_gui(o, true)?;
    restart_cancel(o)?;
    restart_relaunch(o)?;
    connection_recovery(o)?;
    manual_recovery(o)?;
    let mut h = Harness::new()?;
    // Freeze the GUI and replacement daemon so other Cargo builds cannot change
    // test-support features between the two native captures.
    let binaries = h.root.join("fixture-install");
    fs::create_dir(&binaries)?;
    for name in ["terminator", "terminator-daemon", "terminator-hook"] {
        fs::copy(bin().join(name), binaries.join(name))?;
    }
    h.env.insert(
        "TERMINATOR_FIXTURE_GUI".into(),
        binaries.join("terminator").to_string_lossy().into(),
    );
    h.setup()?;
    let project = h.project("installation-recovery")?;
    let shell = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&shell))?;
    let state = h.state()?;
    let generation = state
        .get("generation")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing generation"))?;
    let helper = PathBuf::from(
        state
            .get("attachment_helper_executable")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing attachment helper"))?,
    );
    fs::remove_file(helper)?;
    capture(
        &h,
        o,
        "live-sessions-preserved",
        json!([
            {"at_ms":700,"target":"fix-installation"},
            {"at_ms":1100,"target":"show-stop-all-command"}
        ]),
        2200,
        |_| {
            h.wait(
                |_| {
                    gui(&h).is_ok_and(|s| {
                        s.get("installation")
                            .and_then(|installation| installation.get("settings_visible"))
                            .is_some_and(|visible| visible == &json!(true))
                    })
                },
                5,
            )?;
            let ui = gui(&h)?;
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
            h.assert_pids(std::slice::from_ref(&shell))?;
            ensure!(
                h.state()?
                    .get("generation")
                    .is_some_and(|current| current == &generation),
                "Recovery retired a live daemon"
            );
            Ok(())
        },
    )?;
    capture(
        &h,
        o,
        "repaired",
        json!([
            {"at_ms":700,"target":"fix-installation"}
        ]),
        5500,
        |_| {
            h.wait(
                |_| {
                    gui(&h).is_ok_and(|s| {
                        s.get("installation")
                            .and_then(|installation| installation.get("settings_visible"))
                            .is_some_and(|visible| visible == &json!(true))
                    })
                },
                5,
            )?;
            // The user finishes the last session while the recovery screen is open.
            h.rpc(json!({"Stop":{"session":id(&shell)}}))?;
            // Idle migration starts automatically; the repair button can disappear immediately.
            // Daemon RPC is briefly unavailable during the acknowledged idle restart.
            let deadline = std::time::Instant::now()
                .checked_add(Duration::from_secs(8))
                .ok_or_else(|| anyhow::anyhow!("repair deadline overflow"))?;
            loop {
                if gui(&h).is_ok_and(|s| {
                    s.get("installation").is_some_and(|installation| {
                        installation.get("generation") != Some(&generation)
                            && installation
                                .get("repair_pending")
                                .is_some_and(|pending| pending == &json!(false))
                            && installation
                                .get("problem")
                                .is_some_and(|problem| problem == &json!(false))
                            && installation.get("error").is_none_or(Value::is_null)
                    })
                }) {
                    break;
                }
                ensure!(
                    std::time::Instant::now() < deadline,
                    "Native repair did not finish successfully"
                );
                thread::sleep(Duration::from_millis(50));
            }
            Ok(())
        },
    )?;
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
    h.rpc(json!({"Stop":{"session":id(&new)}}))?;
    h.wait(|s| session(s, id(&new))["lifecycle"] == "ended", 5)?;
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

fn restart_cancel(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("restart-cancel")?;
    let shell = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&shell))?;
    let state = h.state()?;
    let generation = state
        .get("generation")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing generation"))?;
    let helper = PathBuf::from(
        state
            .get("attachment_helper_executable")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing attachment helper"))?,
    );
    fs::remove_file(helper)?;
    capture(
        &h,
        o,
        "restart-cancel",
        json!([
            {"at_ms":800,"target":"restart-session-service"},
            {"at_ms":1300,"target":"cancel-restart-session"}
        ]),
        2200,
        |_| {
            h.wait(
                |_| {
                    gui(&h).is_ok_and(|s| {
                        s.get("installation")
                            .and_then(|installation| installation.get("can_restart"))
                            .is_some_and(|restart| restart == &json!(true))
                    })
                },
                5,
            )?;
            h.wait(
                |_| {
                    gui(&h).is_ok_and(|s| {
                        s.get("installation")
                            .and_then(|installation| installation.get("restart_confirm"))
                            .is_some_and(|confirm| confirm == &json!(true))
                    })
                },
                5,
            )?;
            h.wait(
                |_| {
                    gui(&h).is_ok_and(|s| {
                        s.get("installation")
                            .and_then(|installation| installation.get("restart_confirm"))
                            .is_some_and(|confirm| confirm == &json!(false))
                    })
                },
                5,
            )?;
            ensure!(
                gui(&h)?
                    .get("installation")
                    .and_then(|installation| installation.get("restart_pending"))
                    .is_some_and(|pending| pending == &json!(false))
                    && h.state()?
                        .get("generation")
                        .is_some_and(|current| current == &generation),
                "Cancel must not restart the service"
            );
            h.assert_pids(std::slice::from_ref(&shell))?;
            Ok(())
        },
    )?;
    Ok(())
}

fn restart_relaunch(o: &Options) -> Result<()> {
    fs::create_dir_all(&o.output)?;
    let mut h = Harness::new()?;
    h.setup()?;
    let project = h.project("restart-relaunch")?;
    let shell = h.shell(&project)?;
    let file = h.root.join("restart-relaunch/unsaved.txt");
    fs::create_dir_all(file.parent().unwrap())?;
    fs::write(&file, "saved file\n")?;
    let editor = h.editor(&project, &file)?;
    h.wait(
        |_| {
            h.rpc(json!({"EditorStatus":{"session":id(&editor)}}))
                .is_ok()
        },
        8,
    )?;
    h.write(&mut h.attach(&editor)?, "gg0Cunsaved buffer\u{1b}")?;
    h.wait(
        |_| {
            h.rpc(json!({"EditorStatus":{"session":id(&editor)}}))
                .is_ok_and(|status| status.get("Text").is_some_and(|text| text == "1"))
        },
        8,
    )?;
    h.layout(&project, &[shell, editor])?;
    let state = h.state()?;
    let generation = state
        .get("generation")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing generation"))?;
    let helper = PathBuf::from(
        state
            .get("attachment_helper_executable")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing attachment helper"))?,
    );
    fs::remove_file(&helper)?;
    let log = fs::File::create(o.output.join("restart-relaunch.log"))?;
    let mut command = h.command("terminator");
    command
        .env(
            "TERMINATOR_CAPTURE_PATH",
            o.output.join("restart-confirm.png"),
        )
        .env("TERMINATOR_CAPTURE_AFTER_MS", "60000")
        .env("TERMINATOR_TEST_KEEP_OPEN", "1")
        .env("TERMINATOR_TEST_SCALE", o.scale.to_string())
        .env(
            "TERMINATOR_TEST_ACTIONS",
            json!([
                {"at_ms":700,"target":"restart-session-service"},
                {"at_ms":1100,"target":"confirm-restart-session","hover":true,"capture":true},
                {"at_ms":1500,"target":"confirm-restart-session"}
            ])
            .to_string(),
        );
    if cfg!(target_os = "macos") {
        command
            .env("TERMINATOR_TEST_BACKGROUND", "1")
            .env("TERMINATOR_TEST_RENDER_OCCLUDED", "1");
    }
    command.stdout(log.try_clone()?).stderr(log);
    let mut child = Process(command.spawn()?);
    h.wait(
        |_| {
            gui(&h).is_ok_and(|s| {
                s.get("installation")
                    .and_then(|installation| installation.get("can_restart"))
                    .is_some_and(|restart| restart == &json!(true))
            })
        },
        8,
    )?;
    ensure!(
        wait_child(&mut child.0, Duration::from_secs(20))?.success(),
        "GUI did not close for restart"
    );
    let deadline = std::time::Instant::now()
        .checked_add(Duration::from_secs(40))
        .ok_or_else(|| anyhow::anyhow!("relaunch deadline overflow"))?;
    let mut next = None;
    while next.is_none() {
        ensure!(
            std::time::Instant::now() < deadline,
            "Relaunched GUI did not come back: {}; report: {}; GUI: {:?}",
            fs::read_to_string(h.root.join("restart.log")).unwrap_or_default(),
            fs::read_to_string(h.root.join("restart-result.json")).unwrap_or_default(),
            gui(&h)
                .map(|snapshot| { snapshot.get("installation").cloned().unwrap_or(Value::Null) })
        );
        if let Ok(snapshot) = gui(&h)
            && snapshot
                .get("installation")
                .and_then(|installation| installation.get("generation"))
                .is_some_and(|current| current != &generation)
            && snapshot
                .get("installation")
                .and_then(|installation| installation.get("problem"))
                .is_some_and(|problem| problem == &json!(false))
            && snapshot
                .get("installation")
                .and_then(|installation| installation.get("error"))
                .is_none_or(Value::is_null)
        {
            next = Some(snapshot);
        } else {
            thread::sleep(Duration::from_millis(50));
        }
    }
    let after = h.state()?;
    ensure!(
        after.get("generation").unwrap_or(&Value::Null) != &generation
            && after
                .get("attachment_helper_available")
                .is_some_and(|available| available == &json!(true)),
        "Restart did not start this installation's service"
    );
    ensure!(
        sessions(&after).iter().all(|s| s["lifecycle"] == "ended"),
        "Restart revived an ended session"
    );
    ensure!(
        fs::read_to_string(&file)? == "saved file\n",
        "Restart must not silently save the unsaved buffer"
    );
    let output = h
        .command("terminator-hook")
        .args(["ctl", "shutdown", "--timeout", "10"])
        .output()?;
    ensure!(
        output.status.success(),
        "Could not shut down relaunched service: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    wait_child(&mut h.daemon.as_mut().unwrap().0, Duration::from_secs(5))?;
    h.daemon.take();
    fs::write(
        o.output.join("restart-relaunch.json"),
        serde_json::to_vec_pretty(&json!({
            "restarted":true,"unsaved_buffer_not_written":true,
            "history_not_restarted":true,"helper_available":true
        }))?,
    )?;
    Ok(())
}

fn cleanup_closes_gui(o: &Options, minimized: bool) -> Result<()> {
    fs::create_dir_all(&o.output)?;
    let mut h = Harness::new()?;
    h.setup()?;
    let project = h.project("cleanup-gui")?;
    let shell = h.shell(&project)?;
    let file = h.root.join("cleanup-gui/unsaved.txt");
    fs::write(&file, "saved file\n")?;
    let editor = h.editor(&project, &file)?;
    h.wait(
        |_| {
            h.rpc(json!({"EditorStatus":{"session":id(&editor)}}))
                .is_ok()
        },
        8,
    )?;
    h.write(&mut h.attach(&editor)?, "gg0Cunsaved buffer\u{1b}")?;
    h.wait(
        |_| {
            h.rpc(json!({"EditorStatus":{"session":id(&editor)}}))
                .is_ok_and(|status| status.get("Text").is_some_and(|text| text == "1"))
        },
        8,
    )?;
    h.layout(&project, &[shell, editor])?;
    let log = fs::File::create(o.output.join(if minimized {
        "cleanup-minimized.log"
    } else {
        "cleanup-gui.log"
    }))?;
    let mut command = h.command("terminator");
    command
        .env(
            "TERMINATOR_CAPTURE_PATH",
            o.output.join("cleanup-gui-unused.png"),
        )
        .env("TERMINATOR_CAPTURE_AFTER_MS", "60000")
        .env("TERMINATOR_TEST_BACKGROUND", "1")
        .stdout(log.try_clone()?)
        .stderr(log);
    let mut child = Process(command.spawn()?);
    h.wait(|_| gui(&h).is_ok(), 8)?;
    if minimized {
        ui_control::rpc(
            &Paths::at(h.root.clone()),
            ui_control::Request::Window {
                action: "minimize".into(),
            },
        )?;
        h.wait(
            |_| {
                gui(&h).is_ok_and(|snapshot| {
                    snapshot
                        .get("window")
                        .and_then(|window| window.get("minimized"))
                        .is_some_and(|minimized| minimized == &json!(true))
                })
            },
            8,
        )?;
    }
    let output = h
        .command("terminator-hook")
        .args(["ctl", "shutdown", "--stop-all", "--timeout", "10"])
        .output()?;
    ensure!(
        output.status.success(),
        "GUI cleanup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(
        wait_child(&mut child.0, Duration::from_secs(3))?.success(),
        "GUI did not close normally"
    );
    wait_child(&mut h.daemon.as_mut().unwrap().0, Duration::from_secs(3))?;
    h.daemon.take();
    ensure!(
        fs::read_to_string(file)? == "saved file\n",
        "Stop-all must not silently save the unsaved buffer"
    );
    h.start()?;
    let state = h.state()?;
    ensure!(
        sessions(&state).len() == 2 && sessions(&state).iter().all(|s| s["lifecycle"] == "ended"),
        "GUI cleanup lost or relaunched session records"
    );
    ensure!(
        !state
            .get("projects")
            .and_then(|projects| projects.get(0))
            .and_then(|project| project.get("layout"))
            .is_none_or(Value::is_null),
        "GUI cleanup did not retain its workspace checkpoint"
    );
    fs::write(
        o.output.join(if minimized {
            "cleanup-minimized.json"
        } else {
            "cleanup-gui.json"
        }),
        serde_json::to_vec_pretty(&json!({
            "minimized":minimized,"normal_gui_exit":true,"shell_and_editor_ended":true,
            "unsaved_buffer_not_written":true,"history_not_restarted":true
        }))?,
    )?;
    Ok(())
}

fn manual_recovery(o: &Options) -> Result<()> {
    let mut h = Harness::new()?;
    h.setup()?;
    let state = h.state()?;
    let helper = PathBuf::from(
        state
            .get("attachment_helper_executable")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing attachment helper"))?,
    );
    fs::remove_file(helper)?;
    // Reuse the old-service proxy, which omits advertised capabilities.
    let _proxy = reviews::proxy(&h.root)?;
    h.env.insert(
        "TERMINATOR_RUNTIME_DIR".into(),
        h.root.join("legacy").to_string_lossy().into_owned(),
    );
    let mut paths = Paths::at(h.root.clone());
    paths.runtime = h.root.join("legacy");
    capture(
        &h,
        o,
        "manual-recovery",
        json!([{"at_ms":700,"target":"fix-installation"}]),
        2200,
        |_| {
            h.wait(
                |_| {
                    ui_control::rpc(&paths, ui_control::Request::Snapshot).is_ok_and(|s| {
                        s.get("installation")
                            .and_then(|installation| installation.get("settings_visible"))
                            .is_some_and(|visible| visible == &json!(true))
                    })
                },
                5,
            )?;
            Ok(())
        },
    )?;
    let after = h.state()?;
    ensure!(
        after.get("generation").unwrap_or(&Value::Null)
            == state.get("generation").unwrap_or(&Value::Null)
            && sessions(&after).is_empty(),
        "Showing manual recovery must not restart the service or create sessions"
    );
    Ok(())
}

fn connection_recovery(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    // Keep this transport-compatibility fixture on the legacy endpoint. An idle
    // legacy service now migrates on GUI launch, legitimately removing its socket.
    let project = h.project("connection-recovery")?;
    let shell = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&shell))?;
    save_prefs(
        &h,
        &json!({"version":1,"typography_migrated":true,"attention_migrated":true}),
    )?;
    let wait_connection = |connected: bool| -> Result<()> {
        let deadline = std::time::Instant::now()
            .checked_add(Duration::from_secs(5))
            .ok_or_else(|| anyhow::anyhow!("deadline overflow"))?;
        loop {
            if gui(&h).is_ok_and(|snapshot| {
                let Some(status) = snapshot.get("installation") else {
                    return false;
                };
                status
                    .get("connected")
                    .is_some_and(|value| value == &json!(connected))
                    && if connected {
                        status.get("error").is_none_or(Value::is_null)
                    } else {
                        status
                            .get("error")
                            .and_then(Value::as_str)
                            .is_some_and(|error| error.starts_with("Reconnecting:"))
                    }
            }) {
                return Ok(());
            }
            ensure!(
                std::time::Instant::now() < deadline,
                "GUI did not report connected={connected}"
            );
            thread::sleep(Duration::from_millis(30));
        }
    };
    capture(&h, o, "reconnected", json!([]), 2200, |_| {
        wait_connection(true)?;
        thread::sleep(Duration::from_millis(300));
        let before = h.state()?;
        let socket = Paths::at(h.root.clone()).socket();
        let hidden = socket.with_extension("temporarily-unavailable");
        fs::rename(&socket, &hidden)?;
        let disconnected = wait_connection(false);
        fs::rename(hidden, socket)?;
        disconnected?;
        wait_connection(true)?;
        let after = h.state()?;
        ensure!(
            before.get("generation").unwrap_or(&Value::Null)
                == after.get("generation").unwrap_or(&Value::Null)
                && before.get("revision").unwrap_or(&Value::Null)
                    == after.get("revision").unwrap_or(&Value::Null),
            "Reconnection fixture must retain the exact daemon snapshot"
        );
        Ok(())
    })?;
    Ok(())
}
