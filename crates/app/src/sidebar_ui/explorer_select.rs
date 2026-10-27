//! Explorer selection, keyboard, clipboard, and drag and drop (`IntelliJ`
//! Project view). The tree owns a selection independent of the focused
//! editor: arrows walk it, Shift and Cmd/Ctrl extend it, and file
//! operations (rename, delete, copy, cut, paste, drag) act on it.

use crate::file_actions::FileAction;
use crate::sidebar_cache::ExplorerRow;
use crate::{App, Tab, workspace_ops};
use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Type-to-search resets after this much idle time, like `IntelliJ`.
const SPEED_SEARCH_IDLE: Duration = Duration::from_millis(1500);

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

    /// Keyboard for a focused tree. Arrows move the selection (Shift
    /// extends), Left/Right collapse, expand, and walk the hierarchy,
    /// Enter opens, F2 renames, Delete trashes, Cmd/Ctrl+C/X/V copy, cut,
    /// and paste files, Escape clears, and typed text searches the
    /// visible rows.
    pub(super) fn explorer_keys(
        &mut self,
        ui: &mut egui::Ui,
        rows: &[ExplorerRow],
        host: egui::Id,
        root: &Path,
    ) {
        let focus = tree_focus_id(host);
        if !ui.memory(|memory| memory.has_focus(focus)) || self.name_prompt.is_some() {
            return;
        }
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                focus,
                egui::EventFilter {
                    tab: false,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
        let mut keys: Vec<(egui::Key, egui::Modifiers)> = Vec::new();
        let mut typed = String::new();
        ui.input(|input| {
            for event in &input.events {
                match event {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => keys.push((*key, *modifiers)),
                    egui::Event::Text(text) => typed.push_str(text),
                    _ => {}
                }
            }
        });
        let visible = entry_rows(rows);
        let cursor = self.explorer_selection.last().cloned();
        let at = cursor
            .as_ref()
            .and_then(|cursor| visible.iter().position(|row| &row.path == cursor));
        let mut consumed: Vec<(egui::Key, egui::Modifiers)> = Vec::new();
        for (key, modifiers) in keys {
            let handled = match key {
                egui::Key::ArrowDown | egui::Key::ArrowUp if !modifiers.command => {
                    let next = match (at, key) {
                        (None, _) => Some(0),
                        (Some(i), egui::Key::ArrowDown) => {
                            Some(i.saturating_add(1).min(visible.len().saturating_sub(1)))
                        }
                        (Some(i), _) => Some(i.saturating_sub(1)),
                    };
                    if let Some(row) = next.and_then(|next| visible.get(next)) {
                        let path = row.path.clone();
                        if modifiers.shift
                            && let Some(anchor) = self.explorer_anchor.clone()
                        {
                            self.explorer_select_range(rows, &anchor, &path);
                            self.explorer_scroll_to = Some(path);
                        } else {
                            self.explorer_select_one(path);
                        }
                    }
                    true
                }
                egui::Key::ArrowLeft => {
                    if let Some(row) = at.and_then(|i| visible.get(i)) {
                        if is_dir(row) && self.expanded_dirs.contains(&row.path) {
                            self.expanded_dirs.remove(&row.path);
                        } else if let Some(parent) = row.path.parent()
                            && visible.iter().any(|row| row.path == parent)
                        {
                            self.explorer_select_one(parent.to_owned());
                        }
                    }
                    true
                }
                egui::Key::ArrowRight => {
                    if let Some(row) = at.and_then(|i| visible.get(i))
                        && is_dir(row)
                    {
                        if !self.expanded_dirs.contains(&row.path) {
                            self.expanded_dirs.insert(row.path.clone());
                        } else if let Some(child) = at
                            .and_then(|i| visible.get(i.saturating_add(1)))
                            .filter(|child| child.path.parent() == Some(row.path.as_path()))
                        {
                            self.explorer_select_one(child.path.clone());
                        }
                    }
                    true
                }
                egui::Key::Enter => {
                    if let Some(row) = at.and_then(|i| visible.get(i)) {
                        if is_dir(row) {
                            if !self.expanded_dirs.remove(&row.path) {
                                self.expanded_dirs.insert(row.path.clone());
                            }
                        } else {
                            let path = row.path.clone();
                            self.activate_file_action(ui, &path, FileAction::Open);
                        }
                    }
                    true
                }
                egui::Key::F2 | egui::Key::F6 if key == egui::Key::F2 || modifiers.shift => {
                    if let Some(path) = cursor.clone() {
                        self.explorer_rename(&path);
                    }
                    true
                }
                egui::Key::Delete => {
                    self.explorer_confirm_delete();
                    true
                }
                egui::Key::Backspace if modifiers.command => {
                    self.explorer_confirm_delete();
                    true
                }
                egui::Key::C | egui::Key::X if modifiers.command && !modifiers.alt => {
                    if !self.explorer_selection.is_empty() {
                        let paths = self.explorer_selection.clone();
                        ui.ctx().copy_text(
                            paths
                                .iter()
                                .map(|path| path.display().to_string())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        );
                        self.explorer_clipboard = Some((paths, key == egui::Key::X));
                    }
                    true
                }
                egui::Key::V if modifiers.command && !modifiers.alt => {
                    self.explorer_paste(root);
                    true
                }
                egui::Key::Escape => {
                    if self.explorer_speed.is_empty() {
                        self.explorer_selection.clear();
                    } else {
                        self.explorer_speed.clear();
                    }
                    true
                }
                egui::Key::Backspace if !self.explorer_speed.is_empty() => {
                    self.explorer_speed.pop();
                    true
                }
                _ => false,
            };
            if handled {
                consumed.push((key, modifiers));
            }
        }
        let typed: String = typed.chars().filter(|c| !c.is_control()).collect();
        if !typed.is_empty() {
            if self.explorer_speed_at.elapsed() > SPEED_SEARCH_IDLE {
                self.explorer_speed.clear();
            }
            self.explorer_speed.push_str(&typed);
            self.explorer_speed_at = Instant::now();
            self.explorer_speed_jump(&visible, at);
            ui.input_mut(|input| {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Text(_)));
            });
        }
        ui.input_mut(|input| {
            for (key, modifiers) in consumed {
                input.consume_key(modifiers, key);
            }
        });
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
mod tests {
    use super::*;

    #[test]
    fn moved_maps_the_path_and_everything_under_it() {
        let (from, to) = (Path::new("/r/a"), Path::new("/r/b"));
        assert_eq!(
            moved(Path::new("/r/a"), from, to),
            Some(PathBuf::from("/r/b"))
        );
        assert_eq!(
            moved(Path::new("/r/a/x.rs"), from, to),
            Some(PathBuf::from("/r/b/x.rs"))
        );
        assert_eq!(moved(Path::new("/r/ab"), from, to), None);
    }
}
