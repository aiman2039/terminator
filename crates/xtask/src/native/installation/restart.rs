use super::super::CaptureInput;
use super::super::{
    Duration, Harness, Options, Process, Result, Value, capture, ensure, fs, json, sessions,
    thread, wait_child,
};
use super::{generation, gui, installation_is, remove_attachment_helper, seed_dirty_editor};

pub(super) fn restart_failure_is_visible(o: &Options) -> Result<()> {
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
    capture(
        CaptureInput {
            h: &h,
            opts: o,
            name: "restart-failure-visible",
            actions: json!([]),
            after: 2300,
        },
        |_| {
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
        },
    )?;
    h.assert_pids(&[shell])
}

pub(super) fn restart_cancel(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("restart-cancel")?;
    let shell = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&shell))?;
    let state = h.state()?;
    let generation = generation(&state)?;
    remove_attachment_helper(&state)?;
    capture(
        CaptureInput {
            h: &h,
            opts: o,
            name: "restart-cancel",
            actions: json!([
                {"at_ms":800,"target":"restart-session-service"},
                {"at_ms":1300,"target":"cancel-restart-session"}
            ]),
            after: 2200,
        },
        |_| {
            h.wait(|_| installation_is(&h, "can_restart", &json!(true)), 5)?;
            h.wait(|_| installation_is(&h, "restart_confirm", &json!(true)), 5)?;
            h.wait(|_| installation_is(&h, "restart_confirm", &json!(false)), 5)?;
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

pub(super) fn restart_relaunch(o: &Options) -> Result<()> {
    fs::create_dir_all(&o.output)?;
    let mut h = Harness::new()?;
    h.setup()?;
    let file = seed_dirty_editor(&h, "restart-relaunch")?;
    let state = h.state()?;
    let generation = generation(&state)?;
    remove_attachment_helper(&state)?;
    confirm_restart(&h, o)?;
    wait_relaunched(&h, &generation)?;
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

/// Confirm a service restart in the GUI and wait for it to close itself.
fn confirm_restart(h: &Harness, o: &Options) -> Result<()> {
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
    command.stdout(log.try_clone()?).stderr(log);
    let mut child = Process(command.spawn()?);
    h.wait(|_| installation_is(h, "can_restart", &json!(true)), 8)?;
    ensure!(
        wait_child(&mut child.0, Duration::from_secs(20))?.success(),
        "GUI did not close for restart"
    );
    Ok(())
}

/// Wait for the relaunched GUI to report a healthy new generation.
fn wait_relaunched(h: &Harness, generation: &Value) -> Result<()> {
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
            gui(h).map(|snapshot| { snapshot.get("installation").cloned().unwrap_or(Value::Null) })
        );
        if let Ok(snapshot) = gui(h)
            && snapshot
                .get("installation")
                .and_then(|installation| installation.get("generation"))
                .is_some_and(|current| current != generation)
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
    Ok(())
}
