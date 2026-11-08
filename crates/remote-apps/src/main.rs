//! `terminator-remote-bridge`: the host side of remote app windows. The local
//! daemon starts it over SSH; it speaks the Terminator display protocol on
//! stdin/stdout. Usage:
//!
//! `terminator-remote-bridge --ensure`: find or start this user's wprsd
//! with Xwayland, then serve it (what the daemon runs).
//!
//! `terminator-remote-bridge --socket PATH [--display :N] [--wayland-display NAME]`:
//! serve a wprsd that is already running.

use std::process::ExitCode;

#[cfg(all(target_os = "linux", feature = "bridge"))]
fn main() -> ExitCode {
    use std::path::PathBuf;

    use terminator_remote_apps::bridge::{self, Options};

    let mut socket = None;
    let mut ensure = false;
    let mut display = None;
    let mut wayland_display = "wprs-0".to_owned();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--ensure" {
            ensure = true;
            continue;
        }
        let value = args.next();
        match (arg.as_str(), value) {
            ("--socket", Some(v)) => socket = Some(PathBuf::from(v)),
            ("--display", Some(v)) => display = Some(v),
            ("--wayland-display", Some(v)) => wayland_display = v,
            _ => return report(&format!("unknown or incomplete argument: {arg}")),
        }
    }
    if ensure == socket.is_some() {
        return report("give exactly one of --ensure or --socket PATH");
    }
    let options = Options {
        socket,
        display,
        wayland_display,
    };
    match bridge::run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report(&error.to_string()),
    }
}

#[cfg(not(all(target_os = "linux", feature = "bridge")))]
fn main() -> ExitCode {
    report("terminator-remote-bridge runs on Linux hosts, built with --features bridge")
}

#[expect(
    clippy::print_stderr,
    reason = "stderr is the bridge's only human channel; stdout carries the protocol."
)]
fn report(message: &str) -> ExitCode {
    eprintln!("terminator-remote-bridge: {message}");
    ExitCode::from(2)
}
