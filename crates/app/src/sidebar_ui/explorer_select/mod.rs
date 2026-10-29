//! Explorer selection, keyboard, clipboard, and drag and drop (`IntelliJ`
//! Project view). The tree owns a selection independent of the focused
//! editor: arrows walk it, Shift and Cmd/Ctrl extend it, and file
//! operations (rename, delete, copy, cut, paste, drag) act on it.

mod keys;

use crate::file_actions::FileAction;
use crate::sidebar_cache::ExplorerRow;
use crate::{App, Tab, workspace_ops};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(crate) struct ExplorerKeysInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) rows: &'a [ExplorerRow],
    pub(crate) host: egui::Id,
    pub(crate) root: &'a Path,
}

/// Type-to-search resets after this much idle time, like `IntelliJ`.
const SPEED_SEARCH_IDLE: Duration = Duration::from_millis(1500);
/// Most Explorer renames and moves Cmd/Ctrl+Z can undo.
const EXPLORER_UNDO_LIMIT: usize = 50;

/// Drag payload: the Explorer paths being dragged.
#[derive(Clone)]
pub(crate) struct ExplorerDrag(pub Vec<PathBuf>);

/// Stable keyboard-focus id for one hosted tree.
pub(super) fn tree_focus_id(host: egui::Id) -> egui::Id {
    egui::Id::unique(("explorer-tree", host))
}

/// Rows that stand for entries (status rows such as "Loading…" are skipped).
fn entry_rows(rows: &[ExplorerRow]) -> Vec<&ExplorerRow> {
    rows.iter().filter(|row| row.entry.is_some()).collect()
}

fn is_dir(row: &ExplorerRow) -> bool {
    row.entry.as_ref().is_some_and(|entry| entry.directory)
}

/// A folder, or a file with nested files: Left/Right close and open it.
fn expandable(row: &ExplorerRow) -> bool {
    is_dir(row) || row.nests
}

/// `path` moved from `from` to `to` (itself or anything under it).
fn moved(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    path.strip_prefix(from).ok().map(|rest| {
        if rest.as_os_str().is_empty() {
            to.to_path_buf()
        } else {
            to.join(rest)
        }
    })
}

impl App {
    /// Selection bookkeeping for a row click. Cmd/Ctrl toggles a row and
    /// Shift selects the visible range from the anchor; neither opens
    /// anything. A plain click selects the row alone and returns true so
    /// the caller runs its default action (open the file, toggle the
    /// folder).
    pub(crate) fn explorer_click(
        &mut self,
        rows: &[ExplorerRow],
        path: &Path,
        modifiers: egui::Modifiers,
    ) -> bool {
        if modifiers.command {
            if let Some(at) = self.explorer_selection.iter().position(|p| p == path) {
                self.explorer_selection.remove(at);
            } else {
                self.explorer_selection.push(path.to_owned());
            }
            self.explorer_anchor = Some(path.to_owned());
            return false;
        }
        if modifiers.shift
            && let Some(anchor) = self.explorer_anchor.clone()
        {
            self.explorer_select_range(rows, &anchor, path);
            return false;
        }
        self.explorer_selection = vec![path.to_owned()];
        self.explorer_anchor = Some(path.to_owned());
        true
    }

    fn explorer_select_range(&mut self, rows: &[ExplorerRow], anchor: &Path, to: &Path) {
        let rows = entry_rows(rows);
        let index = |path: &Path| rows.iter().position(|row| row.path == path);
        let (Some(a), Some(b)) = (index(anchor), index(to)) else {
            self.explorer_selection = vec![to.to_owned()];
            return;
        };
        let (lo, hi) = (a.min(b), a.max(b));
        self.explorer_selection = rows
            .get(lo..=hi)
            .unwrap_or_default()
            .iter()
            .map(|row| row.path.clone())
            .collect();
        // Keep the moving end last: it is the keyboard cursor.
        if b < a {
            self.explorer_selection.reverse();
        }
    }

