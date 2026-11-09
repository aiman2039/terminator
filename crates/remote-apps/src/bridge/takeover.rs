//! The newest bridge takes the windows. wprsd serves one client at a time,
//! so an older bridge (a dropped link the host has not noticed yet, a
//! restarted daemon, or another computer) must leave first.
//!
//! The new bridge writes its PID to `bridge.pid`. The old one checks the
//! file about once a second, reports [`ErrorKind::TakenOver`] and exits; if
//! it has not exited after [`GRACE`], the new bridge kills it.
//!
//! [`ErrorKind::TakenOver`]: crate::protocol::ErrorKind::TakenOver

use std::fs;
use std::io;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

const PID_FILE: &str = "bridge.pid";
/// How long the old bridge gets to leave on its own.
const GRACE: Duration = Duration::from_secs(3);

/// Makes this process the owner of `dir`'s wprsd and waits until the
/// previous owner is gone.
pub fn claim(dir: &Path) -> io::Result<()> {
    let own = std::process::id();
    let previous = owner(dir).filter(|pid| *pid != own && is_bridge(*pid));
    super::launch::write_private(dir, PID_FILE, own.to_string().as_bytes())?;
    let Some(previous) = previous else {
        return Ok(());
    };
    let deadline = Instant::now().checked_add(GRACE);
    while alive(previous) && deadline.is_some_and(|d| Instant::now() < d) {
        thread::sleep(Duration::from_millis(50));
    }
    if alive(previous)
        && let Ok(pid) = i32::try_from(previous)
    {
        let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
    }
    Ok(())
}

/// Whether no newer bridge claimed `dir`.
pub fn still_owner(dir: &Path) -> bool {
    owner(dir).is_none_or(|pid| pid == std::process::id())
}

fn owner(dir: &Path) -> Option<u32> {
    fs::read_to_string(dir.join(PID_FILE))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// A PID can be reused; only a process that is a bridge is waited for.
fn is_bridge(pid: u32) -> bool {
    fs::read(format!("/proc/{pid}/cmdline"))
        .is_ok_and(|cmd| String::from_utf8_lossy(&cmd).contains("terminator-remote-bridge"))
}

fn alive(pid: u32) -> bool {
    i32::try_from(pid).is_ok_and(|pid| kill(Pid::from_raw(pid), None).is_ok())
}
