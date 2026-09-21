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
