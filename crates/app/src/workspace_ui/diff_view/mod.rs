//! Native diff pane: toolbar, document panes, and hunk requests.
use super::super::*;
use diff::actions::Kind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod body;
mod requests;
mod split;
mod toolbar;
mod view;

/// Per-frame values the diff pane steps read.
struct DiffFrame<'a> {
    tab: &'a Tab,
    cwd: &'a Path,
    path: &'a Path,
    staged: bool,
    key: String,
    header_path: PathBuf,
    doc: Option<Arc<diff::DiffDocument>>,
    remote: bool,
    read_only: bool,
    enabled: bool,
    count: usize,
    split: bool,
    focused: bool,
    is_md: bool,
    prev_file: Option<PathBuf>,
    next_file: Option<PathBuf>,
    file_pos: usize,
    file_total: usize,
}

/// The checked-out review state, and what this frame's input asked for.
/// Requests are sent after the pane paints.
#[derive(Default)]
struct DiffPass {
    review: diff::review::State,
    action: Option<(usize, Kind)>,
    undo: bool,
    navigate: Option<PathBuf>,
    open_working: bool,
    open_line: Option<u32>,
    context: Option<usize>,
    refresh: bool,
    collapse: bool,
    save_edits: bool,
    gap: Option<(usize, diff::GapPart)>,
    confirmed: Option<usize>,
}

impl DiffPass {
    /// Line to open for the selected change, or the top of the file.
    fn selected_change_line(&self, doc: Option<&Arc<diff::DiffDocument>>) -> Option<u32> {
        self.review
            .selected
            .and_then(|index| doc?.hunks.get(index))
            .and_then(|hunk| u32::try_from(hunk.new.start.saturating_add(1)).ok())
            .or(Some(1))
    }
}

/// Previous/next siblings of `current` in an ordered changed-file list.
/// Returns `(prev, next, position, total)`; position is 1-based, or 0 when
/// the current file is not in the list.
pub(super) fn diff_neighbors(
    paths: &[std::path::PathBuf],
    current: &std::path::Path,
) -> (
    Option<std::path::PathBuf>,
    Option<std::path::PathBuf>,
    usize,
    usize,
) {
    let total = paths.len();
    let Some(index) = paths
        .iter()
        .position(|path| path.components().eq(current.components()))
    else {
        return (None, None, 0, total);
    };
    (
        index.checked_sub(1).and_then(|i| paths.get(i).cloned()),
        paths.get(index.saturating_add(1)).cloned(),
        index.saturating_add(1),
        total,
    )
}

/// Per open diff key: its file's porcelain status and line counts.
pub(crate) type DiffFingerprints = std::collections::HashMap<String, Option<(String, (u32, u32))>>;

impl App {
    /// The view a refresh of `key` should rebuild: its context size (or
    /// `context` when the user picked a new one), the ranges already
    /// expanded, and the whitespace mode.
    pub(crate) fn diff_view_for(&self, key: &str, context: Option<usize>) -> diff::DiffView {
        let review = self.diff_reviews.get(key);
        diff::DiffView {
            context: context.unwrap_or_else(|| review.map_or(3, |review| review.context.max(3))),
            revealed: self
                .diffs
                .get(key)
                .and_then(|doc| doc.as_ref().ok())
                .map(|doc| doc.revealed.clone())
                .unwrap_or_default(),
            whitespace: review.is_some_and(|review| review.ignore_whitespace),
        }
    }

