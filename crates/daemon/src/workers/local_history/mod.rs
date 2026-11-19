//! Local History recorder: watches local project folders and saves file
//! snapshots in the shared data directory. Runs on its own thread; IPC
//! handlers only queue labels here.
mod scan;

use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::{
        Weak,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use terminator_core::{generations, local_history};

use crate::server::{Shared, relock};

/// Queued work for the recorder.
pub(crate) enum LocalHistoryJob {
    /// A timeline marker for every file under `root`.
    Label { root: PathBuf, label: String },
    /// A change the file watcher reported.
    Changed(Vec<PathBuf>),
}

/// A full queue dropped a job since the recorder last reported it.
static DROPPED: AtomicBool = AtomicBool::new(false);
const DROPPED_NOTE: &str = "Local History: the change queue was full; some edits have no snapshot";

/// Never blocks: a full queue drops the job and records that it did.
pub(crate) fn queue_local_history(tx: &mpsc::SyncSender<LocalHistoryJob>, job: LocalHistoryJob) {
    if matches!(tx.try_send(job), Err(mpsc::TrySendError::Full(_))) {
        DROPPED.store(true, Ordering::Release);
    }
}

fn report_dropped(shared: &Shared) {
    crate::console::Console::err(format_args!("{DROPPED_NOTE}"));
    shared.degraded(DROPPED_NOTE);
}

/// Events wait this long without a newer change before a snapshot.
const QUIET: Duration = Duration::from_millis(500);
const ROOTS_EVERY: Duration = Duration::from_secs(5);
const PRUNE_EVERY: Duration = Duration::from_hours(24);
/// After new snapshots, the size budget and per-file cap apply this soon.
const PRUNE_AFTER_RECORD: Duration = Duration::from_mins(1);
/// A root whose watch failed is tried again no sooner than this.
const RETRY_FAILED_WATCH: Duration = Duration::from_mins(1);
/// Files recorded per tick; the rest stay pending for later ticks (#192).
const MAX_BATCH: usize = 256;

pub(crate) struct LocalHistoryWorker {
    store: local_history::Store,
    tx: mpsc::SyncSender<LocalHistoryJob>,
    watcher: Option<notify::RecommendedWatcher>,
    /// Watched project roots; the recorder is on while this is non-empty.
    watched: BTreeSet<PathBuf>,
    /// Wanted roots whose watch failed, with the time of the last try.
    failed: HashMap<PathBuf, Instant>,
    days: u32,
    /// Size budget and per-file cap from the settings.
    limits: (u64, u32),
    pending: HashMap<PathBuf, Instant>,
    roots_checked: Option<Instant>,
    pruned: Option<Instant>,
    /// Snapshots were recorded since the last prune.
    recorded: bool,
}

impl LocalHistoryWorker {
    /// The store under the shared data directory.
    pub(crate) fn store(shared: &Shared) -> local_history::Store {
        let data = shared
            .catalog_paths
            .as_ref()
            .map_or(&shared.paths.data, |paths| &paths.data);
        local_history::Store::in_data(data)
    }

    pub(crate) fn spawn(weak: Weak<Shared>, rx: mpsc::Receiver<LocalHistoryJob>) {
        let Some((store, tx)) = weak
            .upgrade()
            .map(|shared| (Self::store(&shared), shared.local_history.clone()))
        else {
            return;
        };
        thread::spawn(move || {
            let mut worker = Self {
                store,
                tx,
                watcher: None,
                watched: BTreeSet::new(),
                failed: HashMap::new(),
                days: 0,
                limits: (
                    local_history::DEFAULT_MIB,
                    local_history::DEFAULT_FILE_ENTRIES,
                ),
                pending: HashMap::new(),
                roots_checked: None,
                pruned: None,
                recorded: false,
            };
            loop {
                let job = rx.recv_timeout(Duration::from_millis(250));
                if matches!(job, Err(mpsc::RecvTimeoutError::Disconnected)) {
                    break;
                }
                let Some(shared) = weak.upgrade() else {
                    break;
                };
                if shared.shutdown.load(Ordering::Acquire) {
                    break;
                }
                if let Ok(job) = job {
                    worker.apply(job);
                }
                // Drain before `tick`: it runs Git, and one job per pass
                // lets a burst of file events fill the queue.
                worker.drain(&rx);
                if DROPPED.swap(false, Ordering::AcqRel) {
                    report_dropped(&shared);
                }
                worker.tick(&shared);
            }
        });
    }

    fn drain(&mut self, rx: &mpsc::Receiver<LocalHistoryJob>) {
        while let Ok(job) = rx.try_recv() {
            self.apply(job);
        }
    }

    fn apply(&mut self, job: LocalHistoryJob) {
        match job {
            LocalHistoryJob::Label { root, label } => {
                if self.days > 0 {
                    let _ = self.store.label(&root, &label, local_history::now_ms());
                }
            }
            LocalHistoryJob::Changed(paths) => {
                let now = Instant::now();
                for path in paths {
                    if scan::eligible_path(&path) {
                        self.pending.insert(path, now);
                    }
                }
            }
        }
    }

    fn tick(&mut self, shared: &Shared) {
        if self
            .roots_checked
            .is_none_or(|at| at.elapsed() >= ROOTS_EVERY)
        {
            self.roots_checked = Some(Instant::now());
            self.refresh_roots(shared);
        }
        self.record_quiet();
        if self.prune_due() {
            self.pruned = Some(Instant::now());
            self.recorded = false;
            let (mib, file_entries) = self.limits;
            let limits = local_history::Limits::new(self.days, mib, file_entries);
            if let Err(error) = self.store.prune(local_history::now_ms(), limits) {
                shared.degraded(&format!("Local History retention: {error}"));
            }
        }
    }

    /// Daily, and soon after new snapshots so the size budget holds.
    fn prune_due(&self) -> bool {
        let every = if self.recorded {
            PRUNE_AFTER_RECORD
        } else {
            PRUNE_EVERY
        };
        self.days > 0 && self.pruned.is_none_or(|at| at.elapsed() >= every)
    }

    /// Watches the local roots of the active generation's projects, or
    /// nothing when recording is off.
    fn refresh_roots(&mut self, shared: &Shared) {
        let (days, limits, roots) = {
            let state = relock(&shared.state);
            let roots = scan::project_roots(state.projects.iter().map(|p| p.path.clone()));
            let settings = &state.settings;
            let limits = (
                settings.local_history_mib,
                settings.local_history_file_entries,
            );
            (settings.local_history_days, limits, roots)
        };
        self.days = days;
        self.limits = limits;
        let wanted = if days > 0 && Self::active(shared) {
            roots
        } else {
            BTreeSet::new()
        };
        // A wanted root whose watch failed is never in `watched`, so this
        // does not skip its retry.
        if wanted == self.watched {
            self.failed.clear();
            return;
        }
        if wanted.is_empty() {
            self.watcher = None;
            self.watched.clear();
            self.failed.clear();
            self.pending.clear();
            return;
        }
        self.rewatch(&wanted, Instant::now());
    }

    /// Watch the `wanted` roots. Only roots whose watch succeeded go in
    /// `watched`; the others wait in `failed` for their next try.
    fn rewatch(&mut self, wanted: &BTreeSet<PathBuf>, now: Instant) {
        use notify::Watcher;
        if self.watcher.is_none() {
            let tx = self.tx.clone();
            self.watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if let Ok(event) = event
                        && scan::content_event(&event.kind)
                    {
                        queue_local_history(&tx, LocalHistoryJob::Changed(event.paths));
                    }
                })
                .ok();
        }
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        for gone in self.watched.difference(wanted) {
            let _ = watcher.unwatch(gone);
        }
        self.watched.retain(|root| wanted.contains(root));
        self.failed.retain(|root, _| wanted.contains(root));
        let missing: Vec<PathBuf> = wanted.difference(&self.watched).cloned().collect();
        for added in &missing {
            // A root that cannot be watched (gone, inotify limit) is
            // tried again only after RETRY_FAILED_WATCH.
            let last = self.failed.get(added).copied();
            if last.is_some_and(|at| now.saturating_duration_since(at) < RETRY_FAILED_WATCH) {
                continue;
            }
            match watcher.watch(added, notify::RecursiveMode::Recursive) {
                Ok(()) => {
                    self.failed.remove(added);
                    self.watched.insert(added.clone());
                }
                Err(error) => {
                    if last.is_none() {
                        crate::console::Console::err(format_args!(
                            "Local History: cannot watch {}: {error}; trying again every minute",
                            added.display()
                        ));
                    }
                    self.failed.insert(added.clone(), now);
                }
            }
        }
    }

    /// Only the active generation records; a draining one stops watching.
    fn active(shared: &Shared) -> bool {
        let Some(root) = &shared.catalog_paths else {
            return true;
        };
        let generation = relock(&shared.state).generation.clone();
        generations::Catalog::open(root)
            .and_then(|catalog| catalog.active())
            .is_ok_and(|active| active.as_deref() == Some(generation.as_str()))
    }

    fn record_quiet(&mut self) {
        let ready: Vec<PathBuf> = self
            .pending
            .iter()
            .filter(|(_, at)| at.elapsed() >= QUIET)
            .map(|(path, _)| path.clone())
            .take(MAX_BATCH)
            .collect();
        if ready.is_empty() {
            return;
        }
        for path in &ready {
            self.pending.remove(path);
        }
        self.recorded = true;
        scan::record_batch(&self.store, &self.watched, ready);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_queue_is_reported_and_a_burst_is_drained_in_one_pass() {
        let data = tempfile::tempdir().unwrap();
        let (tx, rx) = mpsc::sync_channel(2);
        DROPPED.store(false, Ordering::Release);
        for i in 0..3 {
            queue_local_history(
                &tx,
                LocalHistoryJob::Changed(vec![PathBuf::from(format!("/p/{i}.rs"))]),
            );
        }
        assert!(
            DROPPED.swap(false, Ordering::AcqRel),
            "the third job was dropped"
        );
        let mut worker = worker(data.path(), tx.clone());
        worker.drain(&rx);
        assert_eq!(worker.pending.len(), 2);
        assert!(rx.try_recv().is_err());
    }

    fn worker(data: &std::path::Path, tx: mpsc::SyncSender<LocalHistoryJob>) -> LocalHistoryWorker {
        LocalHistoryWorker {
            store: local_history::Store::in_data(data),
            tx,
            watcher: None,
            watched: BTreeSet::new(),
            failed: HashMap::new(),
            days: 1,
            limits: (
                local_history::DEFAULT_MIB,
                local_history::DEFAULT_FILE_ENTRIES,
            ),
            pending: HashMap::new(),
            roots_checked: None,
            pruned: None,
            recorded: false,
        }
    }

    #[test]
    fn a_root_that_cannot_be_watched_is_retried_after_a_minute() {
        let data = tempfile::tempdir().unwrap();
        let (tx, _rx) = mpsc::sync_channel(16);
        let mut worker = worker(data.path(), tx);
        let root = data.path().join("project");
        let wanted = BTreeSet::from([root.clone()]);
        let start = Instant::now();
        // The folder is missing: the watch fails and the root waits.
        worker.rewatch(&wanted, start);
        assert!(worker.watched.is_empty());
        assert!(worker.failed.contains_key(&root));
        std::fs::create_dir(&root).unwrap();
        // No new try before the retry time.
        worker.rewatch(&wanted, start + Duration::from_secs(5));
        assert!(worker.watched.is_empty());
        assert_eq!(worker.failed.get(&root), Some(&start));
        // After it, the root is watched.
        worker.rewatch(&wanted, start + RETRY_FAILED_WATCH);
        assert_eq!(worker.watched, wanted);
        assert!(worker.failed.is_empty());
        // A root that is no longer wanted leaves both sets.
        worker.rewatch(&BTreeSet::from([data.path().join("other")]), start);
        assert!(worker.watched.is_empty());
        assert!(!worker.failed.contains_key(&root));
    }
}
