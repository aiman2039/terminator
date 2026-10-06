#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::Command;

/// `setsid` in the forked child, before `exec`.
///
/// The `pre_exec` closure only calls `setsid`.
#[cfg(unix)]
pub fn detach_session(command: &mut Command) {
    // SAFETY: `pre_exec` runs in the forked child before exec. The closure only
    // calls async-signal-safe `setsid` and returns its error.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

/// A new process group without a console window, so Ctrl+C in the parent's
/// console does not reach the child. Windows has no fork; the child stays a
/// direct child of the caller.
#[cfg(windows)]
pub fn detach_session(command: &mut Command) {
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

/// Double-fork and `setsid` so the eventual process is not a child of the caller.
///
/// The closure runs after `Command`'s fork and only calls `fork`, `setsid`, and `_exit`.
#[cfg(unix)]
pub fn double_fork_setsid(command: &mut Command) {
    // SAFETY: `pre_exec` runs in the forked child before exec. The closure only
    // calls async-signal-safe `fork`, `setsid`, and `_exit`.
    unsafe {
        command.pre_exec(|| match libc::fork() {
            -1 => Err(std::io::Error::last_os_error()),
            0 => {
                if libc::setsid() < 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            }
            _ => libc::_exit(0),
        });
    }
}

/// No fork on Windows: same detached process-group spawn as [`detach_session`].
/// Recovery helpers must not assume the child is a grandchild here.
#[cfg(windows)]
pub fn double_fork_setsid(command: &mut Command) {
    detach_session(command);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::time::{Duration, Instant};

    /// `ps` may be unavailable in a minimal environment; skip rather than fail.
    #[cfg(unix)]
    fn process_group(pid: u32) -> Option<i64> {
        let output = Command::new("ps")
            .args(["-o", "pgid=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout).trim().parse().ok()
    }

    #[cfg(unix)]
    #[test]
    fn detach_session_makes_the_child_its_own_session_leader() {
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        detach_session(&mut command);
        let mut child = command.spawn().unwrap();
        let group = process_group(child.id());
        let _ = child.kill();
        let _ = child.wait();
        if let Some(group) = group {
            assert_eq!(group, i64::from(child.id()), "child must lead its group");
        }
    }

    #[cfg(unix)]
    #[test]
    fn double_fork_setsid_runs_the_command_beyond_the_direct_child() {
        let marker =
            std::env::temp_dir().join(format!("terminator-sys-double-fork-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("printf done > \"$1\"")
            .arg("sh")
            .arg(&marker);
        double_fork_setsid(&mut command);
        let mut direct = command.spawn().unwrap();
        direct.wait().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(marker.exists(), "grandchild did not run the command");
        let _ = std::fs::remove_file(&marker);
    }

    #[cfg(windows)]
    #[test]
    fn detach_session_spawns_a_runnable_child() {
        let marker =
            std::env::temp_dir().join(format!("terminator-sys-detach-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let mut command = Command::new("cmd");
        command
            .arg("/c")
            .arg(format!("echo done > \"{}\"", marker.display()));
        detach_session(&mut command);
        let mut direct = command.spawn().unwrap();
        direct.wait().unwrap();
        assert!(marker.exists(), "detached child did not run the command");
        let _ = std::fs::remove_file(&marker);
    }
}
