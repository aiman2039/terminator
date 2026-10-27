//! GUI-owned native file editing backing Settings → Editor mode → Native.
//!
//! Unlike daemon editors, native tabs hold their buffer in the GUI: no PTY,
//! no Neovim session. Files load and save on the filesystem worker pool with
//! the same 1 MiB bound as Markdown previews. Saves refuse to overwrite a
//! file that changed on disk unless forced, Reload is disabled while dirty,
//! and closing a dirty tab asks first — nothing is silently discarded.
mod rename;
pub(crate) use rename::RenameDialog;

use super::workspace::WorkspaceTab;
use super::*;
use anyhow::Context as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use terminator_native_edit::blocks::{BlockKind, parse_blocks};
use terminator_native_edit::doc::{Buffer, Doc};
use terminator_native_edit::highlight::{HighlightCache, Language, language_for_path};
use terminator_native_edit::lsp as native_lsp;
use terminator_native_edit::view::{
    DiagnosticMark, SourceOptions, mode_color, mode_name, show_source, source_focus_id,
};
use terminator_native_edit::vim::{
    Effect, Key as VimKey, ModalEngine, VimEngine, VimrcOptions, parse_vimrc,
};

const MAX_NATIVE_BYTES: u64 = 1024 * 1024;

pub struct LoadedFile {
    text: String,
    mtime: Option<SystemTime>,
}

fn read_native_file(path: &Path, limit: u64) -> anyhow::Result<LoadedFile> {
    let file = std::fs::File::open(path).context("Open file")?;
    anyhow::ensure!(file.metadata()?.is_file(), "Not a regular file");
    let mut bytes = Vec::new();
    use std::io::Read as _;
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .context("Read file")?;
    anyhow::ensure!(
        bytes.len() as u64 <= limit,
        "File exceeds the {} MiB viewer limit",
        limit / (1024 * 1024)
    );
    let text = String::from_utf8(bytes).context("File is not UTF-8")?;
    let mtime = disk_mtime(path);
    Ok(LoadedFile { text, mtime })
}

fn disk_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

/// Options from `~/.vimrc`, read once per process. Only `set`/`setlocal`
/// names the native engine implements are applied (wrap, ignorecase);
/// mappings, plugins, and unknown options are ignored, so a large vimrc
/// can never break the editor.
fn vimrc_options() -> VimrcOptions {
    static OPTIONS: std::sync::OnceLock<VimrcOptions> = std::sync::OnceLock::new();
    *OPTIONS.get_or_init(|| {
        let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
        else {
            return VimrcOptions::default();
        };
        std::fs::read_to_string(Path::new(&home).join(".vimrc"))
            .map(|text| parse_vimrc(&text))
            .unwrap_or_default()
    })
}

type LoadResult = anyhow::Result<LoadedFile>;
type SaveResult = anyhow::Result<Option<SystemTime>>;

/// How long a disk modification-time reading stays valid. The header
/// re-checks the file on every frame; stat-ing a network or cold volume
/// that often is wasted work, and a sub-second delay before a clean buffer
/// follows an external edit is invisible.
const MTIME_TTL: Duration = Duration::from_millis(900);

pub struct NativeDoc {
    pub path: PathBuf,
    doc: Option<Doc>,
    markdown_mode: markdown::Mode,
    engine: VimEngine,
    highlight: HighlightCache,
    insert_first: bool,
    /// Buffer revision that matches the bytes on disk: the dirty check
    /// compares revisions instead of whole texts, so typing costs O(1)
    /// per frame instead of one full-text copy per frame.
    saved_revision: u64,
    /// Revision of the text handed to the in-flight save. Edits typed
    /// while the write runs keep the buffer dirty when it lands.
    save_revision: u64,
    loaded_mtime: Option<SystemTime>,
    read_limit: u64,
    mtime_check: Option<(Instant, Option<SystemTime>)>,
    load_error: Option<String>,
    save_error: Option<String>,
    loading: bool,
    saving: bool,
    force_save: bool,
    load_rx: Option<tokio::sync::mpsc::UnboundedReceiver<LoadResult>>,
    save_rx: Option<tokio::sync::mpsc::UnboundedReceiver<SaveResult>>,
}

impl NativeDoc {
    pub(crate) fn new(path: PathBuf, insert_first: bool) -> Self {
        let mut engine = VimEngine::new();
        engine.apply_vimrc(vimrc_options());
        Self {
            path,
            doc: None,
            markdown_mode: markdown::Mode::Preview,
            engine,
            highlight: HighlightCache::default(),
            insert_first,
            saved_revision: 0,
            save_revision: 0,
            loaded_mtime: None,
            read_limit: MAX_NATIVE_BYTES,
            mtime_check: None,
            load_error: None,
            save_error: None,
            loading: false,
            saving: false,
            force_save: false,
            load_rx: None,
            save_rx: None,
        }
    }

    pub fn dirty(&self) -> bool {
        self.doc
            .as_ref()
            .is_some_and(|doc| doc.revision() != self.saved_revision)
    }

    /// Disk modification time, re-stat-ed at most every [`MTIME_TTL`].
    fn disk_mtime_cached(&mut self) -> Option<SystemTime> {
        if let Some((checked, mtime)) = self.mtime_check
            && checked.elapsed() < MTIME_TTL
        {
            return mtime;
        }
        let mtime = disk_mtime(&self.path);
        self.mtime_check = Some((Instant::now(), mtime));
        mtime
    }

    fn ensure_loading(&mut self, services: &gui_services::Services) {
        if self.doc.is_some() || self.load_error.is_some() || self.loading {
            return;
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.load_rx = Some(rx);
        self.loading = true;
        let path = self.path.clone();
        let limit = self.read_limit;
        let service = services.clone();
        let context = OperationContext::new(
            "native-load",
            path.to_string_lossy().into(),
            Policy::ReplaceableRead,
        );
        if services
            .handle()
            .submit(context, CancellationToken::new(), async move {
                let token = CancellationToken::new();
                let result = service
                    .fs()
                    .run(&token, move || read_native_file(&path, limit))
                    .await;
                let _ = tx.send(result);
                Ok(Vec::new())
            })
            .is_err()
        {
            self.loading = false;
            self.load_error = Some("Could not start the file read".into());
        }
    }

    fn poll(&mut self) {
        if let Some(rx) = self.load_rx.as_mut()
            && let Ok(result) = rx.try_recv()
        {
            self.loading = false;
            match result {
                Ok(loaded) => {
                    self.loaded_mtime = loaded.mtime;
                    self.mtime_check = None;
                    self.doc = Some(Doc::new(loaded.text));
                    self.saved_revision = self.doc.as_ref().map_or(0, Doc::revision);
                    self.save_revision = self.saved_revision;
                    if self.insert_first {
                        self.insert_first = false;
                        if let Some(doc) = self.doc.as_mut() {
                            self.engine.press_key(doc, VimKey::Char('i'));
                        }
                    }
                    self.load_error = None;
                }
                Err(error) => {
                    self.load_error = Some(format!("{error:#}"));
                }
            }
        }
        if let Some(rx) = self.save_rx.as_mut()
            && let Ok(result) = rx.try_recv()
        {
            self.saving = false;
            match result {
                Ok(mtime) => {
                    // The written text was the snapshot taken at save start:
                    // edits typed while the write ran keep the buffer dirty.
                    self.saved_revision = self.save_revision;
                    self.loaded_mtime = mtime;
                    self.mtime_check = None;
                    self.save_error = None;
                    self.force_save = false;
                }
                Err(error) => {
                    self.save_error = Some(format!("{error:#}"));
                }
            }
        }
    }

    fn start_save(&mut self, services: &gui_services::Services) {
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        if self.saving {
            return;
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.save_rx = Some(rx);
        self.saving = true;
        self.save_error = None;
        let path = self.path.clone();
        let text = doc.text();
        self.save_revision = doc.revision();
        let expected = self.loaded_mtime;
        let force = self.force_save;
        let service = services.clone();
        let context = OperationContext::new(
            "native-save",
            path.to_string_lossy().into(),
            Policy::ReplaceableRead,
        );
        if services
            .handle()
            .submit(context, CancellationToken::new(), async move {
                let token = CancellationToken::new();
                let result = service
                    .fs()
                    .run(&token, move || {
                        if !force && disk_mtime(&path) != expected {
                            anyhow::bail!("Changed on disk since loading");
                        }
                        anyhow::ensure!(
                            text.len() as u64 <= MAX_NATIVE_BYTES,
                            "Native editing is limited to 1 MiB"
                        );
                        std::fs::write(&path, &text).context("Write file")?;
                        Ok(disk_mtime(&path))
                    })
                    .await;
                let _ = tx.send(result);
                Ok(Vec::new())
            })
            .is_err()
        {
            self.saving = false;
            self.save_error = Some("Could not start the file write".into());
        }
    }
}

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
/// View that asked for a deferred native close, captured when the
/// command starts. Plain closes remove only this view; every other
/// copy of the file stays open.
#[derive(Clone, Debug)]
pub(super) enum CloseIssuer {
    /// A docked pane: project, top-level tab, and leaf recorded up
    /// front, so a later focus move or a same-file twin tab cannot
    /// redirect the close. `delayed` marks a save-deferred `:wq`: the
    /// issuing view may be gone by the time the save settles, and then
    /// the close must take nothing instead of another copy.
    Docked {
        project: String,
        tab: String,
        node: Option<egui_dock::NodePath>,
        delayed: bool,
    },
    /// A pane docked in a floating window: the window plus the leaf
    /// recorded up front. Plain closes remove only the issuing copy;
    /// `delayed` marks a save-deferred `:wq` (exact pane or no-op).
    Floating {
        viewport: egui::ViewportId,
        node: Option<egui_dock::NodePath>,
        delayed: bool,
    },
}

/// Dirty-editor dialog decision for one path, applied by
/// [`App::apply_native_close_choice`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeCloseChoice {
    Discard,
    SaveClose,
}

impl CloseIssuer {
    /// Mark an issuer as save-deferred (`:wq` queued at save start,
    /// closing frames later): exact issuing pane or no-op.
    fn deferred(mut self) -> Self {
        match &mut self {
            Self::Docked { delayed, .. } | Self::Floating { delayed, .. } => {
                *delayed = true;
            }
        }
        self
    }
}

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

