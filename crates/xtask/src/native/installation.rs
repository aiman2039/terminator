//! Exercise recovery in the native GUI, using only a disposable daemon.
use super::*;
use terminator_core::{Paths, ui_control};

fn gui(h: &Harness) -> Result<Value> {
    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)
}

pub fn run(o: &Options) -> Result<()> {
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
            {"at_ms":700,"target":"fix-installation"}
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
