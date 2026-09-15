//! Conservative prompt evidence. Silence and absence of children are not readiness.
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

/// Input generations are daemon-owned; shell callbacks can only acknowledge them.
#[derive(Debug)]
pub struct PromptEvidence {
    pub generation: u64,
    candidate: Option<u64>,
    submitted: usize,
    boundary: bool,
    pub ready: bool,
    pub last_event: &'static str,
}
impl Default for PromptEvidence {
    fn default() -> Self {
        Self {
            generation: 0,
            candidate: Some(0),
            submitted: 0,
            boundary: false,
            ready: false,
            last_event: "none",
        }
    }
}
impl PromptEvidence {
    pub fn input(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.last_event = "input";
        self.generation = self.generation.saturating_add(1);
        self.ready = false;
        self.candidate = None;
        self.submitted = self
            .submitted
            .saturating_add(bytes.iter().filter(|b| matches!(b, b'\r' | b'\n')).count());
        self.boundary = bytes.last().is_some_and(|b| matches!(b, b'\r' | b'\n'));
    }
    pub fn begin(&mut self) -> u64 {
        self.last_event = "command";
        self.ready = false;
        if self.submitted > 0 {
            self.submitted -= 1;
            if self.submitted == 0 && self.boundary {
                self.candidate = Some(self.generation);
            }
        }
        self.generation
    }
    pub fn prompt(&mut self, generation: u64, jobs_empty: bool) {
        self.last_event = if jobs_empty {
            "prompt"
        } else {
            "prompt_jobs_busy"
        };
        self.ready = jobs_empty
            && self.candidate == Some(generation)
            && self.generation == generation
            && self.submitted == 0;
    }
}

/// A fresh process inventory, never a cached GUI observation.
pub fn verify_shell(session: &crate::Session, expected: &std::path::Path) -> anyhow::Result<u32> {
    use anyhow::{Context, ensure};
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};
    ensure!(
        session.kind == crate::SessionKind::Shell,
        "Editors require their own close flow"
    );
    let pid = session.pid.context("Shell identity is unavailable")?;
    let mut system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_processes(ProcessRefreshKind::nothing().with_exe(UpdateKind::Always)),
    );
    // sysinfo's first macOS inventory has BSD process state only; refresh the
    // owned shell once more to obtain its actual thread wait state.
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
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
    // PROC_PIDLISTTHREADS = 6 in the macOS SDK's sys/proc_info.h. Query actual
    // thread IDs: thread ID zero can yield a misleading runnable fallback.
    let mut threads = [0u64; 256];
    let capacity = std::mem::size_of_val(&threads) as i32;
    let bytes =
        unsafe { libc::proc_pidinfo(pid as i32, 6, 0, threads.as_mut_ptr().cast(), capacity) };
    if bytes <= 0 || bytes >= capacity || bytes % 8 != 0 {
        return false;
    }
    threads[..bytes as usize / 8].iter().all(|thread| {
        let mut info: libc::proc_threadinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        let result = unsafe {
            libc::proc_pidinfo(
                pid as i32,
                libc::PROC_PIDTHREADINFO,
                *thread,
                (&mut info as *mut libc::proc_threadinfo).cast(),
                size,
            )
        };
        result == size && info.pth_run_state == libc::TH_STATE_WAITING
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delayed_prompt_cannot_acknowledge_new_input() {
        let mut p = PromptEvidence::default();
        p.prompt(0, true);
        assert!(p.ready);
        p.input(b"read value\r");
        let generation = p.begin();
        assert!(!p.ready); // The builtin has no child, but is not at a prompt.
        p.input(b"new input");
        p.prompt(generation, true);
        assert!(!p.ready);
    }
    #[test]
    fn queued_commands_and_jobs_require_confirmation() {
        let mut p = PromptEvidence::default();
        p.input(b"true\rread value\r");
        let generation = p.begin();
        p.prompt(generation, true);
        assert!(!p.ready);
        p.begin();
        p.prompt(generation, false);
        assert!(!p.ready);
        p.prompt(generation, true);
        assert!(p.ready);
    }
}
