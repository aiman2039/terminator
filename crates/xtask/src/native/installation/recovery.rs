use super::super::CaptureInput;
use super::super::{
    Duration, Harness, Options, Result, Value, capture, ensure, fs, json, save_prefs, sessions,
    thread,
};
use super::{gui, remove_attachment_helper};
use terminator_core::{Paths, ui_control};

pub(super) fn manual_recovery(o: &Options) -> Result<()> {
    let mut h = Harness::new()?;
    h.setup()?;
    let state = h.state()?;
    remove_attachment_helper(&state)?;
    // Reuse the old-service proxy, which omits advertised capabilities.
    let _proxy = super::super::reviews::proxy(&h.root)?;
    h.env.insert(
        "TERMINATOR_RUNTIME_DIR".into(),
        h.root.join("legacy").to_string_lossy().into_owned(),
    );
    let mut paths = Paths::at(h.root.clone());
    paths.runtime = h.root.join("legacy");
    capture(
        CaptureInput {
            h: &h,
            opts: o,
            name: "manual-recovery",
            actions: json!([{"at_ms":700,"target":"fix-installation"}]),
            after: 2200,
        },
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

pub(super) fn connection_recovery(o: &Options) -> Result<()> {
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
    capture(
        CaptureInput {
            h: &h,
            opts: o,
            name: "reconnected",
            actions: json!([]),
            after: 2200,
        },
        |_| {
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
        },
    )?;
    Ok(())
}