    /// Whether the tree paints `path` as selected. With no selection yet,
    /// the focused editor's file stands in, as before the tree had one.
    pub(super) fn explorer_selected(&self, path: &Path, focused_file: Option<&Path>) -> bool {
        if self.explorer_selection.is_empty() {
            focused_file == Some(path)
        } else {
            self.explorer_selection.iter().any(|p| p == path)
        }
    }

    /// Folder that new items and pastes land in: the selected folder, the
    /// selected file's folder, else the tree root.
    pub(crate) fn explorer_target_dir(&self, root: &Path) -> PathBuf {
        let Some(cursor) = self.explorer_selection.last() else {
            return root.to_owned();
        };
        if !cursor.starts_with(root) {
            return root.to_owned();
        }
        if cursor.is_dir() {
            cursor.clone()
        } else {
            cursor
                .parent()
                .map_or_else(|| root.to_owned(), Path::to_path_buf)
        }
    }

    fn explorer_select_one(&mut self, path: PathBuf) {
        self.explorer_anchor = Some(path.clone());
        self.explorer_scroll_to = Some(path.clone());
        self.explorer_selection = vec![path];
    }

    /// Select the first visible row, from the cursor on, whose name
    /// contains the type-to-search text.
    fn explorer_speed_jump(&mut self, visible: &[&ExplorerRow], at: Option<usize>) {
        let needle = self.explorer_speed.to_lowercase();
        let start = at.unwrap_or(0);
        let order = visible.iter().skip(start).chain(visible.iter().take(start));
        if let Some(row) = order
            .into_iter()
            .find(|row| row.label.to_lowercase().contains(&needle))
        {
            let path = row.path.clone();
            self.explorer_select_one(path);
        }
    }

    /// Type-to-search text while it is live, for the tree overlay.
    pub(super) fn explorer_speed_text(&self) -> Option<&str> {
        (!self.explorer_speed.is_empty() && self.explorer_speed_at.elapsed() <= SPEED_SEARCH_IDLE)
            .then_some(self.explorer_speed.as_str())
    }

    /// A folder row is open when any folder of its compact chain is.
    pub(crate) fn explorer_row_expanded(&self, row: &ExplorerRow) -> bool {
        row.filtered_open
            || self.expanded_dirs.contains(&row.path)
            || row
                .chain
                .iter()
                .any(|link| self.expanded_dirs.contains(link))
    }

    /// Open a folder row at its deepest folder, or close every folder of
    /// its chain (closing only one would leave the chain open).
    pub(crate) fn set_explorer_row_expanded(&mut self, row: &ExplorerRow, open: bool) {
        if open {
            self.expanded_dirs.insert(row.path.clone());
        } else {
            self.expanded_dirs.remove(&row.path);
            for link in &row.chain {
                self.expanded_dirs.remove(link);
            }
        }
    }

    pub(super) fn explorer_rename(&mut self, path: &Path) {
        self.open_name_prompt(workspace_ops::NamePrompt::Rename {
            from: path.to_owned(),
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        });
    }

    /// Ask before trashing the selection (one confirmation for all).
    pub(super) fn explorer_confirm_delete(&mut self) {
        let mut paths = self.explorer_selection.clone().into_iter();
        if let Some(first) = paths.next() {
            self.pending_delete = Some(first);
            self.pending_delete_more = paths.collect();
        }
    }

    /// Paste the Explorer clipboard into the target folder: cut moves (and
    /// empties the clipboard), copy duplicates.
    pub(super) fn explorer_paste(&mut self, root: &Path) {
        let Some((paths, cut)) = self.explorer_clipboard.clone() else {
            return;
        };
        let to_dir = self.explorer_target_dir(root);
        if cut {
            self.explorer_clipboard = None;
            self.queue_workspace(workspace_ops::Op::Move {
                from: paths,
                to_dir,
            });
        } else {
            self.queue_workspace(workspace_ops::Op::Copy {
                from: paths,
                to_dir,
            });
        }
    }

