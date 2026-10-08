//! Detach, float, in-split and stacked-tab flows through the public UI.
//!
//! Menu routing, layout assertions and per-action captures run anywhere
//! fixtures run. Observing the float window itself needs a real window
//! manager (macOS `screencapture`); closing it goes through the same
//! dock-back path as the OS close button via a marker file the fixture
//! writes once the window has been observed. Fixtures run foregrounded
//! (stealing focus briefly): background windows are denied Metal
//! drawables, so in-app screenshots never render without it.
use super::Options;
use crate::harness::{Harness, Process, id, session_ids, sessions, wait_child};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::{
    fs,
    path::PathBuf,
    process::Child,
    thread,
    time::{Duration, Instant},
};

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

fn project_layout(state: &Value) -> Result<&Value> {
    state
        .get("projects")
        .and_then(|projects| projects.get(0))
        .and_then(|project| project.get("layout"))
        .ok_or_else(|| anyhow::anyhow!("missing projects[0].layout"))
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
fn drive(
    h: &Harness,
    o: &Options,
    name: &str,
    actions: Value,
    after: u64,
    exit: &PathBuf,
    observe: impl FnOnce(&mut Child) -> Result<()>,
) -> Result<String> {
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
    if cfg!(target_os = "macos") {
        command
            .env("TERMINATOR_TEST_BACKGROUND", "1")
            .env("TERMINATOR_TEST_VISIBLE_CAPTURE", "1")
            .env("TERMINATOR_TEST_RENDER_OCCLUDED", "1");
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

#[cfg(target_os = "macos")]
fn float_window_id(pid: u32, title: &str) -> Option<u32> {
    use core_foundation::{boolean::CFBoolean, number::CFNumber, string::CFString};
    // NB: skip entries with missing keys instead of bailing: window-list
    // dictionaries for shadow/menu helpers routinely lack IsOnscreen or a
    // name, and `?` here used to abort the whole search before reaching
    // our window.
    let windows = terminator_sys::window_dictionaries()?;
    for dictionary in windows {
        let owner = terminator_sys::dictionary_value(&dictionary, "kCGWindowOwnerPID")
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|number| number.to_f64());
        if owner != Some(f64::from(pid)) {
            continue;
        }
        let onscreen = terminator_sys::dictionary_value(&dictionary, "kCGWindowIsOnscreen")
            .and_then(|value| value.downcast::<CFBoolean>())
            .map(bool::from);
        if onscreen != Some(true) {
            continue;
        }
        let name: Option<String> = terminator_sys::dictionary_value(&dictionary, "kCGWindowName")
            .and_then(|value| value.downcast::<CFString>())
            .map(|name| name.to_string());
        if name.as_deref() != Some(title) {
            continue;
        }
        return terminator_sys::dictionary_value(&dictionary, "kCGWindowNumber")
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|number| number.to_f64())
            .and_then(|id| {
                // Window numbers are small positive integers; the range
                // check leaves nothing to truncate or lose the sign of.
                // (`as` saturates, so the eager narrow is harmless.)
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let narrowed = id as u32;
                (id >= 0.0 && id <= f64::from(u32::MAX)).then_some(narrowed)
            });
    }
    None
}

#[cfg(target_os = "macos")]
fn pid_windows(pid: u32) -> Vec<String> {
    use core_foundation::{boolean::CFBoolean, number::CFNumber, string::CFString};
    let mut out = Vec::new();
    let Some(windows) = terminator_sys::window_dictionaries() else {
        return out;
    };
    for dictionary in windows {
        let owner = terminator_sys::dictionary_value(&dictionary, "kCGWindowOwnerPID")
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|number| number.to_f64());
        if owner != Some(f64::from(pid)) {
            continue;
        }
        let name: Option<String> = terminator_sys::dictionary_value(&dictionary, "kCGWindowName")
            .and_then(|value| value.downcast::<CFString>())
            .map(|name| name.to_string());
        let onscreen: Option<bool> =
            terminator_sys::dictionary_value(&dictionary, "kCGWindowIsOnscreen")
                .and_then(|value| value.downcast::<CFBoolean>())
                .map(bool::from);
        let number: Option<f64> = terminator_sys::dictionary_value(&dictionary, "kCGWindowNumber")
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|number| number.to_f64());
        out.push(format!(
            "name={name:?} onscreen={onscreen:?} number={number:?}"
        ));
    }
    out
}

