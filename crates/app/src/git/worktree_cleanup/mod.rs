//! Worktree hand-back and cleanup on the Worktrees page: per task worktree
//! its branch, `+added −deleted` since the task start, a Merged mark, the
//! last commit and the size on disk (read off the GUI thread); Apply to
//! source (merge or cherry-pick into the main checkout) and Remove of
//! several worktrees, each after an explicit confirmation.
mod apply;
mod page;
mod probe;
#[cfg(test)]
mod tests;

use crate::merge::{Done, take_done};
use crate::workspace_ops::CommitRisk;
use crate::*;
pub(crate) use apply::{ApplyMode, ApplyRequest};
use probe::Probe;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The shortest time between two probes of one worktree.
const PROBE_REFRESH: std::time::Duration = std::time::Duration::from_secs(30);

/// A confirmation the page waits for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ask {
    Apply { project: String, mode: ApplyMode },
    Remove(Vec<String>),
}

/// The Worktrees page's state.
#[derive(Default)]
pub(crate) struct Cleanup {
    probes: HashMap<PathBuf, Probe>,
    asked: HashMap<PathBuf, Instant>,
    pending: Vec<(PathBuf, Done<Probe>)>,
    /// Rows ticked for Remove, by project id.
    pub(crate) selected: BTreeSet<String>,
    pub(crate) ask: Option<Ask>,
    /// The Apply to source in flight.
    applying: Option<Done<String>>,
}

impl Cleanup {
    pub(crate) fn probe(&self, path: &Path) -> Option<&Probe> {
        self.probes.get(path)
    }

    /// True when a probe of `path` may start now; it then counts as asked.
    fn take_due(&mut self, path: &Path, now: Instant) -> bool {
        let due = self
            .asked
            .get(path)
            .is_none_or(|asked| now.saturating_duration_since(*asked) >= PROBE_REFRESH);
        if due {
            self.asked.insert(path.to_owned(), now);
        }
        due
    }
}

impl App {
    /// Starts the probe of `path` when due.
    fn ask_cleanup_probe(&mut self, path: &Path) {
        if !self.git_extras.cleanup.take_due(path, Instant::now()) {
            return;
        }
        let work = path.to_path_buf();
        let rx = self.file_pool_job(format!("worktree-probe:{}", path.display()), move || {
            Ok(probe::probe(&work))
        });
        self.git_extras
            .cleanup
            .pending
            .push((path.to_path_buf(), rx));
    }

    /// Takes finished probes and a finished Apply to source.
    fn poll_cleanup(&mut self, ctx: &egui::Context) {
        let cleanup = &mut self.git_extras.cleanup;
        let mut done = Vec::new();
        cleanup
            .pending
            .retain_mut(|(path, rx)| match take_done(rx) {
                Some(result) => {
                    done.push((path.clone(), result));
                    false
                }
                None => true,
            });
        for (path, result) in done {
            if let Ok(probe) = result {
                cleanup.probes.insert(path, probe);
            }
        }
        if let Some(result) = cleanup.applying.as_mut().and_then(take_done) {
            cleanup.applying = None;
            // Every row may have changed: probe again, refresh the panel.
            cleanup.asked.clear();
            self.refresh_request = None;
            self.git_views.clear();
            match result {
                Ok(message) => self.info = Some(message),
                Err(error) => self.error.set(error),
            }
        }
        let cleanup = &self.git_extras.cleanup;
        if !cleanup.pending.is_empty() || cleanup.applying.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    /// Why Apply to source cannot start for `path` now, if it cannot.
    fn apply_blocker(&self, path: &Path) -> Option<&'static str> {
        let cleanup = &self.git_extras.cleanup;
        if cleanup.applying.is_some() {
            return Some("Wait for the running Apply to source");
        }
        let probe = cleanup.probe(path);
        let Some(source) = probe.and_then(|probe| probe.source.as_deref()) else {
            return Some("Reading the worktree…");
        };
        if source == path {
            return Some("This is the source checkout");
        }
        self.commit_wait(Some(source), CommitRisk::Head)
    }

    /// Runs the confirmed Apply to source of `project`'s worktree. A
    /// running commit in the source checkout refuses it.
    pub(crate) fn start_apply(&mut self, project: &str, mode: ApplyMode) {
        let Some(path) = self.managed_worktree(project).map(|w| w.path.clone()) else {
            return;
        };
        let Some(source) = self
            .git_extras
            .cleanup
            .probe(&path)
            .and_then(|probe| probe.source.clone())
        else {
            return;
        };
        if self.git_extras.cleanup.applying.is_some()
            || self.refuse_during_commit(Some(&source), CommitRisk::Head)
        {
            return;
        }
        let request = ApplyRequest {
            mode,
            worktree: path.clone(),
            source: source.clone(),
            base: self
                .preferences
                .task_bases
                .get(&path)
                .map(|base| base.commit.clone()),
        };
        let rx = self.file_pool_job(format!("worktree-apply:{}", source.display()), move || {
            apply::apply(&request)
        });
        self.git_extras.cleanup.applying = Some(rx);
    }

    /// Asks the daemon to remove each worktree. It still refuses a dirty,
    /// locked or live-session checkout; the branch is kept.
    pub(crate) fn remove_worktrees(&mut self, projects: &[String]) {
        for project in projects {
            self.send(Request::WorktreeRemove {
                project: project.clone(),
            });
        }
        self.git_extras.cleanup.selected.clear();
    }

    /// Forgets the task base of every removed worktree.
    pub(crate) fn prune_task_bases(&mut self) {
        let removed: Vec<&Path> = self
            .state
            .worktrees
            .iter()
            .filter(|worktree| worktree.removed)
            .map(|worktree| worktree.path.as_path())
            .collect();
        if removed.is_empty() {
            return;
        }
        self.preferences
            .task_bases
            .retain(|path, _| !removed.contains(&path.as_path()));
    }
}
