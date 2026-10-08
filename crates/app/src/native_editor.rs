//! GUI-owned native file editing backing Settings → Editor mode → Native.
//!
//! Unlike daemon editors, native tabs hold their buffer in the GUI: no PTY,
//! no Neovim session. Files load and save on the filesystem worker pool with
//! the same 1 MiB bound as Markdown previews. Saves refuse to overwrite a
//! file that changed on disk unless forced, Reload is disabled while dirty,
//! and closing a dirty tab asks first — nothing is silently discarded.

use super::workspace::WorkspaceTab;
use super::*;
use anyhow::Context as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use terminator_native_edit::doc::{Buffer, Doc};
use terminator_native_edit::highlight::{HighlightCache, Language, language_for_path};
use terminator_native_edit::lsp as native_lsp;
use terminator_native_edit::view::{
    DiagnosticMark, SourceOptions, mode_color, mode_name, show_source, source_focus_id,
};
use terminator_native_edit::vim::{Effect, Key as VimKey, ModalEngine, VimEngine};

const MAX_NATIVE_BYTES: u64 = 1024 * 1024;

pub struct LoadedFile {
    text: String,
    mtime: Option<SystemTime>,
}

fn read_native_file(path: &Path) -> anyhow::Result<LoadedFile> {
    let file = std::fs::File::open(path).context("Open file")?;
    anyhow::ensure!(file.metadata()?.is_file(), "Not a regular file");
    let mut bytes = Vec::new();
    use std::io::Read as _;
    file.take(MAX_NATIVE_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("Read file")?;
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_NATIVE_BYTES,
        "Native editing is limited to 1 MiB"
    );
    let text = String::from_utf8(bytes).context("File is not UTF-8")?;
    let mtime = disk_mtime(path);
    Ok(LoadedFile { text, mtime })
}

fn disk_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

type LoadResult = anyhow::Result<LoadedFile>;
type SaveResult = anyhow::Result<Option<SystemTime>>;

pub struct NativeDoc {
    pub path: PathBuf,
    doc: Option<Doc>,
    engine: VimEngine,
    highlight: HighlightCache,
    insert_first: bool,
    saved_text: String,
    loaded_mtime: Option<SystemTime>,
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
        Self {
            path,
            doc: None,
            engine: VimEngine::new(),
            highlight: HighlightCache::default(),
            insert_first,
            saved_text: String::new(),
            loaded_mtime: None,
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
            .is_some_and(|doc| doc.text() != self.saved_text)
    }

