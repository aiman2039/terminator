//! Process-group idle close. Children or a non-shell foreground keep confirmation.
use serde::{Deserialize, Serialize};

pub const CAPABILITY: &str = "close-idle-sessions-v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Closed,
    AlreadyEnded,
    Busy,
    Unknown,
    StaleGeneration,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Outcome {
    pub session: String,
    pub status: Status,
    pub reason: String,
}

fn process_inventory() -> sysinfo::System {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System, UpdateKind};
    // Linux task entries include a shell's own worker threads. Only child
    // processes represent jobs; fish keeps worker threads alive while idle.
    System::new_with_specifics(
        RefreshKind::nothing().with_processes(
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_exe(UpdateKind::Always),
        ),
    )
}

/// A fresh process inventory, never a cached GUI observation.
pub fn verify_shell(session: &crate::Session, expected: &std::path::Path) -> anyhow::Result<u32> {
    use anyhow::{Context, ensure};
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
    ensure!(
        session.kind == crate::SessionKind::Shell,
        "Editors require their own close flow"
    );
    let pid = session.pid.context("Shell identity is unavailable")?;
    let mut system = process_inventory();
    // sysinfo's first macOS inventory has BSD process state only; refresh the
    // owned shell once more to obtain its actual thread wait state.
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        ProcessRefreshKind::nothing()
            .without_tasks()
            .with_exe(UpdateKind::Always),
    );
    let process = system
        .process(Pid::from_u32(pid))
        .context("Shell process is unavailable")?;
    ensure!(
        process.start_time().abs_diff(session.created) <= 2,
        "Shell identity changed"
    );
    ensure!(
        process.exe().and_then(|p| p.canonicalize().ok()).as_deref() == Some(expected),
        "Shell executable changed"
    );
    let name = process
        .exe()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str());
    ensure!(
        matches!(name, Some("zsh" | "bash" | "fish")),
        "Shell readiness is unsupported"
    );
    ensure!(
        shell_waiting(pid, process.status()),
        "Shell is not verifiably waiting"
    );
    ensure!(
        !system
            .processes()
            .values()
            .any(|p| p.parent() == Some(Pid::from_u32(pid))),
        "Shell has live descendants or jobs"
    );
    Ok(pid)
}

#[cfg(not(target_os = "macos"))]
fn shell_waiting(_pid: u32, status: sysinfo::ProcessStatus) -> bool {
    matches!(
        status,
        sysinfo::ProcessStatus::Sleep | sysinfo::ProcessStatus::Idle
    )
}

#[cfg(target_os = "macos")]
fn shell_waiting(pid: u32, _status: sysinfo::ProcessStatus) -> bool {
    terminator_sys::all_threads_waiting(pid)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::{process::Command, sync::mpsc, thread, time::Duration};

    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[test]
    fn idle_inventory_excludes_worker_threads_but_keeps_real_children() {
        let (tid_tx, tid_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let worker = thread::spawn(move || {
            let path = std::fs::read_link("/proc/thread-self").unwrap();
            let tid: u32 = path.file_name().unwrap().to_str().unwrap().parse().unwrap();
            tid_tx.send(tid).unwrap();
            let _ = release_rx.recv();
        });
        let tid = tid_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let child = Child(Command::new("sleep").arg("30").spawn().unwrap());
        let inventory = process_inventory();
        let child_process = inventory.process(sysinfo::Pid::from_u32(child.0.id()));
        // Release before assertions so a failed assertion cannot leave a worker blocked.
        drop(release_tx);
        worker.join().unwrap();
        assert!(inventory.process(sysinfo::Pid::from_u32(tid)).is_none());
        assert_eq!(
            child_process.unwrap().parent(),
            Some(sysinfo::Pid::from_u32(std::process::id()))
        );
    }
}
