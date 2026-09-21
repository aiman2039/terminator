//! GUI-owned native file editing backing Settings → Editor mode → Native.
//!
//! Unlike daemon editors, native tabs hold their buffer in the GUI: no PTY,
//! no Neovim session. Files load and save on the filesystem worker pool with
//! the same 1 MiB bound as Markdown previews. Saves refuse to overwrite a
//! file that changed on disk unless forced, Reload is disabled while dirty,
//! and closing a dirty tab asks first — nothing is silently discarded.

use super::*;
use anyhow::Context as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use terminator_native_edit::doc::Doc;
use terminator_native_edit::view::{
    SourceOptions, mode_color, mode_name, show_source, source_focus_id,
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
    fn new(path: PathBuf, insert_first: bool) -> Self {
        Self {
            path,
            doc: None,
            engine: VimEngine::new(),
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
                    self.saved_text = loaded.text.clone();
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
                        self.saved_text = doc.text().to_owned();
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
        let text = doc.text().to_owned();
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
    Close,
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
        self.native_docs.retain(|path, _| live.contains(path));
        self.native_pending_line
            .retain(|path, _| live.contains(path));
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
        }
    }

    /// Every path with a live native editor tab, de-duplicated.
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
        paths
    }

    /// Close a native tab and drop its buffer. Callers confirm dirty tabs first.
    ///
    /// Callers rendering inside the workspace checkout
    /// (`native_editor_view`) must defer through `pending_native_close`:
    /// the workspace is removed from `layouts` while it renders, so a
    /// direct close finds no tab, drops the buffer, and the tab springs
    /// back on the next frame.
    pub(super) fn close_native_tab(&mut self, path: &Path) {
        for workspace in self.layouts.values_mut() {
            for tab in &mut workspace.tabs {
                let target = Tab::NativeEditor {
                    path: path.to_owned(),
                };
                if let Some(found) = tab.layout.find_tab(&target) {
                    tab.layout.remove_tab(found);
                    if tab.primary == Some(target) {
                        tab.primary = tab
                            .layout
                            .iter_all_tabs()
                            .next()
                            .map(|(_, tab)| tab.clone());
                    }
                }
            }
            // Restores the non-empty invariant (`:qa` can empty a
            // workspace); a bare retain panics the next dock access.
            workspace.drop_empty_tabs();
        }
        self.native_docs.remove(path);
        if self.native_close_prompt.as_deref() == Some(path) {
            self.native_close_prompt = None;
        }
        if self.native_close_after_save.as_deref() == Some(path) {
            self.native_close_after_save = None;
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
            self.native_pending_line
                .insert(path.clone(), line as usize - 1);
        }
        let _ = self
            .update_tx
            .send(Update::OpenNativeEditor(project.into(), path, after));
    }

    /// Render the native editor pane for an open file.
    pub(super) fn native_editor_view(&mut self, ui: &mut egui::Ui, path: &Path) {
        let vim = self.state.settings.native_vim;
        let failed_color = appearance::color(&self.theme.status_failed);
        let services = self.services.clone();
        {
            let doc = self.native_doc(path, vim);
            doc.ensure_loading(&services);
            doc.poll();
        }
        if let Some(line) = self.native_pending_line.remove(path) {
            let mut keep = false;
            if let Some(doc) = self.native_docs.get_mut(path) {
                if let Some(buffer) = doc.doc.as_mut() {
                    doc.engine.place_cursor(buffer, line, 0);
                } else if doc.load_error.is_none() {
                    keep = true;
                }
            }
            if keep {
                self.native_pending_line.insert(path.to_owned(), line);
            }
        }

        if self.native_close_after_save.as_deref() == Some(path) {
            let settled = !self.native_dirty(path)
                && !self.native_docs.get(path).is_some_and(|doc| doc.saving);
            if settled {
                // Deferred: the workspace is checked out of `layouts` while
                // this view renders; closing runs after it is checked back in.
                self.pending_native_close.push(path.to_owned());
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
        let mut quit_all: Option<bool> = None;
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
                &SourceOptions::default(),
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
                            post = PostEdit::Close;
                        }
                    }
                    Effect::QuitForce => {
                        post = PostEdit::Close;
                    }
                    Effect::WriteQuit { force } => {
                        // The write is async: closing here would always see
                        // a dirty buffer and refuse, so defer to the
                        // save-settled check instead.
                        doc.force_save = force;
                        doc.start_save(&services);
                        defer_close_after_save = true;
                    }
                    Effect::QuitAll { force } => {
                        quit_all = Some(force);
                    }
                    Effect::Bell => {}
                }
            }
            post
        };
        if matches!(post, PostEdit::Close) {
            self.pending_native_close.push(path.to_owned());
            return;
        }
        if defer_close_after_save {
            self.native_close_after_save = Some(path.to_owned());
        }
        // `:qa` resolves at drain time: the current workspace is checked
        // out of `layouts` here, so its tabs are invisible to the dirty
        // check until check-in.
        if quit_all.is_some() {
            self.pending_quit_all = quit_all;
        }
    }

    /// Run deferred native closes after the workspace is checked back into
    /// `layouts`. Call sites render outside the checkout (modals, quit
    /// flows) keep calling `close_native_tab` directly.
    pub(super) fn drain_pending_native_close(&mut self) {
        let pending = std::mem::take(&mut self.pending_native_close);
        for path in &pending {
            self.close_native_tab(path);
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
            for path in &paths {
                self.close_native_tab(path);
            }
        }
    }

    pub(super) fn native_close_modal(&mut self, ctx: &egui::Context) {
        let Some(path) = self.native_close_prompt.clone() else {
            return;
        };
        if !self.native_dirty(&path) {
            self.close_native_tab(&path);
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
            self.close_native_tab(&path);
        } else if save_close {
            self.native_close_after_save = Some(path.clone());
            if let Some(doc) = self.native_docs.get_mut(&path) {
                doc.force_save = false;
                doc.start_save(&self.services.clone());
            }
        }
        if !open {
            self.native_close_prompt = None;
        }
    }
}
