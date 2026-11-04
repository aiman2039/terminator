//! Linux X11 servers without XInput 2 (old Xvnc/NX hosts) make winit panic
//! at startup. Detect that before anything else and re-run Terminator inside
//! a nested X server (Xephyr, else xpra). The child restores the host
//! display for the daemon so shells are not tied to the nested server.
use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xinput::{self, ConnectionExt as _};

/// Set in the nested child so it does not nest again.
const NESTED: &str = "TERMINATOR_NESTED_X11";
/// Original `DISPLAY`/`XAUTHORITY`, restored for the daemon by the child.
const HOST_DISPLAY: &str = "TERMINATOR_HOST_DISPLAY";
const HOST_XAUTHORITY: &str = "TERMINATOR_HOST_XAUTHORITY";

/// `Some(exit status)` when Terminator ran nested (or could not start);
/// `None` when this process should start the GUI itself.
pub fn run_nested_if_needed() -> Option<i32> {
    if std::env::var_os(NESTED).is_some()
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var_os("TERMINATOR_NO_X11_NEST").is_some()
    {
        return None;
    }
    let display = std::env::var("DISPLAY").ok().filter(|d| !d.is_empty())?;
    let (screen_w, screen_h) = match probe(&display) {
        Probe::Supported | Probe::Unknown => return None,
        Probe::Missing(w, h) => (w, h),
    };
    terminator::console::Console::err(format_args!(
        "X server {display} has no XInput 2; starting Terminator in a nested X server."
    ));
    let size = (screen_w.clamp(900, 1440), screen_h.clamp(550, 900));
    let mut errors = Vec::new();
    match run_xephyr(&display, size) {
        Ok(status) => return Some(code(status)),
        Err(e) => errors.push(format!("Xephyr: {e:#}")),
    }
    match run_xpra(&display) {
        Ok(status) => return Some(code(status)),
        Err(e) => errors.push(format!("xpra: {e:#}")),
    }
    terminator::console::Console::err(format_args!("{}", instructions(&display, &errors)));
    Some(1)
}

enum Probe {
    Supported,
    Missing(u32, u32),
    /// Could not connect or query; let winit report it.
    Unknown,
}

fn probe(display: &str) -> Probe {
    let Ok((conn, screen)) = x11rb::connect(Some(display)) else {
        return Probe::Unknown;
    };
    let size = conn.setup().roots.get(screen).map_or((1440, 900), |s| {
        (u32::from(s.width_in_pixels), u32::from(s.height_in_pixels))
    });
    match conn.extension_information(xinput::X11_EXTENSION_NAME) {
        Ok(Some(_)) => {}
        Ok(None) => return Probe::Missing(size.0, size.1),
        Err(_) => return Probe::Unknown,
    }
    // Same request winit sends; an XI 1.x server answers BadRequest.
    match conn
        .xinput_xi_query_version(2, 2)
        .map(|cookie| cookie.reply())
    {
        Ok(Ok(reply)) if reply.major_version >= 2 => Probe::Supported,
        Ok(Ok(_) | Err(x11rb::errors::ReplyError::X11Error(_))) => Probe::Missing(size.0, size.1),
        _ => Probe::Unknown,
    }
}

fn code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

fn free_display() -> Result<u32> {
    (90..400)
        .find(|n| {
            !Path::new(&format!("/tmp/.X11-unix/X{n}")).exists()
                && !Path::new(&format!("/tmp/.X{n}-lock")).exists()
        })
        .context("no free X display number")
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// The current executable and arguments, with host display kept for the child.
fn child_command(host_display: &str) -> Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(std::env::args_os().skip(1))
        .env(NESTED, "1")
        .env(HOST_DISPLAY, host_display)
        .env(
            HOST_XAUTHORITY,
            std::env::var_os("XAUTHORITY").unwrap_or_default(),
        );
    Ok(command)
}

/// Kills the nested server when the GUI exits.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run_xephyr(host_display: &str, (w, h): (u32, u32)) -> Result<ExitStatus> {
    anyhow::ensure!(on_path("Xephyr"), "not found on PATH");
    anyhow::ensure!(on_path("xauth"), "xauth not found on PATH");
    let n = free_display()?;
    let nested = format!(":{n}");
    // Private cookie: other users on a shared host cannot attach.
    let auth = tempfile::Builder::new()
        .prefix("terminator-xauth-")
        .tempfile()?;
    let cookie = uuid::Uuid::new_v4().simple().to_string();
    let status = Command::new("xauth")
        .arg("-f")
        .arg(auth.path())
        .args(["-q", "add", &nested, "MIT-MAGIC-COOKIE-1", &cookie])
        .status()?;
    anyhow::ensure!(status.success(), "xauth add failed");
    let mut server = Server(
        Command::new("Xephyr")
            .arg(&nested)
            .arg("-auth")
            .arg(auth.path())
            .args(["-nolisten", "tcp", "-screen", &format!("{w}x{h}")])
            .spawn()?,
    );
    wait_for_socket(&mut server.0, n)?;
    let status = child_command(host_display)?
        .env("DISPLAY", &nested)
        .env("XAUTHORITY", auth.path())
        .status()?;
    Ok(status)
}

fn wait_for_socket(server: &mut Child, n: u32) -> Result<()> {
    let socket = format!("/tmp/.X11-unix/X{n}");
    let start = Instant::now();
    while !Path::new(&socket).exists() {
        if let Some(status) = server.try_wait()? {
            anyhow::bail!("exited early ({status})");
        }
        anyhow::ensure!(
            start.elapsed() < Duration::from_secs(10),
            "did not start in 10 s"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// Seamless mode: xpra draws the Terminator window on the host display.
fn run_xpra(host_display: &str) -> Result<ExitStatus> {
    anyhow::ensure!(on_path("xpra"), "not found on PATH");
    let n = free_display()?;
    let child = child_command(host_display)?;
    let words = std::iter::once(child.get_program())
        .chain(child.get_args())
        .map(|w| w.to_str().context("non UTF-8 argument"))
        .collect::<Result<Vec<_>>>()?;
    let start_child = shlex::try_join(words)?;
    let mut command = Command::new("xpra");
    command.args([
        "start",
        &format!(":{n}"),
        "--daemon=no",
        "--attach=yes",
        "--exit-with-children=yes",
        &format!("--start-child={start_child}"),
    ]);
    for (key, value) in child.get_envs() {
        if let Some(value) = value {
            command.env(key, value);
        }
    }
    Ok(command.status()?)
}

fn instructions(display: &str, errors: &[String]) -> String {
    format!(
        "\
Terminator cannot start: X server {display} does not support XInput 2
(winit needs XI 2.x; check with `xinput --version`).
Automatic nested server failed:
  {}

Fix one of these:
  1. Install Xephyr (and xauth), then run Terminator again:
       RHEL/Rocky: sudo dnf install xorg-x11-server-Xephyr xorg-x11-xauth
       Debian/Ubuntu: sudo apt install xserver-xephyr xauth
  2. Or start it by hand:
       Xephyr :99 -screen 1440x900 &
       DISPLAY=:99 terminator
  3. Or use xpra (Xvfb backend):
       xpra start :99 --start-child=terminator --exit-with-children=yes
       xpra attach :99
  4. Or ask IT for an X server with XI 2 (TigerVNC 1.10+, current Xorg).
Set TERMINATOR_NO_X11_NEST=1 to skip this check.",
        errors.join("\n  ")
    )
}
