//! These tests operate only on the fixture GUI's PID. Platform permissions and
//! a real window manager are required; absence is an explicit error, not a pass.
use super::{
    Context, Duration, Harness, Options, Path, PathBuf, Process, Result, Value, ensure, fs, id,
    json, sessions, thread, wait_child,
};
use terminator_core::{
    Paths,
    ui_control::{self, Request},
};
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod x11;
#[cfg(target_os = "macos")]
use macos::Desktop;
#[cfg(target_os = "linux")]
use x11::Desktop;
fn wait(mut check: impl FnMut() -> Result<bool>) -> Result<()> {
    let end = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if check()? {
            return Ok(());
        }
        ensure!(
            std::time::Instant::now() < end,
            "Window-manager assertion timed out"
        );
        thread::sleep(Duration::from_millis(80));
    }
}
fn ui(h: &Harness, request: Request) -> Result<Value> {
    ui_control::rpc(&Paths::at(h.root.clone()), request)
}
fn coordinate_scale(snapshot: &Value) -> f64 {
    let gui = snapshot["window"]["gui_ppp"].as_f64().unwrap_or(1.0);
    if cfg!(target_os = "macos") {
        gui / snapshot["window"]["native_ppp"].as_f64().unwrap_or(gui)
    } else {
        gui
    }
}
pub fn run(o: &Options) -> Result<()> {
    println!("Window fixture: preflight");
    Desktop::preflight()?;
    let h = Harness::new()?;
    h.setup()?;
    let p = h.project("window-controls")?;
    if cfg!(target_os = "macos") {
        let sink = h.root.join("terminator-test-shell");
        std::os::unix::fs::symlink(std::env::current_exe()?, &sink)?;
        let mut settings = h.state()?["settings"].clone();
        settings["shell"] = json!(sink);
        h.rpc(json!({"Settings":settings}))?;
    }
    let shell = h.shell(&p)?;
    h.layout(&p, std::slice::from_ref(&shell))?;
    fs::create_dir_all(&o.output)?;
    let capture = std::path::absolute(o.output.join("native-window.png"))?;
    if capture.exists() {
        fs::remove_file(&capture)?;
    }
    let log = fs::File::create(o.output.join("window.log"))?;
    let mut command = h.command("terminator");
    command
        .env("TERMINATOR_CAPTURE_PATH", &capture)
        .env("TERMINATOR_CAPTURE_AFTER_MS", "2200")
        .env("TERMINATOR_TEST_KEEP_OPEN", "1")
        .env("TERMINATOR_TEST_NATIVE_INPUT", "1")
        .env("TERMINATOR_TEST_SCALE", "1")
        .stdout(log.try_clone()?)
        .stderr(log);
    let mut gui = Process(command.spawn()?);
    wait(|| Ok(ui(&h, Request::Ping).is_ok()))?;
    wait(|| Ok(capture.exists()))?;
    let mut desktop = Desktop::new(gui.0.id(), &o.output)?;
    let original = desktop.geometry()?;
    println!("Window fixture: drag from {original:?}");
    let snapshot = ui(&h, Request::Snapshot)?;
    let point = &snapshot["controls"]["header-drag"];
    let x = point[0].as_f64().context("Missing header drag geometry")?
        + point[2].as_f64().unwrap() / 2.0;
    let y = point[1].as_f64().unwrap() + point[3].as_f64().unwrap() / 2.0;
    let scale = coordinate_scale(&snapshot);
    desktop.drag(
        (original[0] + x * scale, original[1] + y * scale),
        (
            original[0] + x * scale + 80.0,
            original[1] + y * scale + 50.0,
        ),
    )?;
    wait(|| {
        let r = desktop.geometry()?;
        Ok((r[0] - original[0]).abs() > 20.0 || (r[1] - original[1]).abs() > 20.0)
    })?;
    let moved = desktop.geometry()?;
    println!("Window fixture: resize from {moved:?}");
    let snapshot = ui(&h, Request::Snapshot)?;
    let corner = &snapshot["controls"]["resize-se"];
    println!("Window resize control: {corner}");
    let factor = coordinate_scale(&snapshot);
    let x = corner[0].as_f64().unwrap_or((moved[2] - 2.0) / factor)
        + corner[2].as_f64().unwrap_or(0.0) / 2.0;
    let y = corner[1].as_f64().unwrap_or((moved[3] - 2.0) / factor)
        + corner[3].as_f64().unwrap_or(0.0) / 2.0;
    desktop.drag(
        (moved[0] + x * factor, moved[1] + y * factor),
        (moved[0] + x * factor + 100.0, moved[1] + y * factor + 70.0),
    )?;
    wait(|| {
        let r = desktop.geometry()?;
        Ok((r[2] - moved[2]).abs() > 20.0 && (r[3] - moved[3]).abs() > 20.0)
    })?;
    let resized = desktop.geometry()?;
    println!("Window fixture: maximize from {resized:?}");
    ui(
        &h,
        Request::Window {
            action: "maximize".into(),
        },
    )?;
    wait(|| {
        let r = desktop.geometry()?;
        Ok(r[2] > resized[2] + 20.0 || r[3] > resized[3] + 20.0)
    })?;
    ui(
        &h,
        Request::Window {
            action: "restore".into(),
        },
    )?;
    wait(|| {
        let r = desktop.geometry()?;
        Ok((r[2] - resized[2]).abs() < 10.0)
    })?;
    println!("Window fixture: minimize");
    desktop.minimize()?;
    wait(|| desktop.minimized())?;
    println!("Window fixture: restore minimized window");
    desktop.restore()?;
    wait(|| Ok(!desktop.minimized()?))?;
    ui(
        &h,
        Request::Window {
            action: "focus".into(),
        },
    )?;
    thread::sleep(Duration::from_millis(250));
    // Native file chooser: open, choose a fixture file, then cancel a second picker.
    println!("Window fixture: file picker");
    let source = h.root.join("window-controls/picker.rs");
    fs::write(&source, "fn main() {}\n")?;
    desktop.open_file_shortcut()?;
    thread::sleep(Duration::from_millis(700));
    desktop.choose_path(source.to_str().unwrap())?;
    h.wait(
        |state| {
            sessions(state)
                .iter()
                .any(|s| s["file"] == source.to_string_lossy().as_ref())
        },
        8,
    )?;
    let count = sessions(&h.state()?).len();
    desktop.open_file_shortcut()?;
    thread::sleep(Duration::from_millis(600));
    desktop.escape()?;
    thread::sleep(Duration::from_millis(400));
    ensure!(
        sessions(&h.state()?).len() == count,
        "Canceled picker created a session"
    );
    println!("Window fixture: project picker");
    // Selecting the directory just used by the file picker also verifies that
    // native folder selection idempotently selects an existing project.
    let other = h.project("picker-other")?;
    ensure!(
        h.state()?["selected_project"] == id(&other),
        "Fixture selection setup failed"
    );
    let snapshot = ui(&h, Request::Snapshot)?;
    let r = desktop.geometry()?;
    let button = &snapshot["controls"]["project-add"];
    let scale = coordinate_scale(&snapshot);
    println!("Window project picker button: {button}, geometry={r:?}, scale={scale}");
    desktop.click(
        r[0] + (button[0].as_f64().unwrap() + button[2].as_f64().unwrap() / 2.0) * scale,
        r[1] + (button[1].as_f64().unwrap() + button[3].as_f64().unwrap() / 2.0) * scale,
    )?;
    let folder = PathBuf::from(p["path"].as_str().unwrap());
    thread::sleep(Duration::from_millis(700));
    desktop.choose_path(folder.to_str().unwrap())?;
    h.wait(
        |state| {
            state["selected_project"] == id(&p)
                && fs::read_to_string(o.output.join("window.log"))
                    .is_ok_and(|s| s.contains("Fixture native project picker selected=true"))
        },
        8,
    )?;
    ensure!(
        h.state()?["projects"].as_array().unwrap().len() == 2,
        "Native folder picker duplicated project"
    );
    println!("Window fixture: native close");
    desktop.close()?;
    ensure!(
        wait_child(&mut gui.0, Duration::from_secs(5))?.success(),
        "Native close failed"
    );
    h.assert_pids(&[shell])?;
    println!(
        "{}",
        json!({"native_header_drag":true,"edge_resize":true,"maximize_restore":true,"minimize_restore":true,"file_and_project_pickers":true,"picker_cancel":true,"native_close_keeps_shell":true})
    );
    Ok(())
}
