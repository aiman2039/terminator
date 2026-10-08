use anyhow::Result;
use portable_pty::MasterPty;
use std::{
    collections::HashMap,
    io::Write,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::AtomicBool,
        mpsc::{self, SyncSender},
    },
    time::Instant,
};
use terminator_core::*;

use crate::{helper, ntfy, storage, terminal_events};

/// Lock shared daemon state without cascading a previous panic. A minor bug
/// in one request must degrade that connection, never poison the daemon for
/// everyone: recovered state may be partially updated, but the next mutation
/// heals it, a corrupt store is quarantined on restart, and per-connection
/// `catch_unwind` keeps the panic itself contained.
pub(crate) fn relock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) struct Runtime {
    pub(crate) parser: vt100::Parser<terminal_events::Events>,
    pub(crate) master: Box<dyn MasterPty + Send>,
    pub(crate) writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub(crate) subscribers: Vec<SyncSender<Response>>,
    pub(crate) token: String,
    pub(crate) ended: bool,
    pub(crate) closing: bool,
    pub(crate) inputs_in_flight: usize,
    pub(crate) shell_executable: Option<std::path::PathBuf>,
}
/// A cloned input descriptor must not keep a failed output connection alive.
pub(crate) struct Disconnect(pub(crate) transport::Stream);
impl Drop for Disconnect {
    fn drop(&mut self) {
        let _ = self.0.shutdown();
    }
}
pub(crate) enum HistoryJob {
    Append(String, Vec<u8>),
    Prune(u64, mpsc::Sender<Result<Vec<String>, String>>),
    Flush(mpsc::Sender<Result<(), String>>),
    Clear(Option<String>, bool, mpsc::Sender<Result<(), String>>),
}
pub(crate) struct Shared {
    pub(crate) helper: helper::Helper,
    pub(crate) catalog_paths: Option<Paths>,
    pub(crate) state: Mutex<State>,
    pub(crate) store: Mutex<storage::Store>,
    pub(crate) sessions: Mutex<HashMap<String, Arc<Mutex<Runtime>>>>,
    pub(crate) paths: Paths,
    pub(crate) auth: String,
    pub(crate) focused: Mutex<(bool, Instant)>,
    pub(crate) worktree_operations: Mutex<()>,
    pub(crate) terminal_operations: Mutex<()>,
    pub(crate) shutdown: AtomicBool,
    pub(crate) history: SyncSender<HistoryJob>,
    pub(crate) alerts: SyncSender<String>,
    pub(crate) ntfy: SyncSender<ntfy::Ping>,
    pub(crate) ntfy_cooldown: Mutex<ntfy::Cooldown>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn poisoned_locks_recover_instead_of_cascading() {
        let mutex = Mutex::new(7u32);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = mutex.lock().unwrap();
            panic!("fixture panic while holding the lock");
        }));
        assert!(mutex.is_poisoned());
        // The daemon must keep serving on the recovered guard, not die on
        // the poison like `.lock().unwrap()` would.
        assert_eq!(*relock(&mutex), 7);
        *relock(&mutex) = 8;
        assert_eq!(*relock(&mutex), 8);
    }
}
