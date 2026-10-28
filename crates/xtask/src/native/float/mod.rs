//! Detach, float, in-split and stacked-tab flows through the public UI.
//!
//! Menu routing, layout assertions and per-action captures run anywhere
//! fixtures run. Observing the float window itself needs a real window
//! manager (macOS `screencapture`); closing it goes through the same
//! dock-back path as the OS close button via a marker file the fixture
//! writes once the window has been observed. Fixtures run foregrounded
//! (stealing focus briefly): background windows are denied Metal
//! drawables, so in-app screenshots never render without it.
mod close;
#[cfg(target_os = "macos")]
mod macos;
mod suite;

use super::Options;
use crate::harness::{Harness, Process, id, session_ids, sessions, wait_child};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::Child,
    thread,
    time::{Duration, Instant},
};
pub use suite::run;

const KNOWN_TABS: [&str; 8] = [
    "Terminal",
    "NativeEditor",
    "Diff",
    "Image",
    "Browser",
    "Player",
    "CommitLog",
    "Blame",
];

fn poll(mut check: impl FnMut() -> Result<bool>, secs: u64, what: &str) -> Result<()> {
    let end = Instant::now()
        .checked_add(Duration::from_secs(secs))
        .ok_or_else(|| anyhow::anyhow!("deadline overflow"))?;
    loop {
        if check()? {
            return Ok(());
        }
        ensure!(Instant::now() < end, "{what}");
        thread::sleep(Duration::from_millis(200));
    }
}

/// Leaves of a saved dock as (tabs, active index), walking the layout JSON
/// without depending on node enum shapes.
fn leaf_tabs<'a>(node: &'a Value, out: &mut Vec<(&'a Vec<Value>, usize)>) {
    match node {
        Value::Array(items) => items.iter().for_each(|item| leaf_tabs(item, out)),
        Value::Object(map) => {
            if let Some(Value::Array(tabs)) = map.get("tabs")
                && !tabs.is_empty()
                && tabs.iter().all(|tab| match tab {
                    Value::Object(fields) => {
                        fields.len() == 1
                            && fields.keys().all(|key| KNOWN_TABS.contains(&key.as_str()))
                    }
                    _ => false,
                })
            {
                let active = map
                    .get("active")
                    .and_then(Value::as_u64)
                    .and_then(|active| usize::try_from(active).ok())
                    .unwrap_or(usize::MAX);
                out.push((tabs, active));
            }
            map.values().for_each(|value| leaf_tabs(value, out));
        }
        _ => {}
    }
}

/// Spawns a fixture like [`capture`] (same env, actions and log) but
/// never requires an in-app screenshot: occluded windows are denied
/// Metal drawables, so screenshots stall while updates, menus and
/// viewport ordering keep working. Verification is OS-level
/// (`screencapture`, window-list polls) plus daemon state; the fixture
/// exits through `TERMINATOR_TEST_EXIT_MARKER`, with the capture
/// deadline fallback as a safety net.
struct DriveInput<'a> {
    h: &'a Harness,
    o: &'a Options,
    name: &'a str,
    actions: Value,
    after: u64,
    exit: &'a PathBuf,
}

fn drive<F: FnOnce(&mut Child) -> Result<()>>(input: DriveInput<'_>, observe: F) -> Result<String> {
    let DriveInput {
        h,
        o,
        name,
        actions,
        after,
        exit,
    } = input;
    let directory = std::path::absolute(&o.output)?;
    fs::create_dir_all(&directory)?;
    // A stale exit file would close the fresh fixture on its first frame.
    let _ = fs::remove_file(exit);
    let logpath = directory.join(format!("{name}.log"));
    let log = fs::File::create(&logpath)?;
    let mut command = h.command("terminator");
    command
        .env(
            "TERMINATOR_CAPTURE_PATH",
            directory.join(format!("{name}.png")),
        )
        .env("TERMINATOR_CAPTURE_AFTER_MS", after.to_string())
        .env("TERMINATOR_TEST_SCALE", o.scale.to_string())
        .env("TERMINATOR_TEST_ACTIONS", actions.to_string());
    if o.narrow {
        command.env("TERMINATOR_TEST_NARROW", "1");
    } else {
        command.env_remove("TERMINATOR_TEST_NARROW");
    }
    command.stdout(log.try_clone()?).stderr(log);
    let mut gui = Process(command.spawn()?);
    if let Err(error) = observe(&mut gui.0) {
        return Err(error.context(fs::read_to_string(&logpath).unwrap_or_default()));
    }
    // Let every scripted step land before asking for a graceful exit;
    // otherwise the marker can close the app before its actions fire.
    // Count lines, not distinct targets: repeated steps share a target.
    // The deadline fallback closes anyway if this is lost.
    let steps = actions.as_array().unwrap().len();
    let latest = actions
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|action| action.get("at_ms")?.as_u64())
        .max()
        .unwrap_or(0);
    poll(
        || {
            let logs = fs::read_to_string(&logpath).unwrap_or_default();
            Ok(logs.matches("Fixture action:").count() >= steps)
        },
        (latest / 1000).saturating_add(15),
        "fixture actions never landed",
    )
    .map_err(|error| error.context(fs::read_to_string(&logpath).unwrap_or_default()))?;
    fs::write(exit, b"exit")?;
    let status = wait_child(&mut gui.0, Duration::from_secs(90))
        .with_context(|| fs::read_to_string(&logpath).unwrap_or_default())?;
    let logs = fs::read_to_string(logpath)?;
    ensure!(status.success(), "GUI failed: {logs}");
    for action in actions.as_array().unwrap() {
        ensure!(
            logs.contains(&format!(
                "Fixture action: {}",
                action["target"].as_str().unwrap()
            )),
            "Action was not reached: {action}; {logs}"
        );
    }
    ensure!(
        logs.matches("Fixture action:").count() >= actions.as_array().unwrap().len(),
        "Some actions never fired; {logs}"
    );
    Ok(logs)
}

fn dock_leaves(layout: &Value) -> Vec<(&Vec<Value>, usize)> {
    let mut out = Vec::new();
    if let Some(groups) = layout.get("tabs").and_then(Value::as_array) {
        for group in groups {
            if let Some(dock) = group.get("layout") {
                leaf_tabs(dock, &mut out);
            }
        }
    }
    out
}

fn group_with(layout: &Value, sid: &str) -> Option<usize> {
    layout
        .get("tabs")?
        .as_array()?
        .iter()
        .position(|group| session_ids(&group["layout"]).iter().any(|id| id == sid))
}

fn session_label(state: &Value, sid: &str) -> Result<String> {
    sessions(state)
        .iter()
        .find(|s| id(s) == sid)
        .and_then(|s| s.get("label")?.as_str().map(str::to_owned))
        .ok_or_else(|| anyhow::anyhow!("missing label for {sid}"))
}

fn tab_key(sid: &str) -> String {
    format!(r#"{{"Terminal":"{sid}"}}"#)
}

fn top_tabs(layout: &Value) -> Result<usize> {
    layout
        .get("tabs")
        .and_then(Value::as_array)
        .map(Vec::len)
        .ok_or_else(|| anyhow::anyhow!("missing top-level tabs"))
}
