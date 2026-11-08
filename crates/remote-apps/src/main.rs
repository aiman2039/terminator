//! `terminator-remote-bridge`: the host side of remote app windows. The local
//! daemon starts it over SSH; it speaks the Terminator display protocol on
//! stdin/stdout. Usage:
//!
//! `terminator-remote-bridge --socket PATH [--display :N] [--wayland-display NAME]`

use std::process::ExitCode;

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    use std::path::PathBuf;

    use terminator_remote_apps::bridge::{self, Options};

    let mut socket = None;
    let mut display = None;
    let mut wayland_display = "wprs-0".to_owned();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next();
        match (arg.as_str(), value) {
            ("--socket", Some(v)) => socket = Some(PathBuf::from(v)),
            ("--display", Some(v)) => display = Some(v),
            ("--wayland-display", Some(v)) => wayland_display = v,
            _ => return report(&format!("unknown or incomplete argument: {arg}")),
        }
    }
    let Some(socket) = socket else {
        return report("--socket PATH is required");
    };
    let options = Options {
        socket: &socket,
        display,
        wayland_display,
    };
    match bridge::run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report(&error.to_string()),
    }
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    report("terminator-remote-bridge runs on Linux hosts only")
}

#[expect(
    clippy::print_stderr,
    reason = "stderr is the bridge's only human channel; stdout carries the protocol."
)]
fn report(message: &str) -> ExitCode {
    eprintln!("terminator-remote-bridge: {message}");
    ExitCode::from(2)
}
