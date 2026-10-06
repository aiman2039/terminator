//! Process and process-group signals. Pid 0 and pid 1 are refused for group signals.
//!
//! Windows has no POSIX signals. `Kill`, `Term`, and `Hangup` terminate the
//! target (`taskkill /F`; process trees with `/T` for groups), while
//! `Stop`/`Cont` have no counterpart and fail as unsupported. Liveness uses
//! `sysinfo`, which is cross-platform.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcSignal {
    Hangup,
    Kill,
    Term,
    Stop,
    Cont,
}

#[cfg(unix)]
fn rustix_signal(signal: ProcSignal) -> rustix::process::Signal {
    match signal {
        ProcSignal::Hangup => rustix::process::Signal::HUP,
        ProcSignal::Kill => rustix::process::Signal::KILL,
        ProcSignal::Term => rustix::process::Signal::TERM,
        ProcSignal::Stop => rustix::process::Signal::STOP,
        ProcSignal::Cont => rustix::process::Signal::CONT,
    }
}

#[cfg(unix)]
fn pid_from(pid: u32) -> std::io::Result<rustix::process::Pid> {
    let raw = i32::try_from(pid).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "process id out of range")
    })?;
    rustix::process::Pid::from_raw(raw).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "process id must be non-zero",
        )
    })
}

#[cfg(unix)]
pub fn signal_group(pid: u32, signal: ProcSignal) -> std::io::Result<()> {
    let pid = pid_from(pid)?;
    if pid.as_raw_pid() <= 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to signal process group 0 or 1",
        ));
    }
    rustix::process::kill_process_group(pid, rustix_signal(signal)).map_err(std::io::Error::from)
}

#[cfg(unix)]
pub fn signal_process(pid: u32, signal: ProcSignal) -> std::io::Result<()> {
    let pid = pid_from(pid)?;
    rustix::process::kill_process(pid, rustix_signal(signal)).map_err(std::io::Error::from)
}

#[cfg(unix)]
#[must_use]
pub fn process_alive(pid: u32) -> bool {
    let Ok(pid) = pid_from(pid) else {
        return false;
    };
    match rustix::process::test_kill_process(pid) {
        Ok(()) => true,
        Err(rustix::io::Errno::SRCH) => false,
        Err(_) => true,
    }
}

/// `ESRCH`: the kernel has no such process. Other errors, including `EPERM`, do not count.
#[cfg(unix)]
#[must_use]
pub fn process_gone(pid: u32) -> bool {
    let Ok(pid) = pid_from(pid) else {
        return false;
    };
    matches!(
        rustix::process::test_kill_process(pid),
        Err(rustix::io::Errno::SRCH)
    )
}

#[cfg(not(unix))]
fn refuse_system(pid: u32) -> std::io::Result<()> {
    if pid <= 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "refusing to signal process group 0 or 1",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_supported(signal: ProcSignal) -> std::io::Result<()> {
    match signal {
        ProcSignal::Hangup | ProcSignal::Kill | ProcSignal::Term => Ok(()),
        ProcSignal::Stop | ProcSignal::Cont => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "job suspension is not supported on Windows",
        )),
    }
}

/// Terminate the process tree rooted at `pid` (`taskkill /T /F`).
#[cfg(not(unix))]
pub fn signal_group(pid: u32, signal: ProcSignal) -> std::io::Result<()> {
    refuse_system(pid)?;
    check_supported(signal)?;
    let status = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "taskkill of process tree {pid} failed: {status}"
        )))
    }
}

/// Terminate `pid` (`taskkill /F`, without its child tree).
#[cfg(not(unix))]
pub fn signal_process(pid: u32, signal: ProcSignal) -> std::io::Result<()> {
    refuse_system(pid)?;
    check_supported(signal)?;
    let status = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "taskkill of process {pid} failed: {status}"
        )))
    }
}

#[cfg(not(unix))]
#[must_use]
pub fn process_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    if pid <= 1 {
        return true;
    }
    let mut system = System::new();
    let target = Pid::from_u32(pid);
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[target]),
        false,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Never),
    );
    system.process(target).is_some()
}

/// A missing process entry means the process is gone.
#[cfg(not(unix))]
#[must_use]
pub fn process_gone(pid: u32) -> bool {
    !process_alive(pid)
}

#[cfg(test)]
mod tests {
    #[cfg(not(unix))]
    use super::process_gone;
    use super::{ProcSignal, process_alive, signal_group};

    #[test]
    fn signal_group_refuses_init() {
        let error = signal_group(1, ProcSignal::Hangup).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn current_process_is_alive() {
        assert!(process_alive(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn hangup_ends_a_spawned_process_group() {
        use std::os::unix::process::CommandExt;
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 30"]);
        command.process_group(0);
        let mut child = command.spawn().unwrap();
        let pid = child.id();
        std::thread::sleep(std::time::Duration::from_millis(50));
        signal_group(pid, ProcSignal::Hangup).unwrap();
        let started = std::time::Instant::now();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(
                started.elapsed() < std::time::Duration::from_secs(2),
                "process group did not exit after SIGHUP"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[cfg(not(unix))]
    #[test]
    fn kill_ends_a_spawned_process() {
        // `ping -n` with a large count sleeps in small increments.
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "ping -n 30 127.0.0.1 >nul"])
            .spawn()
            .unwrap();
        let pid = child.id();
        assert!(process_alive(pid));
        signal_group(pid, ProcSignal::Hangup).unwrap();
        let started = std::time::Instant::now();
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            assert!(
                started.elapsed() < std::time::Duration::from_secs(10),
                "process tree did not exit after taskkill"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(process_gone(pid));
    }
}
