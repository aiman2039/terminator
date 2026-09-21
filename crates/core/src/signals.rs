//! Process and process-group signals. Pid 0 and pid 1 are refused for group signals.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcSignal {
    Hangup,
    Kill,
    Term,
    Stop,
    Cont,
}

fn rustix_signal(signal: ProcSignal) -> rustix::process::Signal {
    match signal {
        ProcSignal::Hangup => rustix::process::Signal::HUP,
        ProcSignal::Kill => rustix::process::Signal::KILL,
        ProcSignal::Term => rustix::process::Signal::TERM,
        ProcSignal::Stop => rustix::process::Signal::STOP,
        ProcSignal::Cont => rustix::process::Signal::CONT,
    }
}

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

pub fn signal_process(pid: u32, signal: ProcSignal) -> std::io::Result<()> {
    let pid = pid_from(pid)?;
    rustix::process::kill_process(pid, rustix_signal(signal)).map_err(std::io::Error::from)
}

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

#[cfg(test)]
mod tests {
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
}