#[cfg(target_os = "macos")]
fn windows_named(name: &str) -> Vec<String> {
    use core_foundation::{boolean::CFBoolean, number::CFNumber, string::CFString};
    let mut out = Vec::new();
    let Some(windows) = terminator_sys::window_dictionaries() else {
        out.push("window list unavailable".to_owned());
        return out;
    };
    for dictionary in windows {
        let title: Option<String> = terminator_sys::dictionary_value(&dictionary, "kCGWindowName")
            .and_then(|value| value.downcast::<CFString>())
            .map(|title| title.to_string());
        if title.as_deref() != Some(name) {
            continue;
        }
        let owner: Option<f64> = terminator_sys::dictionary_value(&dictionary, "kCGWindowOwnerPID")
            .and_then(|value| value.downcast::<CFNumber>())
            .and_then(|number| number.to_f64());
        let onscreen: Option<bool> =
            terminator_sys::dictionary_value(&dictionary, "kCGWindowIsOnscreen")
                .and_then(|value| value.downcast::<CFBoolean>())
                .map(bool::from);
        out.push(format!("owner={owner:?} onscreen={onscreen:?}"));
    }
    out
}

#[cfg(target_os = "macos")]
fn capture_window(id: u32, path: &PathBuf) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    let status = Command::new("screencapture")
        .args(["-x", "-l", &id.to_string()])
        .arg(path)
        .stderr(std::process::Stdio::null())
        .status()
        .context("screencapture failed")?;
    ensure!(status.success(), "screencapture failed");
    let bytes = path.metadata()?.len();
    ensure!(bytes > 1000, "float capture too small ({bytes} bytes)");
    Ok(())
}

/// Floats one pane through its caption menu, observes the standalone
/// window, closes it through the real dock-back path and asserts the
/// pane returned with its session untouched. `home` is the shell that
/// should keep sharing a tab with the floated pane, or `None` when the
/// floated pane is alone (its tab is dropped: the missing-home path).
fn float_close(
    h: &mut Harness,
    o: &Options,
    name: &str,
    shells: &[Value],
    floated: &Value,
    after: u64,
    exit: &PathBuf,
) -> Result<()> {
    let label = session_label(&h.state()?, id(floated))?;
    let marker = o.output.join(format!("{name}-close"));
    let _ = fs::remove_file(&marker);
    // An absent marker file is inert; the fixture creates it once the
    // standalone window has been observed.
    h.env.insert(
        "TERMINATOR_TEST_CLOSE_FLOATING".into(),
        marker.to_string_lossy().into_owned(),
    );
    let caption = format!("pane-caption:{label}");
    drive(
        h,
        o,
        name,
        json!([
            {"at_ms":1500,"target":caption,"right_click":true},
            {"at_ms":1600,"target":"Float window"},
        ]),
        after,
        exit,
        |child| {
            let pid = child.id();
            #[cfg(target_os = "macos")]
            {
                // The standalone window must appear outside the app.
                let output = std::path::absolute(&o.output)?;
                poll(
                    || Ok(float_window_id(pid, &label).is_some()),
                    60,
                    &format!(
                        "float window never appeared for {label}; pid windows: {:?}; named: {:?}",
                        pid_windows(pid),
                        windows_named(&label)
                    ),
                )?;
                let id = float_window_id(pid, &label).context("float window lost")?;
                // Best-effort pixels: `screencapture` refuses windows the
                // compositor has no content for yet (occluded fixtures may
                // never paint). Existence, title and dock-back below stay
                // strict; the PNG only illustrates when paint arrives.
                let shot = output.join(format!("{name}-floating.png"));
                let shot_poll = poll(|| Ok(capture_window(id, &shot).is_ok()), 10, "shot");
                match shot_poll {
                    Ok(()) => eprintln!("Observed float window {id} for {label}"),
                    Err(_) => eprintln!(
                        "Observed float window {id} for {label} (unpainted: no pixels captured)"
                    ),
                }
                // Close through the same dock-back path as the OS close
                // button, then require the window to disappear.
                fs::write(&marker, b"close")?;
                poll(
                    || Ok(float_window_id(pid, &label).is_none()),
                    15,
                    "float window never docked back",
                )?;
                eprintln!("Float window docked back for {label}");
                Ok(())
            }
            #[cfg(not(target_os = "macos"))]
            {
                fs::write(&marker, b"close")?;
                Ok(())
            }
        },
    )?;
    let _ = fs::remove_file(&marker);
    h.assert_pids(shells)?;
    let state = h.state()?;
    let layout = project_layout(&state)?;
    ensure!(
        group_with(layout, id(floated)).is_some(),
        "floated pane lost after close"
    );
    Ok(())
}

