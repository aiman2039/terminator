//! GUI-owned native file editing backing Settings → Editor mode → Native.
//!
//! Unlike daemon editors, native tabs hold their buffer in the GUI: no PTY,
//! no Neovim session. Files load and save on the filesystem worker pool with
//! the same 1 MiB bound as Markdown previews. Saves refuse to overwrite a
//! file that changed on disk unless forced, Reload is disabled while dirty,
//! and closing a dirty tab asks first — nothing is silently discarded.
mod actions;
mod blame;
mod close;
mod close_copies;
mod complete;
mod doc;
mod file_io;
mod find;
mod find_files;
mod gutter;
mod hierarchy;
mod jumps;
mod navigation;
mod rename;
mod view;
pub(crate) use actions::ActionsPopup;
pub(crate) use blame::EditorBlame;
use close::CloseIssuerInput;
pub(super) use close::{CloseIssuer, NativeCloseChoice};
use close_copies::{RemoveCopyFromFloatDockInput, RemoveCopyFromTabInput};
pub(crate) use complete::{CompletionPopup, SignaturePopup};
pub(crate) use doc::NativeDoc;
use doc::*;
pub(crate) use find::FindBar;
pub(crate) use find_files::FindInFiles;
pub(crate) use gutter::GutterPopup;
pub(crate) use hierarchy::HierarchyView;
pub(crate) use navigation::{NavLocation, UsagesView, read_line_previews};
pub(crate) use rename::RenameDialog;

use super::workspace::WorkspaceTab;
use super::*;
use crate::app::OpenFileInput;
use crate::lsp_manager::SubmitTextInput;
use crate::markdown::ShowSyncedInput;
use crate::remote_ui::OpenRemoteFileInput;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use terminator_native_edit::blocks::{BlockKind, parse_blocks};
use terminator_native_edit::doc::{Buffer, Doc};
use terminator_native_edit::highlight::{Language, language_for_path};
use terminator_native_edit::lsp as native_lsp;
use terminator_native_edit::view::{
    DiagnosticMark, ShowSourceInput, SourceOptions, mode_color, mode_name, show_source,
    source_focus_id,
};

pub(crate) struct OpenNativeInput<'a> {
    pub(crate) project: &'a str,
    pub(crate) path: PathBuf,
    pub(crate) line: Option<u32>,
    pub(crate) split: Option<&'a str>,
}

pub(crate) struct NativeEditorViewInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) path: &'a Path,
    pub(crate) project: Option<&'a str>,
    pub(crate) tab: Option<&'a str>,
    pub(crate) node: Option<egui_dock::NodePath>,
}

use terminator_native_edit::vim::{Effect, ModalEngine, VimEngine};

enum PostEdit {
    Keep,
    /// Close every view of the file. `force` (`:q!`) also takes floating
    /// windows; a plain `:q` leaves them (and their buffer) alone.
    Close {
        force: bool,
    },
}

/// Deferred `:split` / `:vsplit` / `:tabnew`: which layout change a drained
/// engine effect asks for. The engine never touches layout; the GUI maps
/// these onto its own tabs, preserving the originating project.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum NativeSplit {
    /// `:split`: same file in a pane below the current view.
    Below,
    /// `:vsplit`: same file in a pane beside the current view.
    Beside,
    /// `:tabnew`: same file in a new top-level tab.
    NewTab,
}

impl App {
    fn native_doc(&mut self, path: &Path, vim: bool) -> &mut NativeDoc {
        let remote_snapshot = crate::remote_ui::is_snapshot(&self.paths.data, path);
        self.native_docs.entry(path.to_owned()).or_insert_with(|| {
            let mut doc = NativeDoc::new(path.to_owned(), !vim);
            if remote_snapshot {
                doc.read_limit = terminator_core::remote::MAX_FILE_BYTES as u64;
            }
            doc
        })
    }

    pub(super) fn native_dirty(&self, path: &Path) -> bool {
        self.native_docs.get(path).is_some_and(NativeDoc::dirty)
    }

