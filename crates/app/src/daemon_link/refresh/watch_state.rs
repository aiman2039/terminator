//! The refresh worker's filesystem watcher and the paths it watches.
use notify::{RecursiveMode, Watcher};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

pub(super) type Watches = Vec<(PathBuf, RecursiveMode)>;
pub(super) type WatcherFactory = Box<dyn Fn() -> Option<notify::RecommendedWatcher> + Send>;

/// The watcher and the paths it really watches, under one lock. `rewatch`
/// updates both on the fs pool, so a refresh cancelled after it ran cannot
/// leave watched paths that no later pass unwatches.
pub(super) struct WatchState {
    /// Builds a fresh watcher with the same event handler.
    make: WatcherFactory,
    pub(super) watcher: Option<notify::RecommendedWatcher>,
    pub(super) watched: Watches,
    /// Identity of each watched path when it was added. inotify
    /// drops the watch of a deleted root, so a recreated or retargeted
    /// path with the same name must be watched again.
    identities: Vec<Option<(u64, u64)>>,
    /// A watch failed; polling continues until the targets change.
    failed: bool,
}

impl WatchState {
    pub(super) fn new(make: WatcherFactory) -> Self {
        Self {
            watcher: make(),
            make,
            watched: Vec::new(),
            identities: Vec::new(),
            failed: false,
        }
    }
}

/// Swaps watched paths when the targets or their identities changed and
/// records them in the shared state. Returns whether watching fell back to
/// polling; a failed watch keeps polling until a later swap succeeds.
pub(super) fn rewatch(state: &Mutex<WatchState>, targets: Watches) -> bool {
    let identities: Vec<_> = targets.iter().map(|(path, _)| identity(path)).collect();
    let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
    let WatchState {
        make,
        watcher,
        watched,
        identities: current,
        failed,
    } = &mut *state;
    if watcher.is_none() {
        return true;
    }
    if *watched == targets && *current == identities {
        return *failed;
    }
    // notify's inotify backend keeps the stale descriptor of a deleted
    // path, and its late delete event drops the new watch of a recreated
    // path. A fresh watcher has no stale state.
    let replaced = watched.iter().zip(current.iter()).any(|((path, _), old)| {
        targets
            .iter()
            .zip(&identities)
            .any(|((target, _), new)| target == path && new != old)
    });
    if replaced && let Some(fresh) = make() {
        *watcher = Some(fresh);
        watched.clear();
    }
    let Some(watcher) = watcher.as_mut() else {
        return true;
    };
    for (path, _) in watched.drain(..) {
        let _ = watcher.unwatch(&path);
    }
    *failed = false;
    for (path, mode) in &targets {
        if watcher.watch(path, *mode).is_err() {
            *failed = true;
        }
    }
    *watched = targets;
    *current = identities;
    *failed
}

/// Device and inode on Unix; creation time elsewhere.
fn identity(path: &Path) -> Option<(u64, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let created = metadata
            .created()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        Some((created.as_secs(), u64::from(created.subsec_nanos())))
    }
}
