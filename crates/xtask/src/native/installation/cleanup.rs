use super::super::{
    Duration, Harness, Options, Process, Result, Value, ensure, fs, json, sessions, wait_child,
};
use super::{gui, seed_dirty_editor};
use terminator_core::{Paths, ui_control};

pub(super) fn cleanup_closes_gui(o: &Options, minimized: bool) -> Result<()> {
    fs::create_dir_all(&o.output)?;
    let mut h = Harness::new()?;
    h.setup()?;
    let file = seed_dirty_editor(&h, "cleanup-gui")?;
    let mut child = launch_gui(&h, o, minimized)?;
    if minimized {
        minimize(&h)?;
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
    check_checkpoint(&h.state()?)?;
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

fn launch_gui(h: &Harness, o: &Options, minimized: bool) -> Result<Process> {
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
        .stdout(log.try_clone()?)
        .stderr(log);
    let child = Process(command.spawn()?);
    h.wait(|_| gui(h).is_ok(), 8)?;
    Ok(child)
}

fn minimize(h: &Harness) -> Result<()> {
    ui_control::rpc(
        &Paths::at(h.root.clone()),
        ui_control::Request::Window {
            action: "minimize".into(),
        },
    )?;
    h.wait(
        |_| {
            gui(h).is_ok_and(|snapshot| {
                snapshot
                    .get("window")
                    .and_then(|window| window.get("minimized"))
                    .is_some_and(|minimized| minimized == &json!(true))
            })
        },
        8,
    )?;
    Ok(())
}

/// Both sessions ended without relaunching, and the workspace layout survived.
fn check_checkpoint(state: &Value) -> Result<()> {
    ensure!(
        sessions(state).len() == 2 && sessions(state).iter().all(|s| s["lifecycle"] == "ended"),
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
    Ok(())
}
