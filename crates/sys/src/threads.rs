/// Every thread of `pid` is in the mach waiting state.
///
/// Returns false when the thread list is missing, truncated, or not entirely waiting.
pub fn all_threads_waiting(pid: u32) -> bool {
    // PROC_PIDLISTTHREADS = 6 in the macOS SDK's sys/proc_info.h. Query actual
    // thread IDs: thread ID zero can yield a misleading runnable fallback.
    let mut threads = [0u64; 256];
    let Ok(capacity) = i32::try_from(std::mem::size_of_val(&threads)) else {
        return false;
    };
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // SAFETY: `threads` is a 256-id buffer and `capacity` is its byte length.
    // `proc_pidinfo` writes at most that many bytes. A short or failed read is
    // rejected below.
    let bytes = unsafe { libc::proc_pidinfo(pid, 6, 0, threads.as_mut_ptr().cast(), capacity) };
    if bytes <= 0 || bytes >= capacity || bytes.checked_rem(8) != Some(0) {
        return false;
    }
    let Some(count) = usize::try_from(bytes).ok().and_then(|n| n.checked_div(8)) else {
        return false;
    };
    let Some(ids) = threads.get(..count) else {
        return false;
    };
    ids.iter().all(|thread| {
        // SAFETY: `proc_threadinfo` is a plain integer struct. All-zero is a valid bit pattern.
        let mut info: libc::proc_threadinfo = unsafe { std::mem::zeroed() };
        let Ok(size) = i32::try_from(std::mem::size_of_val(&info)) else {
            return false;
        };
        // SAFETY: `info` is valid for `size` bytes. `*thread` came from the list call above.
        let result = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTHREADINFO,
                *thread,
                (&raw mut info).cast(),
                size,
            )
        };
        result == size && info.pth_run_state == libc::TH_STATE_WAITING
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        process::{Child, Command},
        time::{Duration, Instant},
    };

    fn spawn(program: &str, args: &[&str]) -> Child {
        Command::new(program).args(args).spawn().unwrap()
    }

    #[test]
    fn a_sleeping_process_settles_into_all_waiting_threads() {
        let mut child = spawn("/bin/sleep", &["30"]);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut waiting = false;
        while Instant::now() < deadline {
            if all_threads_waiting(child.id()) {
                waiting = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(waiting, "a sleeping process should be entirely waiting");
    }

    #[test]
    fn a_busy_process_is_never_reported_all_waiting() {
        let mut child = spawn("/bin/sh", &["-c", "while :; do :; done"]);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut observed_busy = false;
        while Instant::now() < deadline {
            if !all_threads_waiting(child.id()) {
                observed_busy = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert!(observed_busy, "a running process should not be all waiting");
    }

    #[test]
    fn an_unknown_pid_is_not_considered_waiting() {
        assert!(!all_threads_waiting(u32::MAX));
    }
}