    /// Issuing view for a close requested from a file view: the floating
    /// window it renders in, else the docked project/tab/leaf under
    /// render. Captured now (not at drain time) so later focus moves
    /// cannot redirect the close.
    fn close_issuer(
        ui: &egui::Ui,
        project: Option<&str>,
        tab: Option<&str>,
        node: Option<egui_dock::NodePath>,
    ) -> Option<CloseIssuer> {
        let viewport = ui.ctx().viewport_id();
        if viewport != egui::ViewportId::ROOT {
            Some(CloseIssuer::Floating {
                viewport,
                node,
                delayed: false,
            })
        } else {
            match (project, tab) {
                (Some(project), Some(tab)) => Some(CloseIssuer::Docked {
                    project: project.to_owned(),
                    tab: tab.to_owned(),
                    node,
                    delayed: false,
                }),
                _ => None,
            }
        }
    }

    /// Remove one `target` copy from one top-level tab: the copy in
    /// `node` (the issuing pane recorded when the command started), then
    /// the focused leaf, then the first match. With `strict` (a delayed
    /// `:wq` naming its pane), only the recorded pane goes: a stale node
    /// (issuing view already closed) is a no-op, never another copy.
    /// Fixes `primary` afterwards. Returns whether a copy was removed.
    fn remove_copy_from_tab(
        tab_state: &mut WorkspaceTab,
        target: &Tab,
        node: Option<egui_dock::NodePath>,
        strict: bool,
    ) -> bool {
        let same_leaf = |at: &egui_dock::TabPath, leaf: egui_dock::NodePath| {
            at.surface == leaf.surface && at.node == leaf.node
        };
        let pick = {
            let layout = &tab_state.layout;
            layout
                .iter_all_tabs()
                .filter(|(_, tab)| *tab == target)
                .map(|(at, _)| at)
                .find(|at| node.is_some_and(|stored| same_leaf(at, stored)))
                .or_else(|| {
                    (!strict)
                        .then(|| {
                            layout
                                .iter_all_tabs()
                                .filter(|(_, tab)| *tab == target)
                                .map(|(at, _)| at)
                                .find(|at| {
                                    layout
                                        .focused_leaf()
                                        .is_some_and(|focus| same_leaf(at, focus))
                                })
                        })
                        .flatten()
                })
                .or_else(|| (!strict).then(|| layout.find_tab(target)).flatten())
        };
        let Some(found) = pick else {
            return false;
        };
        tab_state.layout.remove_tab(found);
        if tab_state.primary.as_ref() == Some(target) {
            tab_state.primary = tab_state
                .layout
                .iter_all_tabs()
                .next()
                .map(|(_, tab)| tab.clone());
        }
        true
    }

    /// Remove one docked copy of `path` from `workspace`: the focused
    /// copy first, then the first match. Returns whether a copy was
    /// there.
    fn remove_one_docked_copy(workspace: &mut Workspace, path: &Path) -> bool {
        let target = Tab::NativeEditor {
            path: path.to_owned(),
        };
        let mut removed = false;
        for tab in &mut workspace.tabs {
            if Self::remove_copy_from_tab(tab, &target, None, false) {
                removed = true;
                break;
            }
        }
        // Restores the non-empty invariant (`:qa` can empty a
        // workspace); a bare retain panics the next dock access.
        workspace.drop_empty_tabs();
        removed
    }

    /// Remove one `target` copy from a floating window's dock: the copy
    /// in `node` (the issuing pane recorded when the command started),
    /// then the window's focused copy, then the first match. `strict`
    /// mirrors [`Self::remove_copy_from_tab`]: a delayed `:wq` whose
    /// issuing view is gone closes nothing. Returns whether a copy was
    /// removed.
    fn remove_copy_from_float_dock(
        dock: &mut egui_dock::DockState<Tab>,
        target: &Tab,
        node: Option<egui_dock::NodePath>,
        strict: bool,
    ) -> bool {
        let same_leaf = |at: &egui_dock::TabPath, leaf: egui_dock::NodePath| {
            at.surface == leaf.surface && at.node == leaf.node
        };
        let pick = dock
            .iter_all_tabs()
            .filter(|(_, tab)| *tab == target)
            .map(|(at, _)| at)
            .find(|at| node.is_some_and(|stored| same_leaf(at, stored)))
            .or_else(|| {
                (!strict)
                    .then(|| {
                        dock.iter_all_tabs()
                            .filter(|(_, tab)| *tab == target)
                            .map(|(at, _)| at)
                            .find(|at| {
                                dock.main_surface().focused_leaf().is_some_and(|focus| {
                                    same_leaf(
                                        at,
                                        egui_dock::NodePath {
                                            surface: egui_dock::SurfaceIndex::main(),
                                            node: focus,
                                        },
                                    )
                                })
                            })
                    })
                    .flatten()
            })
            .or_else(|| (!strict).then(|| dock.find_tab(target)).flatten());
        let Some(found) = pick else {
            return false;
        };
        dock.remove_tab(found);
        true
    }

    /// Drop floating windows left with no tabs (a plain close emptied
    /// them, or a force close took their last copy).
    fn drop_empty_floats(&mut self) {
        self.floating.retain(|window| {
            window
                .dock
                .as_ref()
                .is_none_or(|dock| dock.iter_all_tabs().next().is_some())
        });
    }

