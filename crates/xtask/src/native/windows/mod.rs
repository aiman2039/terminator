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
    let end = std::time::Instant::now()
        .checked_add(Duration::from_secs(5))
        .ok_or_else(|| anyhow::anyhow!("window deadline overflow"))?;
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
    let window = snapshot.get("window");
    let gui = window
        .and_then(|window| window.get("gui_ppp"))
        .and_then(Value::as_f64)
        .unwrap_or(1.0);
    if cfg!(target_os = "macos") {
        let native = window
            .and_then(|window| window.get("native_ppp"))
            .and_then(Value::as_f64)
            .unwrap_or(gui);
        gui / native
    } else {
        gui
    }
}
/// The fixture GUI's window, driven through the platform desktop.
struct Window<'a> {
    h: &'a Harness,
    o: &'a Options,
    desktop: Desktop,
}
pub fn run(o: &Options) -> Result<()> {
    println!("Window fixture: preflight");
    Desktop::preflight()?;
    let h = Harness::new()?;
    h.setup()?;
    let p = h.project("window-controls")?;
    if cfg!(target_os = "macos") {
        use_test_shell(&h)?;
    }
    let shell = h.shell(&p)?;
    h.layout(&p, std::slice::from_ref(&shell))?;
    let mut gui = launch_gui(&h, o)?;
    let mut w = Window {
        h: &h,
        o,
        desktop: Desktop::new(gui.0.id(), &o.output)?,
    };
    w.drag_header()?;
    let resized = w.resize_corner()?;
    w.maximize_and_restore(resized)?;
    w.minimize_and_restore()?;
    w.file_picker()?;
    w.project_picker(&p)?;
    println!("Window fixture: native close");
    w.desktop.close()?;
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
fn use_test_shell(h: &Harness) -> Result<()> {
    let sink = h.root.join("terminator-test-shell");
    #[cfg(unix)]
    std::os::unix::fs::symlink(std::env::current_exe()?, &sink)?;
    h.set_setting("shell", json!(sink))
}
/// Start the GUI with native input and wait for its first capture.
fn launch_gui(h: &Harness, o: &Options) -> Result<Process> {
    fs::create_dir_all(&o.output)?;
    let capture = std::path::absolute(o.output.join("native-window.png"))?;
    if capture.exists() {
        fs::remove_file(&capture)?;
    }
    let log = fs::File::create(o.output.join("window.log"))?;
    let mut command = h.command("terminator");
    command
        // Desktop preflight requires an explicit macOS opt-in or isolated X11.
        .env_remove("TERMINATOR_TEST_BACKGROUND")
        .env("TERMINATOR_CAPTURE_PATH", &capture)
        .env("TERMINATOR_CAPTURE_AFTER_MS", "2200")
        .env("TERMINATOR_TEST_KEEP_OPEN", "1")
        .env("TERMINATOR_TEST_NATIVE_INPUT", "1")
        .env("TERMINATOR_TEST_SCALE", "1")
        .stdout(log.try_clone()?)
        .stderr(log);
    let gui = Process(command.spawn()?);
    wait(|| Ok(ui(h, Request::Ping).is_ok()))?;
    wait(|| Ok(capture.exists()))?;
    Ok(gui)
}
/// The centre of a GUI control rectangle `[x, y, width, height]`.
fn center(rect: &Value, geometry: &'static str) -> Result<(f64, f64)> {
    let axis = |index: usize| rect.get(index).and_then(Value::as_f64).context(geometry);
    Ok((axis(0)? + axis(2)? / 2.0, axis(1)? + axis(3)? / 2.0))
}
impl Window<'_> {
    fn drag_header(&mut self) -> Result<()> {
        let original = self.desktop.geometry()?;
        println!("Window fixture: drag from {original:?}");
        let snapshot = ui(self.h, Request::Snapshot)?;
        let point = snapshot
            .get("controls")
            .and_then(|controls| controls.get("header-drag"))
            .ok_or_else(|| anyhow::anyhow!("missing header drag control"))?;
        let (x, y) = center(point, "Missing header drag geometry")?;
        let scale = coordinate_scale(&snapshot);
        self.desktop.drag(
            (original[0] + x * scale, original[1] + y * scale),
            (
                original[0] + x * scale + 80.0,
                original[1] + y * scale + 50.0,
            ),
        )?;
        wait(|| {
            let r = self.desktop.geometry()?;
            Ok((r[0] - original[0]).abs() > 20.0 || (r[1] - original[1]).abs() > 20.0)
        })
    }
    /// Drag the south-east corner; returns the resized geometry.
    fn resize_corner(&mut self) -> Result<[f64; 4]> {
        let moved = self.desktop.geometry()?;
        println!("Window fixture: resize from {moved:?}");
        let snapshot = ui(self.h, Request::Snapshot)?;
        let corner = snapshot
            .get("controls")
            .and_then(|controls| controls.get("resize-se"))
            .unwrap_or(&Value::Null);
        println!("Window resize control: {corner}");
        let factor = coordinate_scale(&snapshot);
        let axis = |index: usize, fallback: f64| {
            corner
                .get(index)
                .and_then(Value::as_f64)
                .unwrap_or(fallback)
        };
        let x = axis(0, (moved[2] - 2.0) / factor) + axis(2, 0.0) / 2.0;
        let y = axis(1, (moved[3] - 2.0) / factor) + axis(3, 0.0) / 2.0;
        self.desktop.drag(
            (moved[0] + x * factor, moved[1] + y * factor),
            (moved[0] + x * factor + 100.0, moved[1] + y * factor + 70.0),
        )?;
        wait(|| {
            let r = self.desktop.geometry()?;
            Ok((r[2] - moved[2]).abs() > 20.0 && (r[3] - moved[3]).abs() > 20.0)
        })?;
        self.desktop.geometry()
    }
    fn maximize_and_restore(&self, resized: [f64; 4]) -> Result<()> {
        println!("Window fixture: maximize from {resized:?}");
        ui(
            self.h,
            Request::Window {
                action: "maximize".into(),
            },
        )?;
        wait(|| {
            let r = self.desktop.geometry()?;
            Ok(r[2] > resized[2] + 20.0 || r[3] > resized[3] + 20.0)
        })?;
        ui(
            self.h,
            Request::Window {
                action: "restore".into(),
            },
        )?;
        wait(|| {
            let r = self.desktop.geometry()?;
            Ok((r[2] - resized[2]).abs() < 10.0)
        })
    }
    fn minimize_and_restore(&mut self) -> Result<()> {
        println!("Window fixture: minimize");
        self.desktop.minimize()?;
        wait(|| self.desktop.minimized())?;
        println!("Window fixture: restore minimized window");
        self.desktop.restore()?;
        wait(|| Ok(!self.desktop.minimized()?))?;
        ui(
            self.h,
            Request::Window {
                action: "focus".into(),
            },
        )?;
        thread::sleep(Duration::from_millis(250));
        Ok(())
    }
    // Native file chooser: open, choose a fixture file, then cancel a second picker.
    fn file_picker(&mut self) -> Result<()> {
        let Self { h, desktop, .. } = self;
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
        Ok(())
    }
    fn project_picker(&mut self, p: &Value) -> Result<()> {
        let h = self.h;
        println!("Window fixture: project picker");
        // Selecting the directory just used by the file picker also verifies that
        // native folder selection idempotently selects an existing project.
        let other = h.project("picker-other")?;
        ensure!(
            h.state()?
                .get("selected_project")
                .is_some_and(|selected| selected == id(&other)),
            "Fixture selection setup failed"
        );
        self.click_project_add()?;
        let folder = PathBuf::from(
            p.get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("missing project path"))?,
        );
        thread::sleep(Duration::from_millis(700));
        self.desktop.choose_path(folder.to_str().unwrap())?;
        h.wait(
            |state| {
                state["selected_project"] == id(p)
                    && fs::read_to_string(self.o.output.join("window.log"))
                        .is_ok_and(|s| s.contains("Fixture native project picker selected=true"))
            },
            8,
        )?;
        ensure!(
            h.state()?
                .get("projects")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow::anyhow!("missing projects"))?
                .len()
                == 2,
            "Native folder picker duplicated project"
        );
        Ok(())
    }
    fn click_project_add(&mut self) -> Result<()> {
        let snapshot = ui(self.h, Request::Snapshot)?;
        let r = self.desktop.geometry()?;
        let button = snapshot
            .get("controls")
            .and_then(|controls| controls.get("project-add"))
            .ok_or_else(|| anyhow::anyhow!("missing project picker button"))?;
        let scale = coordinate_scale(&snapshot);
        println!("Window project picker button: {button}, geometry={r:?}, scale={scale}");
        let (x, y) = center(button, "Missing project picker geometry")?;
        self.desktop.click(r[0] + x * scale, r[1] + y * scale)
    }
}