pub fn run(o: &Options) -> Result<()> {
    let t0 = Instant::now();
    let mut h = Harness::new()?;
    h.setup()?;
    // Foreground every fixture in this case (steals focus briefly):
    // background windows are denied Metal drawables, so in-app
    // screenshots never render without it.
    h.env
        .insert("TERMINATOR_TEST_FOREGROUND".into(), "1".into());
    // Shared graceful-exit marker (see `drive`); removed before each
    // scenario so a stale file can never close a fresh fixture.
    let exit = o.output.join("exit-marker");
    h.env.insert(
        "TERMINATOR_TEST_EXIT_MARKER".into(),
        exit.to_string_lossy().into_owned(),
    );
    let p = h.project("float-window")?;
    let shells: Vec<Value> = (0..2).map(|_| h.shell(&p)).collect::<Result<_>>()?;
    let first = shells.first().cloned().context("missing shell")?;

    // Float from a shared tab: the home tab survives, close docks back.
    h.layout(&p, &shells)?;
    let mut home_opts = o.clone();
    home_opts.output = o.output.join("float-home");
    float_close(
        &mut h,
        &home_opts,
        "float-home",
        &shells,
        &first,
        30000,
        &exit,
    )?;
    let state = h.state()?;
    let layout = project_layout(&state)?;
    ensure!(top_tabs(layout)? == 1, "float changed the top-level tabs");
    eprintln!("float-home done in {:.1}s", t0.elapsed().as_secs_f64());

    // Float a lone pane: its tab is dropped, close lands in a fresh tab.
    h.layout(&p, std::slice::from_ref(&first))?;
    let mut missing_opts = o.clone();
    missing_opts.output = o.output.join("float-missing");
    float_close(
        &mut h,
        &missing_opts,
        "float-missing",
        &shells,
        &first,
        25000,
        &exit,
    )?;
    let state = h.state()?;
    let layout = project_layout(&state)?;
    ensure!(
        top_tabs(layout)? == 1,
        "missing-home dock-back opened a tab"
    );
    ensure!(
        group_with(layout, id(&first)).is_some(),
        "floated pane lost after close"
    );
    eprintln!("float-missing done in {:.1}s", t0.elapsed().as_secs_f64());

    // Open a file inside the current split instead of a new top-level tab.
    let root = PathBuf::from(
        p.get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing project path"))?,
    );
    fs::write(root.join("note.md"), "# notes\n")?;
    h.layout(&p, std::slice::from_ref(&first))?;
    let mut stream = h.attach(&first)?;
    h.write(&mut stream, "printf '\\033[2J\\033[Hnote.md\\n'\n")?;
    drop(stream);
    let mut split_opts = o.clone();
    split_opts.output = o.output.join("insplit");
    let logs = drive(
        &h,
        &split_opts,
        "insplit",
        json!([
            {"at_ms":1500,"target":"terminal","hover":true},
            {"at_ms":2600,"target":"Open in current split"},
        ]),
        20000,
        &exit,
        |_| Ok(()),
    )?;
    ensure!(
        logs.contains("Terminal file menu opened:"),
        "file menu never opened: {logs}"
    );
    h.assert_pids(&shells)?;
    let state = h.state()?;
    let editor = sessions(&state)
        .iter()
        .find(|s| s["kind"] == "editor")
        .cloned()
        .context("no editor session created")?;
    let layout = project_layout(&state)?;
    ensure!(
        top_tabs(layout)? == 1,
        "in-split open created a top-level tab"
    );
    ensure!(
        dock_leaves(layout).iter().any(|(tabs, _)| tabs.len() == 2),
        "in-split open did not stack: {layout}"
    );
    eprintln!("insplit done in {:.1}s", t0.elapsed().as_secs_f64());

    // Leaf + stacks a third tab into the same split, no new top-level tab.
    let mut plus_opts = o.clone();
    plus_opts.output = o.output.join("stacked-plus");
    drive(
        &h,
        &plus_opts,
        "stacked-plus",
        json!([{"at_ms":1500,"target":"pane-plus"}]),
        14000,
        &exit,
        |_| Ok(()),
    )?;
    h.assert_pids(&shells)?;
    let state = h.state()?;
    let layout = project_layout(&state)?;
    ensure!(top_tabs(layout)? == 1, "leaf + opened a top-level tab");
    ensure!(
        dock_leaves(layout).iter().any(|(tabs, _)| tabs.len() == 3),
        "leaf + did not stack: {layout}"
    );
    eprintln!("stacked-plus done in {:.1}s", t0.elapsed().as_secs_f64());

    // Clicking a stacked tab switches to it.
    let state = h.state()?;
    let editor_sid = id(&editor).to_owned();
    let shell_label = session_label(&state, id(&first))?;
    let editor_label = session_label(&state, &editor_sid)?;
    let mut switch_opts = o.clone();
    switch_opts.output = o.output.join("stacked-switch");
    drive(
        &h,
        &switch_opts,
        "stacked-switch",
        json!([{"at_ms":1500,"target":format!("leaf-tab:{}",tab_key(&editor_sid))}]),
        12000,
        &exit,
        |_| Ok(()),
    )?;
    let state = h.state()?;
    let layout = project_layout(&state)?;
    let switched = dock_leaves(layout).iter().any(|(tabs, active)| {
        tabs.iter().position(|tab| {
            tab.get("Terminal").and_then(Value::as_str) == Some(editor_sid.as_str())
        }) == Some(*active)
    });
    ensure!(
        switched,
        "stacked switch did not select the editor: {layout}"
    );
    eprintln!("stacked-switch done in {:.1}s", t0.elapsed().as_secs_f64());

    // Detach to a new tab, then dock back through the submenu.
    let mut detach_opts = o.clone();
    detach_opts.output = o.output.join("detach-dockback");
    drive(
        &h,
        &detach_opts,
        "detach-dockback",
        json!([
            {"at_ms":1000,"target":format!("leaf-tab:{}", tab_key(id(&first)))},
            {"at_ms":1800,"target":format!("pane-caption:{shell_label}"),"right_click":true},
            {"at_ms":2600,"target":"Detach to new tab"},
            {"at_ms":4400,"target":format!("pane-caption:{shell_label}"),"right_click":true},
            {"at_ms":5200,"target":"Move to tab","hover":true},
            {"at_ms":6000,"target":editor_label},
        ]),
        16000,
        &exit,
        |_| Ok(()),
    )?;
    h.assert_pids(&shells)?;
    let state = h.state()?;
    let layout = project_layout(&state)?;
    ensure!(
        top_tabs(layout)? == 1,
        "dock-back left extra tabs: {layout}"
    );
    ensure!(
        dock_leaves(layout).iter().any(|(tabs, _)| tabs.len() == 3),
        "dock-back did not restore the stack: {layout}"
    );
    eprintln!("detach-dockback done in {:.1}s", t0.elapsed().as_secs_f64());

    // Real pane drag to the strip +: press-hold, moves, release.
    h.layout(&p, &shells)?;
    let state = h.state()?;
    let shell_label = session_label(&state, id(&first))?;
    let mut drag_opts = o.clone();
    drag_opts.output = o.output.join("drag-detach");
    drive(
        &h,
        &drag_opts,
        "drag-detach",
        json!([
            {"at_ms":1500,"target":format!("pane-caption:{shell_label}"),"hold":true},
            {"at_ms":2300,"target":"workspace-plus","hover":true},
            {"at_ms":3000,"target":"workspace-plus","hover":true},
            {"at_ms":3700,"target":"workspace-plus","release_button":true},
        ]),
        12000,
        &exit,
        |_| Ok(()),
    )?;
    h.assert_pids(&shells)?;
    let state = h.state()?;
    let layout = project_layout(&state)?;
    ensure!(top_tabs(layout)? == 2, "drag did not detach: {layout}");
    ensure!(
        group_with(layout, id(&first)).is_some(),
        "dragged pane lost"
    );
    eprintln!("drag-detach done in {:.1}s", t0.elapsed().as_secs_f64());
    eprintln!(
        "float-window suite done in {:.1}s",
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}
