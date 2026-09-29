use std::{os::unix::process::CommandExt, process::Command};

/// `setsid` in the forked child, before `exec`.
///
/// The `pre_exec` closure only calls `setsid`.
pub fn detach_session(command: &mut Command) {
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

/// Double-fork and `setsid` so the eventual process is not a child of the caller.
///
/// The closure runs after `Command`'s fork and only calls `fork`, `setsid`, and `_exit`.
pub fn double_fork_setsid(command: &mut Command) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// `ps` may be unavailable in a minimal environment; skip rather than fail.
    fn process_group(pid: u32) -> Option<i64> {
        let output = Command::new("ps")
            .args(["-o", "pgid=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout).trim().parse().ok()
    }

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
}