    /// Close a native tab and drop its buffer. Callers confirm dirty tabs first.
    /// `force` (`:q!`, `:qa!`, explicit discard) closes every view of the
    /// file. A plain close removes only its issuing view (`issuer`):
    /// one docked copy in the issuing project, or the issuing copy of the
    /// floating window; every other copy stays open with its buffer.
    ///
    /// Callers rendering inside the workspace checkout
    /// (`native_editor_view`) must defer through `pending_native_close`:
    /// the workspace is removed from `layouts` while it renders, so a
    /// direct close finds no tab, drops the buffer, and the tab springs
    /// back on the next frame.
    pub(super) fn close_native_tab(
        &mut self,
        path: &Path,
        force: bool,
        issuer: Option<CloseIssuer>,
    ) {
        let target = Tab::NativeEditor {
            path: path.to_owned(),
        };
        if force {
            for workspace in self.layouts.values_mut() {
                while Self::remove_one_docked_copy(workspace, path) {}
            }
            for window in &mut self.floating {
                if let Some(dock) = window.dock.as_mut() {
                    while Self::remove_copy_from_float_dock(dock, &target, None, false) {}
                }
            }
            self.drop_empty_floats();
        } else {
            match issuer {
                // One copy in the issuing tab: the recorded pane first,
                // then that tab's focused copy. Same-file twin tabs and
                // other projects keep theirs.
                Some(CloseIssuer::Docked {
                    project,
                    tab,
                    node,
                    delayed,
                }) => {
                    let target = Tab::NativeEditor {
                        path: path.to_owned(),
                    };
                    // A delayed `:wq` naming its pane closes exactly that
                    // pane: with the issuing view gone, every fallback
                    // below would take a copy that never asked to close.
                    let strict = delayed && node.is_some();
                    let mut removed = false;
                    if let Some(workspace) = self.layouts.get_mut(&project) {
                        if let Some(tab_state) = workspace
                            .tabs
                            .iter_mut()
                            .find(|tab_state| tab_state.id == *tab)
                        {
                            removed = Self::remove_copy_from_tab(tab_state, &target, node, strict);
                        }
                        if !removed && !strict {
                            // Issuing tab replaced since the request queued:
                            // one copy in the project, focused first.
                            removed = Self::remove_one_docked_copy(workspace, path);
                        }
                        workspace.drop_empty_tabs();
                    }
                    if !removed && !strict {
                        // Project gone since the request queued: take one
                        // copy anywhere rather than dropping the close.
                        for workspace in self.layouts.values_mut() {
                            if Self::remove_one_docked_copy(workspace, path) {
                                break;
                            }
                        }
                    }
                }
                // Only the issuing copy in the issuing window; docked
                // views and other windows keep the file.
                Some(CloseIssuer::Floating {
                    viewport,
                    node,
                    delayed,
                }) => {
                    let strict = delayed && node.is_some();
                    if let Some(window) = self.floating.iter_mut().find(|w| w.viewport == viewport)
                        && let Some(dock) = window.dock.as_mut()
                    {
                        Self::remove_copy_from_float_dock(dock, &target, node, strict);
                    }
                    self.drop_empty_floats();
                }
                // Not view-issued (modal, quit flows): legacy removal of
                // every docked copy.
                None => {
                    for workspace in self.layouts.values_mut() {
                        while Self::remove_one_docked_copy(workspace, path) {}
                    }
                }
            }
        }
        // Split views (`:split` / `:vsplit` / `:tabnew`) share the one
        // buffer: dropping it while another view stays open would reload
        // from disk on the next frame and lose unsaved changes.
        if !self.all_native_tab_paths().contains(&path.to_owned()) {
            self.native_docs.remove(path);
        }
        if self
            .native_close_prompt
            .as_ref()
            .is_some_and(|(prompt, _)| prompt == path)
        {
            self.native_close_prompt = None;
        }
        if self.native_close_after_save.as_deref() == Some(path) {
            self.native_close_after_save = None;
            self.native_close_after_save_issuer = None;
        }
        self.save_layouts();
    }