    /// Drop Explorer paths on a folder: move them there, unless a source
    /// already lives in it.
    pub(super) fn explorer_drop(&mut self, paths: Vec<PathBuf>, to_dir: PathBuf) {
        let from: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| path.parent() != Some(to_dir.as_path()) && !to_dir.starts_with(path))
            .collect();
        if !from.is_empty() {
            self.queue_workspace(workspace_ops::Op::Move { from, to_dir });
        }
    }

    /// Remember a finished rename or move for undo, unless it is an undo.
    pub(crate) fn explorer_record_move(&mut self, mut moved: Vec<(PathBuf, PathBuf)>) {
        moved.retain(|pair| {
            let Some(at) = self.explorer_undoing.iter().position(|undo| undo == pair) else {
                return true;
            };
            self.explorer_undoing.swap_remove(at);
            false
        });
        if moved.is_empty() {
            return;
        }
        if self.explorer_undo.len() >= EXPLORER_UNDO_LIMIT {
            self.explorer_undo.remove(0);
        }
        self.explorer_undo.push(moved);
    }

    /// Undo the last Explorer rename or move by renaming each path back.
    pub(super) fn explorer_undo_last(&mut self) {
        let Some(moved) = self.explorer_undo.pop() else {
            self.info = Some("Nothing to undo in Explorer".into());
            return;
        };
        for (from, to) in moved.into_iter().rev() {
            self.explorer_undoing.push((to.clone(), from.clone()));
            self.queue_workspace(workspace_ops::Op::Rename { from: to, to: from });
        }
    }

    /// "Always select opened file": reveal the focused file once each time
    /// focus moves to a different file, unless a search owns the tree.
    pub(super) fn explorer_autoreveal(&mut self, host: egui::Id) {
        if !self.preferences.explorer_autoreveal || !self.explorer_query.is_empty() {
            return;
        }
        let focused = self.focused_local_file();
        if focused == self.explorer_followed {
            return;
        }
        self.explorer_followed.clone_from(&focused);
        if let Some(file) = focused
            && self.reveal_current_file_in_explorer(host)
        {
            self.explorer_selection = vec![file.clone()];
            self.explorer_anchor = Some(file);
        }
    }

    /// After a rename or move, point open tabs, editor buffers, and the
    /// Explorer at the new location, so nothing edits a vanished path.
    pub(crate) fn remap_moved_path(&mut self, from: &Path, to: &Path) {
        for workspace in self.layouts.values_mut() {
            for group in &mut workspace.tabs {
                let panes = group
                    .layout
                    .iter_all_tabs_mut()
                    .map(|(_, pane)| pane)
                    .chain(group.primary.as_mut());
                for pane in panes {
                    if let Tab::NativeEditor { path } | Tab::Image { path } = pane
                        && let Some(next) = moved(path, from, to)
                    {
                        *path = next;
                    }
                }
            }
        }
        let docs: Vec<PathBuf> = self
            .native_docs
            .keys()
            .filter(|path| path.starts_with(from))
            .cloned()
            .collect();
        for old in docs {
            if let (Some(mut doc), Some(next)) =
                (self.native_docs.remove(&old), moved(&old, from, to))
            {
                doc.path.clone_from(&next);
                self.native_docs.insert(next, doc);
            }
        }
        for path in &mut self.preferences.recent_native_files {
            if let Some(next) = moved(path, from, to) {
                *path = next;
            }
        }
        self.preferences.pinned_native_files =
            std::mem::take(&mut self.preferences.pinned_native_files)
                .into_iter()
                .map(|path| moved(&path, from, to).unwrap_or(path))
                .collect();
        for path in &mut self.explorer_selection {
            if let Some(next) = moved(path, from, to) {
                *path = next;
            }
        }
        let expanded: Vec<PathBuf> = self
            .expanded_dirs
            .iter()
            .filter(|path| path.starts_with(from))
            .cloned()
            .collect();
        for old in expanded {
            self.expanded_dirs.remove(&old);
            if let Some(next) = moved(&old, from, to) {
                self.expanded_dirs.insert(next);
            }
        }
    }
}

#[cfg(test)]
mod tests;
