//! One refresh coordinator: coalesced invalidations and no overlapping Git commands.
#[cfg(test)]
mod harness;
mod live;
#[cfg(test)]
mod tests;

#[cfg(test)]
use harness::spawn;
pub use live::spawn_async;

use crate::services;
use notify::RecursiveMode;
use std::path::PathBuf;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub cwd: PathBuf,
    pub generation: u64,
    pub directories: Vec<PathBuf>,
    /// The Git view needs per-file untracked status, line counts, and a full watch.
    pub include_stats: bool,
}

fn watch_targets(
    request: &Request,
    context: &services::ContextData,
) -> Vec<(PathBuf, RecursiveMode)> {
    let mut targets = std::collections::BTreeMap::new();
    if request.include_stats {
        targets.insert(
            normalize(context.root.as_ref().unwrap_or(&request.cwd)),
            RecursiveMode::Recursive,
        );
    } else {
        for path in &request.directories {
            targets.insert(normalize(path), RecursiveMode::NonRecursive);
        }
    }
    for path in &context.git_dirs {
        targets.insert(normalize(path), RecursiveMode::Recursive);
    }
    let all: Vec<_> = targets.into_iter().collect();
    all.iter()
        .filter(|(path, _)| {
            !all.iter().any(|(parent, mode)| {
                parent != path && *mode == RecursiveMode::Recursive && path.starts_with(parent)
            })
        })
        .cloned()
        .collect()
}

const MAX_DIRTY_PATHS: usize = 1024;

/// Paths the watcher reported since the last finished refresh. `full`
/// stands for "too many to track" (or a rescan request); `epoch` counts
/// events, so a refresh can tell whether more arrived while it ran.
#[derive(Clone, Default)]
struct Dirty {
    paths: Vec<PathBuf>,
    full: bool,
    epoch: u64,
}

impl Dirty {
    fn record(&mut self, event: &notify::Result<notify::Event>) {
        self.epoch = self.epoch.wrapping_add(1);
        match event {
            Ok(event) if event.need_rescan() => {
                self.paths.clear();
                self.full = true;
            }
            Ok(event) => accumulate_dirty(&mut self.paths, event.paths.clone(), &mut self.full),
            Err(_) => {
                self.paths.clear();
                self.full = true;
            }
        }
    }

    /// Forget what a finished refresh (`seen`) covered. Events that arrived
    /// meanwhile stay; when that cannot be told apart, everything stays.
    fn settle(&mut self, seen: &Self) {
        if self.epoch == seen.epoch {
            *self = Self {
                epoch: self.epoch,
                ..Self::default()
            };
        } else if !seen.full && !self.full {
            self.paths.drain(..seen.paths.len().min(self.paths.len()));
        }
    }
}

/// Why a refresh runs: only watch events and newly shown folders can
/// re-list part of the tree; forced refreshes and polls re-list it all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wake {
    Request,
    Watch,
    Timer,
}

/// The visible directories a refresh lists again: every one when `full`,
/// else those not listed yet and those a changed path is in (or under a
/// changed path, for moved folders). `dirty` holds normalized paths.
fn directories_to_list(
    directories: &[PathBuf],
    listed: &dyn Fn(&std::path::Path) -> bool,
    dirty: &[PathBuf],
    full: bool,
) -> Vec<PathBuf> {
    directories
        .iter()
        .filter(|path| {
            if full || !listed(path) {
                return true;
            }
            let canonical = normalize(path);
            dirty.iter().any(|changed| {
                changed.parent() == Some(canonical.as_path()) || canonical.starts_with(changed)
            })
        })
        .cloned()
        .collect()
}

/// A change to ignore rules or the index can flip any entry's ignored
/// state, so it re-lists everything.
fn ignore_rules_changed(dirty: &[PathBuf]) -> bool {
    dirty.iter().any(|path| {
        path.file_name()
            .is_some_and(|name| name == ".gitignore" || name == "exclude" || name == "index")
    })
}

fn accumulate_dirty(paths: &mut Vec<PathBuf>, incoming: Vec<PathBuf>, full: &mut bool) {
    if *full || paths.len().saturating_add(incoming.len()) > MAX_DIRTY_PATHS {
        paths.clear();
        *full = true;
        return;
    }
    paths.extend(incoming.into_iter().map(|path| normalize(&path)));
}

fn normalize(path: &std::path::Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        return normalize(parent).join(name);
    }
    path.into()
}
