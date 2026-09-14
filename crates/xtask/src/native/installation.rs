//! Exercise recovery in the native GUI, using only a disposable daemon.
use super::*;
use terminator_core::{Paths, ui_control};

fn gui(h: &Harness) -> Result<Value> {
    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)
}

pub fn run(o: &Options) -> Result<()> {
    cleanup_closes_gui(o, false)?;
    cleanup_closes_gui(o, true)?;
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
    let generation = state["generation"].clone();
    let helper = PathBuf::from(state["attachment_helper_executable"].as_str().unwrap());
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
                |_| gui(&h).is_ok_and(|s| s["installation"]["settings_visible"] == true),
                5,
            )?;
            let ui = gui(&h)?;
            ensure!(
                ui["installation"]["problem"] == true && ui["installation"]["can_repair"] == false,
                "Recovery must remain disabled with a live session"
            );
            h.assert_pids(std::slice::from_ref(&shell))?;
            ensure!(
                h.state()?["generation"] == generation,
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
            {"at_ms":700,"target":"fix-installation"},
            {"at_ms":2200,"target":"repair-installation"}
        ]),
        5500,
        |_| {
            h.wait(
                |_| gui(&h).is_ok_and(|s| s["installation"]["settings_visible"] == true),
                5,
            )?;
            // The user finishes the last session while the recovery screen is open.
            h.rpc(json!({"Stop":{"session":id(&shell)}}))?;
            h.wait(
                |_| gui(&h).is_ok_and(|s| s["installation"]["can_repair"] == true),
                5,
            )?;
            // Daemon RPC is briefly unavailable during the acknowledged idle restart.
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            loop {
                if gui(&h).is_ok_and(|s| {
                    s["installation"]["generation"] != generation
                        && s["installation"]["repair_pending"] == false
                        && s["installation"]["problem"] == false
                        && s["installation"]["error"].is_null()
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
        repaired["generation"] != generation && repaired["attachment_helper_available"] == true,
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
    h.rpc(json!("ShutdownIfIdle"))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while Paths::at(h.root.clone()).socket().exists() {
        ensure!(
            std::time::Instant::now() < deadline,
            "Repaired fixture service did not shut down"
        );
        thread::sleep(Duration::from_millis(50));
    }
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
                .is_ok_and(|r| r["Text"] == "1")
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
            |_| gui(&h).is_ok_and(|s| s["window"]["minimized"] == true),
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
        !state["projects"][0]["layout"].is_null(),
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
    let helper = PathBuf::from(state["attachment_helper_executable"].as_str().unwrap());
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
                    ui_control::rpc(&paths, ui_control::Request::Snapshot)
                        .is_ok_and(|s| s["installation"]["settings_visible"] == true)
                },
                5,
            )?;
            Ok(())
        },
    )?;
    let after = h.state()?;
    ensure!(
        after["generation"] == state["generation"] && sessions(&after).is_empty(),
        "Showing manual recovery must not restart the service or create sessions"
    );
    Ok(())
}

fn connection_recovery(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    save_prefs(
        &h,
        &json!({"version":1,"typography_migrated":true,"attention_migrated":true}),
    )?;
    let wait_connection = |connected: bool| -> Result<()> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if gui(&h).is_ok_and(|s| {
                let status = &s["installation"];
                status["connected"] == connected
                    && if connected {
                        status["error"].is_null()
                    } else {
                        status["error"]
                            .as_str()
                            .is_some_and(|e| e.starts_with("Reconnecting:"))
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
            before["generation"] == after["generation"] && before["revision"] == after["revision"],
            "Reconnection fixture must retain the exact daemon snapshot"
        );
        Ok(())
    })?;
    Ok(())
}