    pub(super) fn open_native(
        &mut self,
        project: &str,
        path: PathBuf,
        line: Option<u32>,
        split: Option<&str>,
    ) {
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

    /// Render the native editor pane for an open file. `project`/`tab`/
    /// `node` locate the docked view under render, if any: close requests
    /// remember their issuing view so plain closes take only that copy.
    pub(super) fn native_editor_view(
        &mut self,
        ui: &mut egui::Ui,
        path: &Path,
        project: Option<&str>,
        tab: Option<&str>,
        node: Option<egui_dock::NodePath>,
    ) {
        let vim = self.state.settings.native_vim;
        let failed_color = appearance::color(&self.theme.status_failed);
        let services = self.services.clone();
        {
            let doc = self.native_doc(path, vim);
            doc.ensure_loading(&services);
            doc.poll();
        }
        // Language servers pump even while the file loads: slow serving
        // must never stall typing, and diagnostics arrive when they do.
        self.lsp.poll();
        if let Some(line) = self.native_pending_line.remove(path) {
            let col = self.native_pending_col.remove(path).unwrap_or(0);
            let mut keep = false;
            if let Some(doc) = self.native_docs.get_mut(path) {
                if let Some(buffer) = doc.doc.as_mut() {
                    doc.engine.place_cursor(buffer, line, col);
                } else if doc.load_error.is_none() {
                    keep = true;
                }
            }
            if keep {
                self.native_pending_line.insert(path.to_owned(), line);
                self.native_pending_col.insert(path.to_owned(), col);
            }
        }

        if self.native_close_after_save.as_deref() == Some(path) {
            let settled = !self.native_dirty(path)
                && !self.native_docs.get(path).is_some_and(|doc| doc.saving);
            if settled {
                // Deferred: the workspace is checked out of `layouts` while
                // this view renders; closing runs after it is checked back in.
                // Consumed here: every other copy rendering later must not
                // queue its own close for the same request.
                let issuer = self.native_close_after_save_issuer.take();
                self.native_close_after_save = None;
                self.pending_native_close
                    .push((path.to_owned(), false, issuer));
                return;
            }
        }

        // Tab-close prompt for this pane: an in-pane bar, not a modal, so
        // the rest of the GUI stays usable while the choice is pending.
        // Quit-flow prompts carry no issuer and stay modal (`native_close_modal`).
        if let Some((prompt, issuer)) = self.native_close_prompt.clone()
            && prompt == path
            && issuer.is_some()
        {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let enabled = !self
                .native_docs
                .get(path)
                .is_some_and(|doc| doc.saving || doc.loading);
            let bar = appearance::unsaved_close_bar(
                ui,
                appearance::UnsavedCloseBar {
                    theme: &self.theme,
                    message: &format!("{name} has unsaved changes."),
                    enabled,
                },
            );
            if let Some(choice) = bar.choice() {
                self.native_close_prompt = None;
                match choice {
                    appearance::UnsavedCloseChoice::Save => {
                        self.apply_native_close_choice(
                            path,
                            NativeCloseChoice::SaveClose,
                            issuer.clone(),
                        );
                    }
                    // Deferred: the workspace is checked out while this view
                    // renders, so the close runs after it is checked back in.
                    appearance::UnsavedCloseChoice::Discard => {
                        self.pending_native_close
                            .push((path.to_owned(), false, issuer.clone()))
                    }
                    appearance::UnsavedCloseChoice::Cancel => {
                        if let Some(doc) = self.native_docs.get(path) {
                            let focus = source_focus_id(&format!(
                                "{:?}",
                                ui.make_persistent_id((
                                    path,
                                    project,
                                    tab,
                                    node,
                                    ui.ctx().viewport_id()
                                ))
                            ));
                            let _ = doc;
                            ui.memory_mut(|memory| memory.request_focus(focus));
                        }
                    }
                }
                if matches!(
                    choice,
                    appearance::UnsavedCloseChoice::Discard | appearance::UnsavedCloseChoice::Save
                ) {
                    return;
                }
            }
            ui.separator();
        }

        let remote_source = self.remote_file_source(path);
        let file_location = self
            .remote_file_label(path)
            .unwrap_or_else(|| path.display().to_string());
        if let Some((remote, relative)) = &remote_source {
            ui.horizontal(|ui| {
                ui.weak("Remote snapshot · read-only");
                if ui.button("Open in SSH editor").clicked() {
                    self.open_remote_file(remote, relative.clone(), None, None, true);
                }
            });
        }
        let project_root = self.selected_project().map(|project| project.path.clone());
        let breadcrumb = if remote_source.is_some() {
            file_location.clone()
        } else {
            self.selected_project()
                .and_then(|project| path.strip_prefix(&project.path).ok())
                .unwrap_or(path)
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" › ")
        };
        let pinned_files = self.preferences.pinned_native_files.clone();
        let mut recent_files = self.preferences.recent_native_files.clone();
        if let Some(root) = project_root.as_ref() {
            recent_files.retain(|item| item.starts_with(root) && item != path);
        } else {
            recent_files.clear();
        }
        recent_files.sort_by_key(|item| !pinned_files.contains(item));
        let pinned = pinned_files.contains(path);
        let outline = self.lsp.symbols_for(path);
        // Deferred closes run after the borrow below ends.
        let mut defer_close_after_save = false;
        // The view issuing `:wq`: the save settles frames later, when a
        // different view of the file may render first.
        let mut wq_issuer = None;
        let mut quit_all: Option<bool> = None;
        // Same for splits/tabs: the workspace is checked out while this
        // renders, so layout changes wait for the drain below. Hover/goto
        // answers are async too: effects only queue requests, handled
        // after the drain.
        let mut splits: Vec<NativeSplit> = Vec::new();
        let mut hover_at: Option<(native_lsp::Position, (usize, usize))> = None;
        let mut goto_at: Option<native_lsp::Position> = None;
        let mut references_at: Option<native_lsp::Position> = None;
        let mut rename_at: Option<native_lsp::Position> = None;
        let mut open_file_picker = false;
        let mut toggle_pin = false;
        let mut recent_open: Option<PathBuf> = None;
        let activated_editor;
        let language: Language = language_for_path(&path.to_string_lossy());
        let is_markdown = markdown::supported(path);
        let preview_key = format!("native:{}", path.display());
        let preview_hovered = self
            .markdown
            .entries
            .get(&preview_key)
            .is_some_and(|preview| preview.contains_pointer(ui));
        let preview_scroll = if preview_hovered {
            self.markdown
                .entries
                .get(&preview_key)
                .map(|preview| preview.scroll_fraction)
        } else {
            None
        };
        let source_scroll;
        // Split views share one buffer. Their widget and focus ids must
        // still differ per pane, or the second split inherits the first
        // pane's scroll offset and keyboard focus.
        let source_id = format!(
            "{:?}",
            ui.make_persistent_id((path, project, tab, node, ui.ctx().viewport_id()))
        );
        if self.native_focus_pending.as_ref().is_some_and(|target| {
            project == Some(target.project.as_str())
                && tab == Some(target.group.as_str())
                && node == Some(target.path.node_path())
        }) && self
            .native_docs
            .get(path)
            .is_some_and(|doc| doc.doc.is_some())
        {
            self.native_focus_pending = None;
            ui.memory_mut(|memory| memory.request_focus(source_focus_id(&source_id)));
        }
        let rename_applying = self
            .native_rename
            .as_ref()
            .is_some_and(|dialog| dialog.applying_to(path));
        // Diagnostics snapshots for this frame: the manager borrow ends
        // before the buffer borrow below begins.
        let marks: Vec<DiagnosticMark> = self.lsp.row_marks(path);
        let problems = self.lsp.problems_for(path);
        let remote_snapshot = crate::remote_ui::is_snapshot(&self.paths.data, path);
        let post = {
            let doc = self.native_doc(path, vim);
            let mut post = PostEdit::Keep;
            // Clicking a header button moves keyboard focus to it; hand focus
            // back to the file so arrows and typing keep editing text.
            let file_focus = source_focus_id(&source_id);
            let current_symbol = doc.doc.as_ref().and_then(|_| {
                let line = u32::try_from(doc.engine.cursor().line).ok()?;
                outline
                    .iter()
                    .filter(|symbol| {
                        symbol.range.start.line <= line && line <= symbol.range.end.line
                    })
                    .max_by_key(|symbol| (symbol.depth, symbol.range.start.line))
                    .map(|symbol| symbol.name.as_str())
            });
            let mut heading_jump = None;
            // Wrapped: on a narrow pane the mode badge and buttons move to
            // a second row instead of being clipped off the edge.
            ui.horizontal_wrapped(|ui| {
                let title_width = (ui.available_width() - if is_markdown { 260.0 } else { 200.0 })
                    .clamp(48.0, 260.0);
                ui.add_sized(
                    [title_width, appearance::TOOLBAR_BUTTON],
                    egui::Label::new(egui::RichText::new(&breadcrumb).weak()).truncate(),
                )
                .on_hover_text(&file_location);
                if let Some(symbol) = current_symbol {
                    ui.weak("›");
                    ui.monospace(symbol);
                }
                if doc.dirty() {
                    ui.monospace("●");
                }
                if is_markdown {
                    for (option, icon) in [
                        (markdown::Mode::Edit, "Pencil"),
                        (markdown::Mode::Preview, "Eye"),
                        (markdown::Mode::Split, "Columns2"),
                    ] {
                        let response = appearance::selectable_icon(
                            ui,
                            icon,
                            option.label(),
                            doc.markdown_mode == option,
                        );
                        #[cfg(feature = "test-support")]
                        crate::diagnostics::record(
                            ui.ctx(),
                            &format!("native-markdown-mode:{}", option.label()),
                            response.rect,
                        );
                        if response.clicked() {
                            doc.markdown_mode = option;
                            if option == markdown::Mode::Edit {
                                ui.memory_mut(|memory| memory.request_focus(file_focus));
                            }
                        }
                    }
                    appearance::icon_menu_button(ui, "ListTree", |ui| {
                        let Some(buffer) = doc.doc.as_ref() else {
                            return;
                        };
                        let mut found = false;
                        for block in parse_blocks(buffer) {
                            let BlockKind::Heading(level) = block.kind else {
                                continue;
                            };
                            found = true;
                            let title = buffer.line_text(block.start_line);
                            let title = title.trim_start_matches('#').trim();
                            if ui
                                .button(format!("{} {title}", "·".repeat(level.into())))
                                .clicked()
                            {
                                heading_jump = Some(block.start_line);
                                ui.close();
                            }
                        }
                        if !found {
                            ui.weak("No headings");
                        }
                    })
                    .response
                    .on_hover_text("Go to heading");
                } else if vim {
                    // Modeless editing has no mode to report: the badge
                    // would always read NORMAL and only add noise.
                    let mode = doc.engine.mode();
                    ui.colored_label(mode_color(mode), format!("-- {} --", mode_name(mode)));
                }
                if doc.saving {
                    ui.weak("Saving…");
                }
                if rename_applying {
                    ui.weak("Renaming…");
                }
                if ui
                    .add_enabled_ui(doc.dirty() && !doc.saving && !rename_applying, |ui| {
                        appearance::sidebar_action(ui, "Save", "Save file")
                    })
                    .inner
                    .clicked()
                {
                    doc.force_save = false;
                    doc.start_save(&services);
                    ui.memory_mut(|memory| memory.request_focus(file_focus));
                }
                // Reload while dirty would silently discard edits: disabled.
                let reload = ui
                    .add_enabled_ui(!doc.dirty() && !doc.loading && !rename_applying, |ui| {
                        appearance::sidebar_action(ui, "RefreshCw", "Reload file")
                    })
                    .inner
                    .on_disabled_hover_text("Unsaved changes are kept");
                if reload.clicked() {
                    doc.load_error = None;
                    doc.loading = false;
                    doc.load_rx = None;
                    doc.doc = None;
                    ui.memory_mut(|memory| memory.request_focus(file_focus));
                }
                if native_lsp::server_language(language).is_some()
                    && appearance::sidebar_action(ui, "ListTree", "Find references").clicked()
                {
                    let cursor = doc.engine.cursor();
                    let row = doc
                        .doc
                        .as_ref()
                        .map(|live| live.line_text(cursor.line))
                        .unwrap_or_default();
                    references_at = Some(native_lsp::Position {
                        line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                        character: native_lsp::char_col_to_utf16(&row, cursor.col),
                    });
                    ui.memory_mut(|memory| memory.request_focus(file_focus));
                }
                if appearance::selectable_icon(
                    ui,
                    "FileText",
                    if pinned {
                        "Remove bookmark"
                    } else {
                        "Bookmark file"
                    },
                    pinned,
                )
                .clicked()
                {
                    toggle_pin = true;
                    ui.memory_mut(|memory| memory.request_focus(file_focus));
                }
                appearance::icon_menu_button(ui, "History", |ui| {
                    if recent_files.is_empty() {
                        ui.weak("No other recent files in this project");
                    }
                    for item in &recent_files {
                        let name = item.file_name().unwrap_or_default().to_string_lossy();
                        let marker = if pinned_files.contains(item) {
                            "◆ "
                        } else {
                            ""
                        };
                        if ui
                            .button(format!("{marker}{name}"))
                            .on_hover_text(item.display().to_string())
                            .clicked()
                        {
                            recent_open = Some(item.clone());
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Recent files");
            });
            if let Some(line) = heading_jump
                && let Some(buffer) = doc.doc.as_mut()
            {
                doc.markdown_mode = markdown::Mode::Edit;
                doc.engine.place_cursor(buffer, line, 0);
                ui.memory_mut(|memory| memory.request_focus(file_focus));
            }
            if let Some(error) = doc.save_error.clone() {
                ui.horizontal(|ui| {
                    ui.colored_label(failed_color, format!("Save failed: {error}"));
                    if error.contains("Changed on disk") && ui.small_button("Save anyway").clicked()
                    {
                        doc.force_save = true;
                        doc.start_save(&services);
                        ui.memory_mut(|memory| memory.request_focus(file_focus));
                    }
                });
            }
            let on_disk = doc.disk_mtime_cached();
            if !doc.dirty() && doc.doc.is_some() && on_disk != doc.loaded_mtime && !doc.loading {
                // Clean buffer follows the disk without asking; nothing is lost.
                doc.doc = None;
                doc.load_rx = None;
            }
            if doc.dirty() && doc.doc.is_some() && on_disk != doc.loaded_mtime {
                ui.weak("Changed on disk. Reload is disabled while dirty; Save anyway overwrites.");
            }
            if let Some(error) = doc.load_error.clone() {
                ui.colored_label(failed_color, format!("Could not open: {error}"));
                return;
            }
            let Some(buffer) = doc.doc.as_mut() else {
                ui.weak("Loading…");
                ui.ctx().request_repaint();
                return;
            };
            let mode = if is_markdown {
                doc.markdown_mode
            } else {
                markdown::Mode::Edit
            };
            // `:set wrap` / `:set nowrap` (also from `~/.vimrc`) override
            // the per-type default: Markdown wraps, code does not.
            let soft_wrap = doc.engine.wrap().unwrap_or(is_markdown);
            let source = |ui: &mut egui::Ui| {
                show_source(
                    ui,
                    buffer,
                    &mut doc.engine,
                    &SourceOptions {
                        vim,
                        read_only: rename_applying || remote_snapshot,
                        menu_style: Some(appearance::menu_style),
                        menu_header: Some(appearance::target_header),
                        menu_item: Some(appearance::menu_item),
                        soft_wrap,
                        font_size: if is_markdown { 14.0 } else { 13.0 },
                        scroll_fraction: if mode == markdown::Mode::Split {
                            preview_scroll
                        } else {
                            None
                        },
                        diagnostics: marks,
                        ..SourceOptions::default()
                    },
                    &mut doc.highlight,
                    language,
                    &source_id,
                )
            };
            let outcome = match mode {
                markdown::Mode::Preview => None,
                markdown::Mode::Edit => Some(source(ui)),
                markdown::Mode::Split => {
                    let width = ui.available_width();
                    Some(
                        egui::Panel::left(egui::Id::unique(("native-markdown-editor", path)))
                            .resizable(true)
                            .default_size(width * 0.5)
                            .size_range(80.0..=(width - 80.0).max(80.0))
                            .frame(egui::Frame::NONE)
                            .show(ui, source)
                            .inner,
                    )
                }
            };
            activated_editor = outcome.as_ref().is_some_and(|outcome| outcome.activated);
            source_scroll = outcome.as_ref().map(|outcome| outcome.scroll_fraction);
            if let Some(outcome) = outcome {
                #[cfg(feature = "test-support")]
                crate::diagnostics::record(ui.ctx(), "native-file", outcome.content_rect);
                #[cfg(feature = "test-support")]
                if let Some(rect) = outcome.rename_action_rect {
                    crate::diagnostics::record(ui.ctx(), "native-rename-action", rect);
                }
                #[cfg(feature = "test-support")]
                if let Some(rect) = outcome.hover_action_rect {
                    crate::diagnostics::record(ui.ctx(), "native-hover-action", rect);
                }
                for effect in outcome.effects {
                    match effect {
                        Effect::Save => {
                            doc.force_save = false;
                            doc.start_save(&services);
                        }
                        Effect::SaveForce => {
                            doc.force_save = true;
                            doc.start_save(&services);
                        }
                        Effect::Quit => {
                            if doc.dirty() {
                                doc.save_error =
                                    Some("No write since last change (add ! to override)".into());
                            } else {
                                post = PostEdit::Close { force: false };
                            }
                        }
                        Effect::QuitForce => {
                            post = PostEdit::Close { force: true };
                        }
                        Effect::WriteQuit { force } => {
                            // The write is async: closing here would always see
                            // a dirty buffer and refuse, so defer to the
                            // save-settled check instead.
                            doc.force_save = force;
                            doc.start_save(&services);
                            defer_close_after_save = true;
                            wq_issuer = Self::close_issuer(ui, project, tab, node);
                        }
                        Effect::QuitAll { force } => {
                            quit_all = Some(force);
                        }
                        Effect::Split => {
                            splits.push(NativeSplit::Below);
                        }
                        Effect::Vsplit => {
                            splits.push(NativeSplit::Beside);
                        }
                        Effect::TabNew => {
                            splits.push(NativeSplit::NewTab);
                        }
                        Effect::OpenFilePicker => {
                            open_file_picker = true;
                        }
                        Effect::Hover => {
                            let cursor = doc.engine.cursor();
                            let row = doc
                                .doc
                                .as_ref()
                                .map(|live| live.line_text(cursor.line))
                                .unwrap_or_default();
                            hover_at = Some((
                                native_lsp::Position {
                                    line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                                    character: native_lsp::char_col_to_utf16(&row, cursor.col),
                                },
                                (cursor.line, cursor.col),
                            ));
                        }
                        Effect::GotoDefinition => {
                            let cursor = doc.engine.cursor();
                            let row = doc
                                .doc
                                .as_ref()
                                .map(|live| live.line_text(cursor.line))
                                .unwrap_or_default();
                            goto_at = Some(native_lsp::Position {
                                line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                                character: native_lsp::char_col_to_utf16(&row, cursor.col),
                            });
                        }
                        Effect::FindReferences => {
                            let cursor = doc.engine.cursor();
                            let row = doc
                                .doc
                                .as_ref()
                                .map(|live| live.line_text(cursor.line))
                                .unwrap_or_default();
                            references_at = Some(native_lsp::Position {
                                line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                                character: native_lsp::char_col_to_utf16(&row, cursor.col),
                            });
                        }
                        Effect::RenameSymbol => {
                            let cursor = doc.engine.cursor();
                            let row = doc
                                .doc
                                .as_ref()
                                .map(|live| live.line_text(cursor.line))
                                .unwrap_or_default();
                            rename_at = Some(native_lsp::Position {
                                line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                                character: native_lsp::char_col_to_utf16(&row, cursor.col),
                            });
                        }
                        Effect::Bell => {}
                    }
                }
            }
            // Problems panel: click targets under the header. Clicks
            // place the cursor directly; the buffer is borrowed here.
            if !problems.is_empty() {
                let mut jumps: Vec<(usize, usize)> = Vec::new();
                let errors = problems
                    .iter()
                    .filter(|problem| problem.severity == Some(native_lsp::Severity::Error))
                    .count();
                let warnings = problems
                    .iter()
                    .filter(|problem| problem.severity == Some(native_lsp::Severity::Warning))
                    .count();
                let mut summary = format!(
                    "{} problem{}",
                    problems.len(),
                    if problems.len() == 1 { "" } else { "s" }
                );
                if errors > 0 {
                    summary.push_str(&format!(
                        " · {errors} error{}",
                        if errors == 1 { "" } else { "s" }
                    ));
                }
                if warnings > 0 {
                    summary.push_str(&format!(
                        " · {warnings} warning{}",
                        if warnings == 1 { "" } else { "s" }
                    ));
                }
                ui.horizontal(|ui| {
                    ui.weak(summary);
                });
                // The panel must not push the source view off screen; the
                // first five stay clickable and the rest are counted.
                const LISTED: usize = 5;
                for problem in problems.iter().take(LISTED) {
                    let line = usize::try_from(problem.range.start.line).unwrap_or(usize::MAX);
                    let row = doc
                        .doc
                        .as_ref()
                        .map(|live| live.line_text(line))
                        .unwrap_or_default();
                    let col = native_lsp::utf16_to_char_col(&row, problem.range.start.character);
                    let tag = match problem.severity {
                        Some(native_lsp::Severity::Error) => "E",
                        Some(native_lsp::Severity::Warning) => "W",
                        _ => "·",
                    };
                    let label = format!(
                        "{tag} {}: {}",
                        line.saturating_add(1),
                        problem.message.lines().next().unwrap_or("")
                    );
                    if ui.small_button(label).clicked() {
                        jumps.push((line, col));
                    }
                }
                if problems.len() > LISTED {
                    ui.weak(format!("+{} more", problems.len().saturating_sub(LISTED)));
                }
                for (line, col) in jumps {
                    if let Some(live) = doc.doc.as_mut() {
                        doc.engine.place_cursor(live, line, col);
                    }
                }
            }
            post
        };
        if is_markdown {
            let mode = self.native_docs.get(path).map(|doc| doc.markdown_mode);
            if mode != Some(markdown::Mode::Edit) {
                let text = self
                    .native_docs
                    .get(path)
                    .and_then(|doc| doc.doc.as_ref())
                    .map(Doc::text);
                if let Some(text) = text {
                    let state = if self.native_dirty(path) {
                        "Unsaved changes"
                    } else {
                        "Saved"
                    };
                    let cursor = self.native_docs.get(path).map(|doc| doc.engine.cursor());
                    let status = cursor.map_or_else(
                        || state.to_owned(),
                        |cursor| {
                            format!(
                                "Ln {}, Col {}  ·  {state}",
                                cursor.line.saturating_add(1),
                                cursor.col.saturating_add(1)
                            )
                        },
                    );
                    let remote_base = std::path::PathBuf::from("/__terminator_remote__")
                        .join(path.file_name().unwrap_or_default());
                    let preview = self.markdown.snapshot(
                        &preview_key,
                        if remote_snapshot { &remote_base } else { path },
                        &text,
                    );
                    let mut link = None;
                    let mut show_preview = |ui: &mut egui::Ui| {
                        let sync = if mode == Some(markdown::Mode::Split) && !preview_hovered {
                            source_scroll
                        } else {
                            None
                        };
                        link = preview.show_synced(ui, &preview_key, Some(&status), sync);
                    };
                    if mode == Some(markdown::Mode::Split) {
                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(ui, &mut show_preview);
                    } else {
                        show_preview(ui);
                    }
                    if let Some(link) = link {
                        match link {
                            markdown::Link::File(target) => {
                                self.open_file(target, None, None, false);
                            }
                            markdown::Link::Web(url) => {
                                let _ = self.jobs.send(Job::Browser(url));
                            }
                        }
                    }
                }
            }
        }
        if let PostEdit::Close { force } = post {
            self.pending_native_close.push((
                path.to_owned(),
                force,
                Self::close_issuer(ui, project, tab, node),
            ));
            return;
        }
        if activated_editor {
            self.follow_focus_tab(Some(Tab::NativeEditor {
                path: path.to_owned(),
            }));
        }
        if open_file_picker {
            self.open_path = true;
        }
        if toggle_pin {
            if pinned {
                self.preferences.pinned_native_files.remove(path);
            } else {
                self.preferences.pinned_native_files.insert(path.to_owned());
            }
        }
        if let Some(path_to_open) = recent_open
            && let Some(uri) = native_lsp::file_uri(&path_to_open)
        {
            let at = native_lsp::Position {
                line: 0,
                character: 0,
            };
            self.pending_goto.push(lsp_manager::GotoJump {
                from: path.to_owned(),
                targets: vec![native_lsp::GotoTarget {
                    uri: uri.to_string(),
                    range: native_lsp::Range { start: at, end: at },
                }],
            });
        }
        // Language-server sync for the visible buffer: clone the text
        // only when the manager actually wants it.
        if let Some(revision) = self
            .native_docs
            .get(path)
            .and_then(|doc| doc.doc.as_ref())
            .map(|buffer| buffer.revision())
            && self.lsp.sync_doc(path, language, revision)
            && let Some(text) = self
                .native_docs
                .get(path)
                .and_then(|doc| doc.doc.as_ref())
                .map(|buffer| buffer.text())
        {
            self.lsp.submit_text(path, language, revision, &text);
        }
        if let Some((position, cursor)) = hover_at {
            self.lsp.request_hover(path, position, cursor);
        }
        if let Some(position) = goto_at {
            self.lsp.request_definition(path, position);
        }
        if let Some(position) = references_at {
            self.native_references = None;
            self.lsp.request_references(path, position);
        }
        if let Some(position) = rename_at {
            self.begin_native_rename(path, project, position);
        }
        if let Some(results) = self.lsp.take_references() {
            self.native_references = Some(results);
        }
        // Finished jumps queue until the workspace is checked back in:
        // this view renders while it is checked out, so opening the
        // target now would miss its project and drop the jump.
        if let Some(jump) = self.lsp.take_goto() {
            self.pending_goto.push(jump);
        }
        self.show_hover_popup(ui, path);
        self.show_references_popup(ui, path);
        if defer_close_after_save {
            self.native_close_after_save = Some(path.to_owned());
            self.native_close_after_save_issuer = wq_issuer.map(CloseIssuer::deferred);
        }
        // `:qa` resolves at drain time: the current workspace is checked
        // out of `layouts` here, so its tabs are invisible to the dirty
        // check until check-in.
        if quit_all.is_some() {
            self.pending_quit_all = quit_all;
        }
        // The issuing viewport travels with the request so splits land
        // in the floating window they were typed in.
        let viewport = ui.ctx().viewport_id();
        for split in splits {
            self.pending_native_splits
                .push((viewport, path.to_owned(), split));
        }
    }

    /// Open one finished goto jump: same file places the cursor, other
    /// files open at the target line and column through the pending
    /// machinery (`open_native` carries the line, `native_pending_col`
    /// the column). Runs after the workspace is checked back in, when
    /// the originating project resolves again.
    fn apply_goto_jump(&mut self, jump: lsp_manager::GotoJump) {
        let Some(target) = jump.targets.first() else {
            return;
        };
        let Some(target_path) = native_lsp::uri_to_path(&target.uri) else {
            return;
        };
        let line = usize::try_from(target.range.start.line).unwrap_or(usize::MAX);
        // Column from live text when possible (the same-file buffer
        // converts unsaved edits exactly), else the file on disk.
        let row = self
            .native_docs
            .get(&target_path)
            .and_then(|doc| doc.doc.as_ref())
            .map(|buffer| buffer.line_text(line))
            .or_else(|| {
                std::fs::read_to_string(&target_path)
                    .ok()
                    .and_then(|text| text.lines().nth(line).map(str::to_owned))
            })
            .unwrap_or_default();
        let col = native_lsp::utf16_to_char_col(&row, target.range.start.character);
        if target_path == jump.from
            && let Some(doc) = self.native_docs.get_mut(&target_path)
            && let Some(buffer) = doc.doc.as_mut()
        {
            doc.engine.place_cursor(buffer, line, col);
            return;
        }
        if target_path == jump.from {
            return;
        }
        let Some(project) = self.project_with_native_tab(&jump.from) else {
            return;
        };
        self.open_native(
            &project,
            target_path.clone(),
            target.range.start.line.checked_add(1),
            None,
        );
        let absolute = std::path::absolute(&target_path).unwrap_or(target_path);
        self.native_pending_col.insert(absolute, col);
    }

    /// Hover answer window for one file: cursor moves and the X both
    /// dismiss; the manager owns the text.
    fn show_hover_popup(&mut self, ui: &mut egui::Ui, path: &Path) {
        let cursor = self.native_docs.get(path).map(|doc| doc.engine.cursor());
        let Some(cursor) = cursor else {
            return;
        };
        let Some((text, markdown)) = self.lsp.hover_popup(path, (cursor.line, cursor.col)) else {
            return;
        };
        let mut open = true;
        self.popups
            .window(ui.ctx(), "Hover")
            .open(&mut open)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(500.0)
                    .show(ui, |ui| {
                        ui.set_max_width(700.0);
                        if markdown {
                            egui_commonmark::CommonMarkViewer::new()
                                .explicit_image_uri_scheme(true)
                                .show(ui, &mut self.native_hover_markdown_cache, &text);
                        } else {
                            ui.monospace(&text);
                        }
                    });
            });
        if !open {
            self.lsp.close_hover();
        }
    }

    fn show_references_popup(&mut self, ui: &mut egui::Ui, path: &Path) {
        let Some(results) = self
            .native_references
            .as_ref()
            .filter(|results| results.from == path)
        else {
            return;
        };
        let mut open = true;
        let mut selected = None;
        self.popups
            .window(ui.ctx(), "References")
            .open(&mut open)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                if results.targets.is_empty() {
                    ui.weak("No references found.");
                }
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for target in &results.targets {
                            let label = native_lsp::uri_to_path(&target.uri).map_or_else(
                                || target.uri.clone(),
                                |path| path.display().to_string(),
                            );
                            if ui
                                .selectable_label(
                                    false,
                                    format!(
                                        "{label}:{}",
                                        target.range.start.line.saturating_add(1)
                                    ),
                                )
                                .clicked()
                            {
                                selected = Some(target.clone());
                            }
                        }
                    });
            });
        if let Some(target) = selected {
            self.pending_goto.push(lsp_manager::GotoJump {
                from: path.to_owned(),
                targets: vec![target],
            });
            open = false;
        }
        if !open {
            self.native_references = None;
        }
    }

    /// Run deferred native closes after the workspace is checked back into
    /// `layouts`. Call sites render outside the checkout (modals, quit
    /// flows) keep calling `close_native_tab` directly. Deferred `:split`
    /// / `:vsplit` / `:tabnew` drain here too: the workspace is checked
    /// out while the file view renders, so layout changes wait for it.
    /// Finished goto jumps drain here as well: opening a cross-file
    /// target needs the originating project, which is missing while the
    /// workspace is checked out for rendering.
    pub(super) fn drain_pending_native_close(&mut self) {
        let pending = std::mem::take(&mut self.pending_native_close);
        for (path, force, issuer) in &pending {
            self.close_native_tab(path, *force, issuer.clone());
        }
        let splits = std::mem::take(&mut self.pending_native_splits);
        for (viewport, path, split) in splits {
            self.split_native_view(viewport, &path, split);
        }
        let jumps = std::mem::take(&mut self.pending_goto);
        for jump in jumps {
            self.apply_goto_jump(jump);
        }
        if let Some(force) = self.pending_quit_all.take() {
            let paths = self.all_native_tab_paths();
            if !force {
                let dirty: Vec<PathBuf> = paths
                    .iter()
                    .filter(|p| self.native_dirty(p))
                    .cloned()
                    .collect();
                if !dirty.is_empty() {
                    for path in &dirty {
                        if let Some(doc) = self.native_docs.get_mut(path) {
                            doc.save_error = Some("Unsaved changes (add ! to override)".into());
                        }
                    }
                    return;
                }
            }
            // `:qa` takes every view including floating ones; the dirty
            // guard above already ran for the non-force case.
            for path in &paths {
                self.close_native_tab(path, true, None);
            }
        }
    }

    /// Project owning a live native view of `path`, for deferred splits:
    /// the originating project is preserved, never the selected one.
    /// Floating windows count too: a jump issued from a floated editor
    /// opens in its home project.
    fn project_with_native_tab(&self, path: &Path) -> Option<String> {
        self.layouts
            .iter()
            .find_map(|(project, workspace)| {
                workspace
                    .tabs
                    .iter()
                    .flat_map(|tab| tab.layout.iter_all_tabs())
                    .any(|(_, tab)| {
                        matches!(tab, Tab::NativeEditor { path: current } if current == path)
                    })
                    .then(|| project.clone())
            })
            .or_else(|| {
                self.floating.iter().find_map(|window| {
                    window
                        .dock
                        .as_ref()
                        .is_some_and(|dock| {
                            dock.iter_all_tabs().any(|(_, tab)| {
                                matches!(tab, Tab::NativeEditor { path: current } if current == path)
                            })
                        })
                        .then(|| window.home.0.clone())
                })
            })
    }

    /// Carry out a deferred `:split` / `:vsplit` / `:tabnew`: the same
    /// file opens beside, below, or in a new top-level tab of its own
    /// project — or beside/below inside the issuing floating window.
    /// Both views share the one buffer and engine for now (one cursor
    /// across panes); per-view cursors are future work.
    fn split_native_view(&mut self, viewport: egui::ViewportId, path: &Path, split: NativeSplit) {
        let tab = Tab::NativeEditor {
            path: path.to_owned(),
        };
        if split == NativeSplit::NewTab {
            let Some(project) = self.project_with_native_tab(path) else {
                return;
            };
            self.place_gui_tab(
                project,
                tab,
                After::Workspace(terminator_core::id(), Vec::new()),
            );
            return;
        }
        // A split issued from a floating window stays in that window.
        if viewport != egui::ViewportId::ROOT
            && let Some(window) = self
                .floating
                .iter_mut()
                .find(|window| window.viewport == viewport)
            && let Some(dock) = window.dock.as_mut()
        {
            Self::split_dock(dock, tab, split);
            return;
        }
        let Some(project) = self.project_with_native_tab(path) else {
            return;
        };
        let Some(dock) = self.layouts.get_mut(&project) else {
            return;
        };
        Self::split_dock(dock, tab, split);
    }

    /// Add one more view of `tab` beside or below the focused leaf. The
    /// issuing copy is focused first when present, so repeated splits fan
    /// out from the pane the user is typing in; a dock that no longer
    /// holds it (closed or reloaded between frames) still splits at its
    /// focused leaf instead of silently dropping the request.
    fn split_dock(dock: &mut egui_dock::DockState<Tab>, tab: Tab, split: NativeSplit) {
        if let Some(found) = dock.find_tab(&tab) {
            dock.set_focused_node_and_surface(found.node_path());
        }
        let tree = dock.main_surface_mut();
        if tree.is_empty() {
            tree.push_to_focused_leaf(tab);
            return;
        }
        let node = tree.focused_leaf().unwrap_or(NodeIndex::root());
        let placed = match split {
            NativeSplit::Below => tree.split_below(node, 0.5, vec![tab]),
            _ => tree.split_right(node, 0.5, vec![tab]),
        };
        tree.set_focused_node(placed[1]);
    }

    /// Apply a dirty-editor dialog choice for one path. Discard closes the
    /// issuing view when the prompt names one (scoped like `:q`, never
    /// every copy) and keeps legacy removal for flow-issued prompts.
    /// Save-and-close defers the issuing pane through the save so the
    /// settled close lands on the view that asked instead of every
    /// docked copy — or instead of leaving a float open.
    pub(super) fn apply_native_close_choice(
        &mut self,
        path: &Path,
        choice: NativeCloseChoice,
        issuer: Option<CloseIssuer>,
    ) {
        match choice {
            NativeCloseChoice::Discard => match issuer {
                Some(issuer) => self.close_native_tab(path, false, Some(issuer)),
                None => self.close_native_tab(path, true, None),
            },
            NativeCloseChoice::SaveClose => {
                self.native_close_after_save = Some(path.to_owned());
                self.native_close_after_save_issuer = issuer.map(CloseIssuer::deferred);
                if let Some(doc) = self.native_docs.get_mut(path) {
                    doc.force_save = false;
                    doc.start_save(&self.services.clone());
                }
            }
        }
    }

    pub(super) fn native_close_modal(&mut self, ctx: &egui::Context) {
        let Some((path, issuer)) = self.native_close_prompt.clone() else {
            return;
        };
        // Tab-close prompts render as an in-pane bar inside the issuing
        // view; the modal would steal input from the rest of the GUI.
        if issuer.is_some() {
            return;
        }
        if !self.native_dirty(&path) {
            self.close_native_tab(&path, false, issuer);
            return;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut open = true;
        let mut save_close = false;
        let mut discard = false;
        let mut cancel = false;
        self.popups
            .window(ctx, "Unsaved native changes?")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("{name} has unsaved changes."));
                ui.horizontal(|ui| {
                    if ui.button("Save and close").clicked() {
                        save_close = true;
                    }
                    if ui.button("Discard changes").clicked() {
                        discard = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if cancel {
            open = false;
        }
        if discard {
            self.apply_native_close_choice(&path, NativeCloseChoice::Discard, issuer);
        } else if save_close {
            self.apply_native_close_choice(&path, NativeCloseChoice::SaveClose, issuer);
        } else if !open {
            // Dismissing the prompt is the quit's Cancel. Sparkle keeps
            // waiting if we only hide the prompt.
            self.cancel_app_quit();
        }
        if !open {
            self.native_close_prompt = None;
        }
    }
}

#[cfg(test)]
impl NativeDoc {
    pub(super) fn dirty_for_test(path: PathBuf) -> Self {
        let mut doc = Self::new(path, true);
        // The buffer carries an edit the saved revision never saw.
        let mut buffer = Doc::new("unsaved");
        buffer.insert(7, " edit");
        doc.doc = Some(buffer);
        doc
    }
}

#[cfg(test)]
mod view_buffer_tests {
    use super::*;

    #[test]
    fn remote_markdown_loads_up_to_two_mib_and_local_limit_stays_one_mib() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let path = crate::remote_ui::snapshot_root(&app.paths.data, "fixture").join("large.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes = vec![b'a'; terminator_core::remote::MAX_FILE_BYTES];
        std::fs::write(&path, &bytes).unwrap();
        let limit = app.native_doc(&path, false).read_limit;
        assert_eq!(
            read_native_file(&path, limit).unwrap().text.len(),
            bytes.len()
        );
        let local = dir.path().join("local.md");
        std::fs::write(&local, &bytes).unwrap();
        assert!(read_native_file(&local, app.native_doc(&local, false).read_limit).is_err());
        std::fs::write(&path, vec![b'a'; bytes.len() + 1]).unwrap();
        assert!(read_native_file(&path, limit).is_err());
    }

    #[test]
    fn closing_a_clean_editor_copy_keeps_the_other_views_undo_history() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let path = dir.path().join("shared.txt");
        let pane = Tab::NativeEditor { path: path.clone() };
        let mut native = NativeDoc::new(path.clone(), true);
        let mut buffer = Doc::new("original");
        native.engine.press_key(&mut buffer, VimKey::Char('i'));
        native.engine.type_text(&mut buffer, "saved ");
        native.engine.press_key(&mut buffer, VimKey::Escape);
        native.saved_revision = buffer.revision();
        native.doc = Some(buffer);
        app.native_docs.insert(path.clone(), native);
        app.layouts.insert(
            "a".into(),
            Workspace::from_layout(egui_dock::DockState::new(vec![pane.clone(), pane.clone()])),
        );
        let at = app.layouts["a"].find_tab(&pane).unwrap();
        {
            let mut viewer = Viewer {
                app: &mut app,
                strip: false,
                project: Some("a".into()),
                tab: None,
                window: None,
                render_path: None,
                render_tab: None,
            };
            assert_eq!(
                viewer.on_close(&mut pane.clone(), at.node_path()),
                OnCloseResponse::Close
            );
        }
        app.layouts.get_mut("a").unwrap().remove_tab(at);
        app.prune_native_docs();
        let native = app
            .native_docs
            .get_mut(&path)
            .expect("shared buffer remains");
        let buffer = native.doc.as_mut().unwrap();
        native.engine.undo(buffer);
        assert_eq!(buffer.text(), "original");
    }

    #[test]
    fn an_edit_during_a_save_stays_unsaved_when_the_write_lands() {
        let mut native = NativeDoc::new(PathBuf::from("/tmp/save-race.txt"), true);
        native.doc = Some(Doc::new("one"));
        native.saved_revision = native.doc.as_ref().map_or(0, Doc::revision);
        assert!(!native.dirty(), "a freshly loaded buffer is clean");
        // The write snapshots the revision it is sending to disk.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        native.save_rx = Some(rx);
        native.saving = true;
        native.save_revision = native.doc.as_ref().map_or(0, Doc::revision);
        // Typing while the write runs.
        native.doc.as_mut().expect("buffer").insert(3, " more");
        assert!(native.dirty());
        tx.send(Ok(Some(SystemTime::now())))
            .expect("deliver the save result");
        native.poll();
        assert!(!native.saving);
        assert!(
            native.dirty(),
            "the buffer changed after the snapshot: it is still unsaved"
        );
        assert_eq!(native.doc.as_ref().expect("buffer").text(), "one more");

        // A save that takes the newer text clears the flag again.
        native.save_revision = native.doc.as_ref().map_or(0, Doc::revision);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        native.save_rx = Some(rx);
        native.saving = true;
        tx.send(Ok(Some(SystemTime::now())))
            .expect("deliver the save result");
        native.poll();
        assert!(!native.dirty(), "the written revision is clean again");
    }

    #[test]
    fn disk_mtime_reads_are_throttled_between_frames() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("throttled.txt");
        std::fs::write(&path, "one").unwrap();
        let mut native = NativeDoc::new(path, true);
        assert!(native.disk_mtime_cached().is_some());
        // Inside the TTL the recorded reading stands in for a fresh stat.
        native.mtime_check = Some((Instant::now(), Some(SystemTime::UNIX_EPOCH)));
        assert_eq!(
            native.disk_mtime_cached(),
            Some(SystemTime::UNIX_EPOCH),
            "a reading younger than the TTL is reused"
        );
        // Past the TTL the file is checked again.
        native.mtime_check = Some((
            Instant::now()
                .checked_sub(MTIME_TTL + Duration::from_millis(1))
                .expect("instant inside the clock range"),
            Some(SystemTime::UNIX_EPOCH),
        ));
        assert_ne!(
            native.disk_mtime_cached(),
            Some(SystemTime::UNIX_EPOCH),
            "the expired reading is replaced with a fresh stat"
        );
    }
}
