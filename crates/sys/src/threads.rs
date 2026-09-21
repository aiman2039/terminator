/// Every thread of `pid` is in the mach waiting state.
///
/// Returns false when the thread list is missing, truncated, or not entirely waiting.
pub fn all_threads_waiting(pid: u32) -> bool {
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
                (&raw mut info).cast(),
                size,
            )
        };
        result == size && info.pth_run_state == libc::TH_STATE_WAITING
    })
}
