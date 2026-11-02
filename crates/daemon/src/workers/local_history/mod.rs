//! Local History recorder: watches local project folders and saves file
//! snapshots in the shared data directory. Runs on its own thread; IPC
//! handlers only queue labels here.
mod scan;

use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::{Weak, atomic::Ordering, mpsc},
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

/// Events wait this long without a newer change before a snapshot.
const QUIET: Duration = Duration::from_millis(500);
const ROOTS_EVERY: Duration = Duration::from_secs(5);
const PRUNE_EVERY: Duration = Duration::from_hours(24);
/// Files recorded per tick; the rest stay pending for later ticks (#192).
const MAX_BATCH: usize = 256;

pub(crate) struct LocalHistoryWorker {
    store: local_history::Store,
    tx: mpsc::SyncSender<LocalHistoryJob>,
    watcher: Option<notify::RecommendedWatcher>,
    /// Watched project roots; the recorder is on while this is non-empty.
    watched: BTreeSet<PathBuf>,
    days: u32,
    pending: HashMap<PathBuf, Instant>,
    roots_checked: Option<Instant>,
    pruned: Option<Instant>,
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
                days: 0,
                pending: HashMap::new(),
                roots_checked: None,
                pruned: None,
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
                worker.tick(&shared);
            }
        });
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
        if self.days > 0 && self.pruned.is_none_or(|at| at.elapsed() >= PRUNE_EVERY) {
            self.pruned = Some(Instant::now());
            if let Err(error) = self.store.prune(local_history::now_ms(), self.days) {
                shared.degraded(&format!("Local History retention: {error}"));
            }
        }
    }

    /// Watches the local roots of the active generation's projects, or
    /// nothing when recording is off.
    fn refresh_roots(&mut self, shared: &Shared) {
        let (days, roots) = {
            let state = relock(&shared.state);
            let roots = scan::project_roots(state.projects.iter().map(|p| p.path.clone()));
            (state.settings.local_history_days, roots)
        };
        self.days = days;
        let wanted = if days > 0 && Self::active(shared) {
            roots
        } else {
            BTreeSet::new()
        };
        if wanted == self.watched {
            return;
        }
        if wanted.is_empty() {
            self.watcher = None;
            self.watched.clear();
            self.pending.clear();
            return;
        }
        self.rewatch(wanted);
    }

    fn rewatch(&mut self, wanted: BTreeSet<PathBuf>) {
        use notify::Watcher;
        if self.watcher.is_none() {
            let tx = self.tx.clone();
            self.watcher =
                notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if let Ok(event) = event
                        && scan::content_event(&event.kind)
                    {
                        let _ = tx.try_send(LocalHistoryJob::Changed(event.paths));
                    }
                })
                .ok();
        }
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        for gone in self.watched.difference(&wanted) {
            let _ = watcher.unwatch(gone);
        }
        for added in wanted.difference(&self.watched) {
            // A root that cannot be watched (gone, inotify limit) stays in
            // the set so it is not retried every few seconds.
            let _ = watcher.watch(added, notify::RecursiveMode::Recursive);
        }
        self.watched = wanted;
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
        scan::record_batch(&self.store, &self.watched, ready);
    }
}