    /// Dirty native buffers inside one workspace tab, for close guards.
    /// Dock-level closes go through `on_close`; data-level closes (workspace
    /// close, quit) must check this first because they bypass that hook.
    pub(super) fn dirty_native_in_tab(&self, project: &str, tab_id: &str) -> Vec<PathBuf> {
        self.layouts
            .get(project)
            .and_then(|workspace| workspace.tabs.iter().find(|tab| tab.id == tab_id))
            .map(|tab| {
                tab.layout
                    .iter_all_tabs()
                    .filter_map(|(_, tab)| match tab {
                        Tab::NativeEditor { path } if self.native_dirty(path) => Some(path.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Drop buffers (and pending lines/prompts) for paths with no tab left.
    /// Data-level tab removal bypasses `on_close`, so call it after those.
    /// Floating windows hold their panes outside the docks: a floated
    /// editor keeps its buffer even when no workspace tab shows it, or
    /// closing an unrelated tab would discard unsaved changes.
    pub(super) fn prune_native_docs(&mut self) {
        let mut live = std::collections::HashSet::new();
        for workspace in self.layouts.values() {
            for tab in &workspace.tabs {
                for (_, tab) in tab.layout.iter_all_tabs() {
                    if let Tab::NativeEditor { path } = tab {
                        live.insert(path.clone());
                    }
                }
            }
        }
        for window in &self.floating {
            let Some(dock) = window.dock.as_ref() else {
                continue;
            };
            for (_, tab) in dock.iter_all_tabs() {
                if let Tab::NativeEditor { path } = tab {
                    live.insert(path.clone());
                }
            }
        }
        self.native_docs.retain(|path, _| live.contains(path));
        self.native_pending_line
            .retain(|path, _| live.contains(path));
        self.native_pending_col
            .retain(|path, _| live.contains(path));
        self.lsp.prune(&live);
        if self
            .native_close_prompt
            .as_ref()
            .is_some_and(|(path, _)| !live.contains(path))
        {
            self.native_close_prompt = None;
        }
        if self
            .native_close_after_save
            .as_ref()
            .is_some_and(|path| !live.contains(path))
        {
            self.native_close_after_save = None;
            self.native_close_after_save_issuer = None;
        }
    }

    /// Every path with a live native editor tab, de-duplicated. Floating
    /// windows count: closing the docked copy of a floated file must
    /// retain its buffer.
    fn all_native_tab_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for workspace in self.layouts.values() {
            for tab in &workspace.tabs {
                for (_, tab) in tab.layout.iter_all_tabs() {
                    if let Tab::NativeEditor { path } = tab
                        && !paths.contains(path)
                    {
                        paths.push(path.clone());
                    }
                }
            }
        }
        for window in &self.floating {
            let Some(dock) = window.dock.as_ref() else {
                continue;
            };
            for (_, tab) in dock.iter_all_tabs() {
                if let Tab::NativeEditor { path } = tab
                    && !paths.contains(path)
                {
                    paths.push(path.clone());
                }
            }
        }
        paths
    }

    /// Open `path` at a language-server position: the line goes through
    /// [`App::open_file`], the UTF-16 column through the native pending
    /// machinery so it converts once the buffer has loaded.
    pub(crate) fn open_file_at(&mut self, path: PathBuf, position: native_lsp::Position) {
        self.record_jump_origin();
        let absolute = std::path::absolute(&path).unwrap_or_else(|_| path.clone());
        self.open_file(OpenFileInput {
            path,
            line: position.line.checked_add(1),
            split: None,
            external: false,
        });
        if self.native_pending_line.contains_key(&absolute) {
            self.native_pending_col.insert(absolute, position.character);
        }
    }

    pub(super) fn open_native(&mut self, input: OpenNativeInput<'_>) {
        let OpenNativeInput {
            project,
            path,
            line,
            split,
        } = input;
        self.hide_center_overlay();
        let origin = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()));
        let after = self.editor_target(project, origin.as_ref(), split);
        let path = std::path::absolute(&path).unwrap_or(path);
        self.preferences
            .recent_native_files
            .retain(|item| item != &path);
        self.preferences.recent_native_files.insert(0, path.clone());
        self.preferences.recent_native_files.truncate(30);
        if let Some(line) = line.filter(|line| *line > 0) {
            self.native_pending_line.insert(
                path.clone(),
                usize::try_from(line)
                    .unwrap_or(usize::MAX)
                    .saturating_sub(1),
            );
        }
        let _ = self
            .update_tx
            .send(Update::OpenNativeEditor(project.into(), path, after));
    }
}

#[cfg(test)]
mod view_buffer_tests;