    fn ensure_loading(&mut self, services: &gui_services::Services) {
        if self.doc.is_some() || self.load_error.is_some() || self.loading {
            return;
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.load_rx = Some(rx);
        self.loading = true;
        let path = self.path.clone();
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
                    .run(&token, move || read_native_file(&path))
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
                    self.saved_text.clone_from(&loaded.text);
                    self.loaded_mtime = loaded.mtime;
                    self.doc = Some(Doc::new(loaded.text));
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
                    if let Some(doc) = self.doc.as_ref() {
                        self.saved_text = doc.text();
                    }
                    self.loaded_mtime = mtime;
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
    /// redirect the close.
    Docked {
        project: String,
        tab: String,
        node: Option<egui_dock::NodePath>,
    },
    /// A floating window: plain closes remove that pane.
    Floating(egui::ViewportId),
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
        self.native_docs
            .entry(path.to_owned())
            .or_insert_with(|| NativeDoc::new(path.to_owned(), !vim))
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
        for pane in &self.floating {
            if let Some(Tab::NativeEditor { path }) = pane.tab.as_ref() {
                live.insert(path.clone());
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
            .is_some_and(|path| !live.contains(path))
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
        for pane in &self.floating {
            if let Some(Tab::NativeEditor { path }) = pane.tab.as_ref()
                && !paths.contains(path)
            {
                paths.push(path.clone());
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
            Some(CloseIssuer::Floating(viewport))
        } else {
            match (project, tab) {
                (Some(project), Some(tab)) => Some(CloseIssuer::Docked {
                    project: project.to_owned(),
                    tab: tab.to_owned(),
                    node,
                }),
                _ => None,
            }
        }
    }

    /// Remove one `target` copy from one top-level tab: the copy in
    /// `node` (the issuing pane recorded when the command started), then
    /// the focused leaf, then the first match. A stale node (layout moved
    /// since) simply misses and the fallbacks take over. Fixes `primary`
    /// afterwards. Returns whether a copy was removed.
    fn remove_copy_from_tab(
        tab_state: &mut WorkspaceTab,
        target: &Tab,
        node: Option<egui_dock::NodePath>,
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
                .or_else(|| layout.find_tab(target))
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
            if Self::remove_copy_from_tab(tab, &target, None) {
                removed = true;
                break;
            }
        }
        // Restores the non-empty invariant (`:qa` can empty a
        // workspace); a bare retain panics the next dock access.
        workspace.drop_empty_tabs();
        removed
    }

    /// Close a native tab and drop its buffer. Callers confirm dirty tabs first.
    /// `force` (`:q!`, `:qa!`, explicit discard) closes every view of the
    /// file. A plain close removes only its issuing view (`issuer`):
    /// one docked copy in the issuing project, or the issuing floating
    /// pane; every other copy stays open with its buffer.
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
        let issuer_viewport = match &issuer {
            Some(CloseIssuer::Floating(viewport)) => Some(*viewport),
            _ => None,
        };
        if force {
            for workspace in self.layouts.values_mut() {
                while Self::remove_one_docked_copy(workspace, path) {}
            }
        } else {
            match issuer {
                // One copy in the issuing tab: the recorded pane first,
                // then that tab's focused copy. Same-file twin tabs and
                // other projects keep theirs.
                Some(CloseIssuer::Docked { project, tab, node }) => {
                    let target = Tab::NativeEditor {
                        path: path.to_owned(),
                    };
                    let mut removed = false;
                    if let Some(workspace) = self.layouts.get_mut(&project) {
                        if let Some(tab_state) = workspace
                            .tabs
                            .iter_mut()
                            .find(|tab_state| tab_state.id == *tab)
                        {
                            removed = Self::remove_copy_from_tab(tab_state, &target, node);
                        }
                        if !removed {
                            // Issuing tab replaced since the request queued:
                            // one copy in the project, focused first.
                            removed = Self::remove_one_docked_copy(workspace, path);
                        }
                        workspace.drop_empty_tabs();
                    }
                    if !removed {
                        // Project gone since the request queued: take one
                        // copy anywhere rather than dropping the close.
                        for workspace in self.layouts.values_mut() {
                            if Self::remove_one_docked_copy(workspace, path) {
                                break;
                            }
                        }
                    }
                }
                // Only the issuing pane; docked views keep the file.
                Some(CloseIssuer::Floating(_)) => {}
                // Not view-issued (modal, quit flows): legacy removal of
                // every docked copy.
                None => {
                    for workspace in self.layouts.values_mut() {
                        while Self::remove_one_docked_copy(workspace, path) {}
                    }
                }
            }
        }
        // Floating views hold the path outside the docks. Force closes
        // take all of them; a plain close takes only its issuing pane.
        if force {
            self.floating.retain(|pane| {
                !matches!(&pane.tab, Some(Tab::NativeEditor { path: current }) if current == path)
            });
        } else if let Some(viewport) = issuer_viewport {
            self.floating.retain(|pane| {
                pane.viewport != viewport
                    || !matches!(&pane.tab, Some(Tab::NativeEditor { path: current }) if current == path)
            });
        }
        // Split views (`:split` / `:vsplit` / `:tabnew`) share the one
        // buffer: dropping it while another view stays open would reload
        // from disk on the next frame and lose unsaved changes.
        if !self.all_native_tab_paths().contains(&path.to_owned()) {
            self.native_docs.remove(path);
        }
        if self.native_close_prompt.as_deref() == Some(path) {
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

        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
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
        let language: Language = language_for_path(&path.to_string_lossy());
        // Diagnostics snapshots for this frame: the manager borrow ends
        // before the buffer borrow below begins.
        let marks: Vec<DiagnosticMark> = self.lsp.row_marks(path);
        let problems = self.lsp.problems_for(path);
        let post = {
            let doc = self.native_doc(path, vim);
            let mut post = PostEdit::Keep;
            // Clicking a header button moves keyboard focus to it; hand focus
            // back to the file so arrows and typing keep editing text.
            let file_focus = source_focus_id(&path.to_string_lossy());
            ui.horizontal(|ui| {
                ui.monospace(&filename);
                if doc.dirty() {
                    ui.monospace("●");
                }
                let mode = doc.engine.mode();
                ui.colored_label(mode_color(mode), format!("-- {} --", mode_name(mode)));
                if doc.saving {
                    ui.weak("Saving…");
                }
                if ui
                    .add_enabled(doc.dirty() && !doc.saving, egui::Button::new("Save"))
                    .clicked()
                {
                    doc.force_save = false;
                    doc.start_save(&services);
                    ui.memory_mut(|memory| memory.request_focus(file_focus));
                }
                // Reload while dirty would silently discard edits: disabled.
                let reload = ui
                    .add_enabled(!doc.dirty() && !doc.loading, egui::Button::new("Reload"))
                    .on_disabled_hover_text("Unsaved changes are kept");
                if reload.clicked() {
                    doc.load_error = None;
                    doc.loading = false;
                    doc.load_rx = None;
                    doc.doc = None;
                    ui.memory_mut(|memory| memory.request_focus(file_focus));
                }
            });
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
            let on_disk = disk_mtime(path);
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
            let outcome = show_source(
                ui,
                buffer,
                &mut doc.engine,
                &SourceOptions {
                    vim,
                    diagnostics: marks,
                    ..SourceOptions::default()
                },
                &mut doc.highlight,
                language,
                &path.to_string_lossy(),
            );
            #[cfg(feature = "test-support")]
            crate::diagnostics::record(ui.ctx(), "native-file", outcome.content_rect);
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
                    Effect::Bell => {}
                }
            }
            // Problems panel: click targets under the header. Clicks
            // place the cursor directly; the buffer is borrowed here.
            if !problems.is_empty() {
                let mut jumps: Vec<(usize, usize)> = Vec::new();
                ui.horizontal(|ui| {
                    ui.weak(format!(
                        "{} problem{}",
                        problems.len(),
                        if problems.len() == 1 { "" } else { "s" }
                    ));
                });
                for problem in problems.iter().take(5) {
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
                for (line, col) in jumps {
                    if let Some(live) = doc.doc.as_mut() {
                        doc.engine.place_cursor(live, line, col);
                    }
                }
            }
            post
        };
        if let PostEdit::Close { force } = post {
            self.pending_native_close.push((
                path.to_owned(),
                force,
                Self::close_issuer(ui, project, tab, node),
            ));
            return;
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
        // Finished jumps queue until the workspace is checked back in:
        // this view renders while it is checked out, so opening the
        // target now would miss its project and drop the jump.
        if let Some(jump) = self.lsp.take_goto() {
            self.pending_goto.push(jump);
        }
        self.show_hover_popup(ui, path);
        if defer_close_after_save {
            self.native_close_after_save = Some(path.to_owned());
            self.native_close_after_save_issuer = wq_issuer;
        }
        // `:qa` resolves at drain time: the current workspace is checked
        // out of `layouts` here, so its tabs are invisible to the dirty
        // check until check-in.
        if quit_all.is_some() {
            self.pending_quit_all = quit_all;
        }
        for split in splits {
            self.pending_native_splits.push((path.to_owned(), split));
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
        let Some(text) = self.lsp.hover_popup(path, (cursor.line, cursor.col)) else {
            return;
        };
        let mut open = true;
        self.popups
            .window(ui.ctx(), "Hover")
            .open(&mut open)
            .collapsible(false)
            .show(ui.ctx(), |ui| {
                ui.monospace(&text);
            });
        if !open {
            self.lsp.close_hover();
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
        for (path, split) in splits {
            self.split_native_view(&path, split);
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
                self.floating.iter().find_map(|pane| {
                    matches!(&pane.tab, Some(Tab::NativeEditor { path: current }) if current == path)
                        .then(|| pane.home.0.clone())
                })
            })
    }

    /// Carry out a deferred `:split` / `:vsplit` / `:tabnew`: the same
    /// file opens beside, below, or in a new top-level tab of its own
    /// project. Both views share the one buffer and engine for now (one
    /// cursor across panes); per-view cursors are future work.
    fn split_native_view(&mut self, path: &Path, split: NativeSplit) {
        let Some(project) = self.project_with_native_tab(path) else {
            return;
        };
        let tab = Tab::NativeEditor {
            path: path.to_owned(),
        };
        if split == NativeSplit::NewTab {
            self.place_gui_tab(
                project,
                tab,
                After::Workspace(terminator_core::id(), Vec::new()),
            );
            return;
        }
        let Some(dock) = self.layouts.get_mut(&project) else {
            return;
        };
        // Focus the issuing view first so the split lands next to it.
        if let Some(found) = dock.find_tab(&tab) {
            dock.set_focused_node_and_surface(found.node_path());
            let tree = dock.main_surface_mut();
            if !tree.is_empty() {
                let node = tree.focused_leaf().unwrap_or(NodeIndex::root());
                let placed = match split {
                    NativeSplit::Below => tree.split_below(node, 0.5, vec![tab]),
                    _ => tree.split_right(node, 0.5, vec![tab]),
                };
                tree.set_focused_node(placed[1]);
            }
        }
    }

    pub(super) fn native_close_modal(&mut self, ctx: &egui::Context) {
        let Some(path) = self.native_close_prompt.clone() else {
            return;
        };
        if !self.native_dirty(&path) {
            self.close_native_tab(&path, false, None);
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
            self.close_native_tab(&path, true, None);
        } else if save_close {
            self.native_close_after_save = Some(path.clone());
            if let Some(doc) = self.native_docs.get_mut(&path) {
                doc.force_save = false;
                doc.start_save(&self.services.clone());
            }
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
        doc.doc = Some(Doc::new("unsaved"));
        doc
    }
}
