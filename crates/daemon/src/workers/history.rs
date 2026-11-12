use std::{
    sync::{Weak, atomic::Ordering, mpsc},
    thread,
    time::{Duration, Instant},
};

use crate::server::{HistoryJob, Shared, relock};
use crate::storage;

/// Warning prefixes; each clears when its source succeeds again.
const FLUSH_FAILED: &str = "History flush failed: ";
const CLEANUP_FAILED: &str = "History cleanup failed: ";

/// Owns scrollback storage: applies queued jobs and periodically flushes
/// and prunes history.
pub(crate) struct HistoryWorker {
    history: storage::History,
    last_flush: Instant,
    last_prune: Instant,
    previous_settings: Option<(u64, u64, u64)>,
    /// History limits only; refreshed when they change so appends do not
    /// clone the full Settings under the state lock (#236).
    limits: terminator_core::Settings,
}

impl HistoryWorker {
    pub(crate) fn spawn(weak: Weak<Shared>, jobs: mpsc::Receiver<HistoryJob>) {
        thread::spawn(move || {
            let Some(mut worker) = Self::start(&weak) else {
                return;
            };
            worker.run(&weak, &jobs);
        });
    }

    fn start(weak: &Weak<Shared>) -> Option<Self> {
        let shared = weak.upgrade()?;
        let history = match storage::History::new(shared.paths.clone()) {
            Ok(h) => h,
            Err(e) => {
                shared.degraded(&format!("History initialization failed: {e}"));
                return None;
            }
        };
        drop(shared);
        Some(Self {
            history,
            last_flush: Instant::now(),
            last_prune: Instant::now(),
            previous_settings: None,
            limits: terminator_core::Settings::default(),
        })
    }

    fn run(&mut self, weak: &Weak<Shared>, jobs: &mpsc::Receiver<HistoryJob>) {
        loop {
            let Some(s) = weak.upgrade() else { break };
            match jobs.recv_timeout(Duration::from_millis(250)) {
                Ok(job) => self.handle_job(&s, job),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                _ => {}
            }
            self.maintain(&s);
            if s.shutdown.load(Ordering::Relaxed) {
                break;
            }
        }
        let _ = self.history.flush();
    }

    fn handle_job(&mut self, s: &Shared, job: HistoryJob) {
        let history = &mut self.history;
        match job {
            HistoryJob::Prune(budget, tx) => {
                let settings = relock(&s.state).settings.clone();
                let _ = tx.send(
                    history
                        .prune_with_budget(&settings, budget)
                        .map_err(|e| e.to_string()),
                );
            }
            HistoryJob::Append(sid, data) => {
                if let Err(e) = history.append(&sid, &data) {
                    let mut state = relock(&s.state);
                    if let Some(record) = state.sessions.iter_mut().find(|r| r.id == sid) {
                        record.truncated = true;
                    }
                    state.degraded = Some(format!(
                        "History write failed; some terminal history was not saved: {e}"
                    ));
                    state.revision = state.revision.saturating_add(1);
                }
            }
            HistoryJob::Flush(tx) => {
                let _ = tx.send(history.flush().map_err(|e| e.to_string()));
            }
            HistoryJob::Clear(session, remove, tx) => {
                let _ = tx.send(
                    history
                        .clear(session.as_deref(), remove)
                        .map_err(|e| e.to_string()),
                );
            }
        }
    }

    fn maintain(&mut self, s: &Shared) {
        if self.last_flush.elapsed() >= Duration::from_millis(250) {
            match self.history.flush() {
                Err(e) => s.degraded(&format!("{FLUSH_FAILED}{e}")),
                Ok(()) => s.recovered(FLUSH_FAILED),
            }
            self.last_flush = Instant::now();
        }
        let limits = {
            let state = relock(&s.state);
            let settings = &state.settings;
            (
                settings.history_days,
                settings.session_mib,
                settings.total_mib,
            )
        };
        let changed = self.previous_settings != Some(limits);
        if changed {
            (
                self.limits.history_days,
                self.limits.session_mib,
                self.limits.total_mib,
            ) = limits;
        }
        if changed
            || self.history.exceeds(&self.limits)
            || self.last_prune.elapsed() >= Duration::from_secs(30)
        {
            self.prune(s);
            self.previous_settings = Some(limits);
            self.last_prune = Instant::now();
        }
    }

    fn prune(&mut self, s: &Shared) {
        match self.history.prune(&self.limits) {
            Ok(ids) if !ids.is_empty() => {
                let mut state = relock(&s.state);
                for record in &mut state.sessions {
                    if ids.contains(&record.id) {
                        record.truncated = true;
                    }
                }
                state.revision = state.revision.saturating_add(1);
                drop(state);
                if let Err(e) = s.persist() {
                    s.degraded(&format!("History truncation not saved: {e}"));
                }
            }
            Ok(_) => s.recovered(CLEANUP_FAILED),
            Err(e) => s.degraded(&format!("{CLEANUP_FAILED}{e}")),
        }
    }
}