    /// Local diff tabs open anywhere in the layouts, one per key.
    fn open_local_diffs(&self) -> Vec<Tab> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for workspace in self.layouts.values() {
            for group in &workspace.tabs {
                let panes = group
                    .layout
                    .iter_all_tabs()
                    .map(|(_, pane)| pane)
                    .chain(group.primary.as_ref());
                for pane in panes {
                    if let Tab::Diff { remote: None, .. } = pane
                        && seen.insert(pane.key())
                    {
                        out.push(pane.clone());
                    }
                }
            }
        }
        out
    }

    /// What the Git status says about each open local diff's file: its
    /// porcelain status and line counts. A change means the diff is stale.
    pub(crate) fn diff_fingerprints(&self) -> DiffFingerprints {
        let context = self.context.as_ref();
        self.open_local_diffs()
            .into_iter()
            .filter_map(|tab| {
                let Tab::Diff { cwd, path, .. } = &tab else {
                    return None;
                };
                let absolute = if path.is_absolute() {
                    path.clone()
                } else {
                    cwd.join(path)
                };
                let print = context.and_then(|context| {
                    let change = context.changes.iter().find(|c| c.path == absolute)?;
                    let stats = context.stats.get(&absolute).copied().unwrap_or_default();
                    Some((change.status.clone(), stats))
                });
                Some((tab.key(), print))
            })
            .collect()
    }

    /// Re-request open diffs whose file changed since `before` (an agent
    /// edit, staging from the sidebar, a commit), keeping each one's view.
    /// Busy, loading, and unsaved-edit diffs are left alone.
    pub(crate) fn refresh_stale_diffs(&mut self, before: &DiffFingerprints) {
        let after = self.diff_fingerprints();
        for tab in self.open_local_diffs() {
            let key = tab.key();
            let stale = before.get(&key) != after.get(&key);
            let idle = self.diffs.get(&key).is_some_and(Result::is_ok)
                && !self.loading.contains(&key)
                && self
                    .diff_reviews
                    .get(&key)
                    .is_none_or(|review| !review.busy && !review.unsaved_edits());
            if stale && idle {
                let view = self.diff_view_for(&key, None);
                self.loading.insert(key);
                let _ = self.jobs.send(Job::DiffContext(tab, view));
            }
        }
    }

    /// Replace the current diff pane in place with a sibling file's diff,
    /// keeping the same layout slot (RustRover-style file navigation).
    pub(super) fn navigate_diff(&mut self, current: &Tab, target: std::path::PathBuf) {
        if self.block_unsaved_diff(Some(&current.key())) {
            return;
        }
        let Tab::Diff {
            cwd,
            staged,
            remote,
            ..
        } = current
        else {
            return;
        };
        let next = Tab::Diff {
            remote: remote.clone(),
            cwd: cwd.clone(),
            path: target,
            staged: *staged,
        };
        self.diff_replacement = Some((current.clone(), next));
    }

    /// Swap this staged (read-only) pane for the editable working-tree diff
    /// of the same file, in the same layout slot.
    pub(super) fn open_working_diff(&mut self, current: &Tab) {
        if self.block_unsaved_diff(Some(&current.key())) {
            return;
        }
        let Tab::Diff {
            cwd, path, remote, ..
        } = current
        else {
            return;
        };
        let next = Tab::Diff {
            remote: remote.clone(),
            cwd: cwd.clone(),
            path: path.clone(),
            staged: false,
        };
        self.diff_replacement = Some((current.clone(), next));
    }

    /// Apply navigation after the checked-out main or floating dock returns.
    pub(crate) fn apply_diff_replacement(&mut self) {
        let Some((current, next)) = self.diff_replacement.clone() else {
            return;
        };
        let mut replaced = false;
        if let Some(project) = self.selected.clone()
            && let Some(workspace) = self.layouts.get_mut(&project)
        {
            for group in &mut workspace.tabs {
                if group.primary.as_ref() == Some(&current) {
                    group.primary = Some(next.clone());
                    replaced = true;
                }
                for (_, pane) in group.layout.iter_all_tabs_mut() {
                    if pane == &current {
                        *pane = next.clone();
                        replaced = true;
                    }
                }
            }
        }
        for window in &mut self.floating {
            if let Some(dock) = &mut window.dock {
                for (_, pane) in dock.iter_all_tabs_mut() {
                    if pane == &current {
                        *pane = next.clone();
                        replaced = true;
                    }
                }
            }
        }
        if !replaced {
            return;
        }
        self.diff_replacement = None;
        self.focus_tab = Some(next.clone());
        self.loading.insert(next.key());
        if self.state.settings.diff_split_default {
            self.diff_split.insert(next.key());
        } else {
            self.diff_split.remove(&next.key());
        }
        let _ = self.jobs.send(Job::Diff(next));
    }
}
