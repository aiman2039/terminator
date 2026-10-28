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
mod complete;
mod find;
mod gutter;
mod hierarchy;
mod navigation;
mod rename;
pub(crate) use actions::ActionsPopup;
pub(crate) use blame::EditorBlame;
pub(super) use close::{CloseIssuer, NativeCloseChoice};
pub(crate) use complete::{CompletionPopup, SignaturePopup};
pub(crate) use find::FindBar;
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
use anyhow::Context as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use terminator_native_edit::blocks::{BlockKind, parse_blocks};
use terminator_native_edit::doc::{Buffer, Doc};
use terminator_native_edit::highlight::{HighlightCache, Language, language_for_path};
use terminator_native_edit::lsp as native_lsp;
use terminator_native_edit::view::{
    DiagnosticMark, ShowSourceInput, SourceOptions, mode_color, mode_name, show_source,
    source_focus_id,
};

struct NativeClosePromptBarInput<'a> {
    ui: &'a mut egui::Ui,
    path: &'a Path,
    project: Option<&'a str>,
    tab: Option<&'a str>,
    node: Option<egui_dock::NodePath>,
}

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

struct NativeMarkdownPreviewInput<'a> {
    ui: &'a mut egui::Ui,
    path: &'a Path,
    preview_key: &'a str,
    preview_hovered: bool,
    source_scroll: Option<f32>,
    remote_snapshot: bool,
}

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
    /// Hash of the text on disk (as loaded or last saved) and of the
    /// in-flight save's text: an edit undone back to the saved text is
    /// clean again, like `IntelliJ`.
    saved_hash: u64,
    save_hash: u64,
    /// Engines and layout caches of the other views of this buffer (split
    /// panes), keyed by view id. `engine`/`highlight` belong to `holder`;
    /// between frames the holder is the focused view, so LSP, find, and
    /// navigation act on the caret the user is working with.
    views: std::collections::HashMap<String, (VimEngine, HighlightCache)>,
    /// Gutter change blocks against the `HEAD` text, keyed by buffer
    /// revision and base identity, with their marks for the view.
    gutter: Option<GutterCache>,
    /// Language-server inlay hints adopted for this buffer.
    inlay: terminator_native_edit::inlay::InlayCache,
    holder: Option<String>,
    focused_view: Option<String>,
    /// The file uses CRLF line endings: the buffer holds `\n` only (so
    /// carets never land on a `\r`) and saves convert back.
    crlf: bool,
    /// `(revision, dirty)` for the last revision checked against
    /// `saved_hash`, so the full-text hash runs once per edit.
    dirty_memo: std::cell::Cell<(u64, bool)>,
    loaded_mtime: Option<SystemTime>,
    read_limit: u64,
    mtime_check: Option<(Instant, Option<SystemTime>)>,
    /// In-flight background stat; the GUI thread never stats the file.
    mtime_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Option<SystemTime>>>,
    load_error: Option<String>,
    save_error: Option<String>,
    loading: bool,
    saving: bool,
    force_save: bool,
    load_rx: Option<tokio::sync::mpsc::UnboundedReceiver<LoadResult>>,
    save_rx: Option<tokio::sync::mpsc::UnboundedReceiver<SaveResult>>,
}

impl NativeDoc {
    /// Make view `id`'s caret and layout the live ones, stashing the
    /// previous holder's. A view seen for the first time starts as a fork
    /// of the current caret.
    fn enter_view(&mut self, id: &str) {
        if self.holder.as_deref() == Some(id) {
            return;
        }
        if self.focused_view.is_none() {
            self.focused_view = Some(id.to_owned());
        }
        let Some(previous) = self.holder.replace(id.to_owned()) else {
            return;
        };
        let (engine, highlight) = self
            .views
            .remove(id)
            .unwrap_or_else(|| (self.engine.fork_view(), HighlightCache::default()));
        let old_engine = std::mem::replace(&mut self.engine, engine);
        let old_highlight = std::mem::replace(&mut self.highlight, highlight);
        self.views.insert(previous, (old_engine, old_highlight));
    }

    /// Put the focused view's caret back in place after another view drew.
    fn restore_focused_view(&mut self) {
        if let Some(focused) = self.focused_view.clone()
            && self.holder.as_ref() != Some(&focused)
            && self.views.contains_key(&focused)
        {
            self.enter_view(&focused);
        }
    }

    /// A buffer already loaded with `text` (tests outside this module).
    #[cfg(test)]
    pub(crate) fn loaded(path: PathBuf, text: &str) -> Self {
        let mut doc = Self::new(path, false);
        doc.doc = Some(Doc::new(text));
        doc
    }

    /// Loaded buffer, if any (outline and caret readers).
    pub(crate) fn buffer(&self) -> Option<&Doc> {
        self.doc.as_ref()
    }

    /// Caret line of a loaded buffer.
    pub(crate) fn caret_line(&self) -> Option<usize> {
        self.doc.as_ref().map(|_| self.engine.cursor().line)
    }

    /// Caret as an LSP position (line, UTF-16 column).
    pub(crate) fn caret_position(&self) -> Option<(u32, u32)> {
        let doc = self.doc.as_ref()?;
        let cursor = self.engine.cursor();
        let character = native_lsp::char_col_to_utf16(&doc.line_text(cursor.line), cursor.col);
        Some((u32::try_from(cursor.line).ok()?, character))
    }

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
            saved_hash: 0,
            save_hash: 0,
            crlf: false,
            views: std::collections::HashMap::new(),
            gutter: None,
            inlay: terminator_native_edit::inlay::InlayCache::default(),
            holder: None,
            focused_view: None,
            dirty_memo: std::cell::Cell::new((0, false)),
            loaded_mtime: None,
            read_limit: MAX_NATIVE_BYTES,
            mtime_check: None,
            mtime_rx: None,
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
        let Some(doc) = self.doc.as_ref() else {
            return false;
        };
        if doc.revision() == self.saved_revision {
            return false;
        }
        let (revision, dirty) = self.dirty_memo.get();
        if revision == doc.revision() {
            return dirty;
        }
        let dirty = text_hash(&doc.text()) != self.saved_hash;
        self.dirty_memo.set((doc.revision(), dirty));
        dirty
    }

    /// Disk modification time, re-stat-ed on the file pool at most every
    /// [`MTIME_TTL`]. Until the first reading lands, the loaded time stands
    /// in, so a slow volume never blocks a frame or flags a false change.
    fn disk_mtime_cached(&mut self, services: &gui_services::Services) -> Option<SystemTime> {
        self.poll_mtime();
        if self.mtime_probe_due() {
            self.start_mtime_probe(services);
        }
        self.mtime_check
            .map_or(self.loaded_mtime, |(_, mtime)| mtime)
    }

    fn mtime_probe_due(&self) -> bool {
        self.mtime_rx.is_none()
            && self
                .mtime_check
                .is_none_or(|(checked, _)| checked.elapsed() >= MTIME_TTL)
    }

    fn poll_mtime(&mut self) {
        let Some(rx) = self.mtime_rx.as_mut() else {
            return;
        };
        match rx.try_recv() {
            Ok(mtime) => {
                self.mtime_check = Some((Instant::now(), mtime));
                self.mtime_rx = None;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => self.mtime_rx = None,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
        }
    }

    /// Forget the last reading after a load or save moved `loaded_mtime`;
    /// a stat still in flight may predate the write.
    fn reset_mtime(&mut self) {
        self.mtime_check = None;
        self.mtime_rx = None;
    }

    fn start_mtime_probe(&mut self, services: &gui_services::Services) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let path = self.path.clone();
        let service = services.clone();
        let context = OperationContext::new(
            "native-mtime",
            path.to_string_lossy().into(),
            Policy::ReplaceableRead,
        );
        if services
            .handle()
            .submit(context, CancellationToken::new(), async move {
                let token = CancellationToken::new();
                if let Ok(mtime) = service
                    .fs()
                    .run(&token, move || Ok(disk_mtime(&path)))
                    .await
                {
                    let _ = tx.send(mtime);
                }
                Ok(Vec::new())
            })
            .is_ok()
        {
            self.mtime_rx = Some(rx);
        } else {
            // Busy pool: keep the last reading and retry after the TTL.
            self.mtime_check = Some((
                Instant::now(),
                self.mtime_check.map_or(self.loaded_mtime, |(_, m)| m),
            ));
        }
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
                    self.reset_mtime();
                    self.crlf = loaded.text.contains("\r\n");
                    let text = if self.crlf {
                        loaded.text.replace("\r\n", "\n")
                    } else {
                        loaded.text
                    };
                    self.saved_hash = text_hash(&text);
                    self.doc = Some(Doc::new(text));
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
                    self.saved_hash = self.save_hash;
                    self.loaded_mtime = mtime;
                    self.reset_mtime();
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
        self.save_hash = text_hash(&text);
        let text = if self.crlf {
            text.replace('\n', "\r\n")
        } else {
            text
        };
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
                        write_atomically(&path, text.as_bytes())?;
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

type GutterCache = (
    u64,
    usize,
    std::sync::Arc<Vec<gutter::GutterHunk>>,
    std::sync::Arc<Vec<terminator_native_edit::view::ChangeMark>>,
);

fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Replace a file's contents without a window where it is truncated or
/// half-written: write a sibling temp file, give it the target's
/// permissions, then rename it over the target. A symlink is followed so
/// the link itself survives.
fn write_atomically(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write as _;
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = target.parent().context("File has no parent folder")?;
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = dir.join(format!(".{name}.terminator-save-{}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&temp).context("Create temporary file")?;
        file.write_all(bytes).context("Write file")?;
        file.sync_all().context("Flush file")?;
        if let Ok(meta) = std::fs::metadata(&target) {
            std::fs::set_permissions(&temp, meta.permissions()).context("Copy permissions")?;
        }
        std::fs::rename(&temp, &target).context("Replace file")
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
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

    /// Render the native editor pane for an open file. `project`/`tab`/
    /// `node` locate the docked view under render, if any: close requests
    /// remember their issuing view so plain closes take only that copy.
    pub(super) fn native_editor_view(&mut self, input: NativeEditorViewInput<'_>) {
        let NativeEditorViewInput {
            ui,
            path,
            project,
            tab,
            node,
        } = input;
        let vim = self.state.settings.native_vim;
        let failed_color = appearance::color(&self.theme.status_failed);
        let services = self.services.clone();
        self.native_view_load(path, vim, &services);

        if self.native_close_after_save_settled(path) {
            return;
        }

        if self.native_close_prompt_bar(NativeClosePromptBarInput {
            ui,
            path,
            project,
            tab,
            node,
        }) {
            return;
        }

        let remote_source = self.remote_file_source(path);
        let file_location = self
            .remote_file_label(path)
            .unwrap_or_else(|| path.display().to_string());
        if let Some((remote, relative)) = &remote_source {
            ui.horizontal(|ui| {
                ui.weak("Remote snapshot · read-only");
                if ui.button("Open in SSH editor").clicked() {
                    self.open_remote_file(OpenRemoteFileInput {
                        project: remote,
                        path: relative.clone(),
                        line: None,
                        split: None,
                        text: true,
                    });
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
        // The breadcrumb needs the outline whether or not the Structure
        // panel is open; the request is revision-gated, so this is cheap.
        let outline = self.document_outline(path);
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
        let mut hierarchy_at: Option<(native_lsp::Position, native_lsp::HierarchyKind)> = None;
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
        // "Copy reference" names the file relative to its project.
        let reference = self
            .selected_project()
            .and_then(|project| path.strip_prefix(&project.path).ok())
            .unwrap_or(path)
            .display()
            .to_string();
        // Blame column for this frame (blank while a fresh blame runs).
        let blame_revision = self
            .native_docs
            .get(path)
            .and_then(|doc| doc.doc.as_ref())
            .map(Buffer::revision);
        let annotations =
            blame_revision.and_then(|revision| self.editor_blame_annotations(path, revision));
        let blame_offer = (!crate::remote_ui::is_snapshot(&self.paths.data, path))
            .then(|| self.editor_blame.contains_key(path));
        let mut blame_toggle = false;
        let mut blame_click = None;
        let inlay_answer = self
            .state
            .settings
            .inlay_hints
            .then(|| self.lsp.inlay_hints(path))
            .flatten();
        let remote_snapshot = crate::remote_ui::is_snapshot(&self.paths.data, path);
        // The find bar is checked out while this view borrows the buffer.
        let mut find = if self.find_bar.as_ref().is_some_and(|bar| bar.path == path) {
            self.find_bar.take()
        } else {
            None
        };
        let mut completion = if self
            .completion
            .as_ref()
            .is_some_and(|popup| popup.path == path)
        {
            self.completion.take()
        } else {
            None
        };
        let mut complete_now = false;
        let mut signature = if self
            .signature
            .as_ref()
            .is_some_and(|popup| popup.path == path)
        {
            self.signature.take()
        } else {
            None
        };
        let mut signature_request = None;
        let mut actions_popup = if self
            .code_actions
            .as_ref()
            .is_some_and(|popup| popup.path == path)
        {
            self.code_actions.take()
        } else {
            None
        };
        let mut format_now = false;
        let mut actions_at: Option<native_lsp::Range> = None;
        let mut picked_action: Option<native_lsp::CodeActionItem> = None;
        let git_base = if crate::remote_ui::is_snapshot(&self.paths.data, path) {
            None
        } else {
            self.git_base(path)
        };
        let mut gutter_popup = if self
            .gutter_popup
            .as_ref()
            .is_some_and(|popup| popup.path == path)
        {
            self.gutter_popup.take()
        } else {
            None
        };
        let change_colors = [
            appearance::color(&self.theme.git_added),
            appearance::color(&self.theme.git_modified),
            appearance::color(&self.theme.git_deleted),
        ];
        let mut completion_request = None;
        let caret_rect;
        let post = {
            let doc = self.native_doc(path, vim);
            // Split views of one buffer keep their own caret and layout.
            doc.enter_view(&source_id);
            let mut post = PostEdit::Keep;
            // Clicking a header button moves keyboard focus to it; hand focus
            // back to the file so arrows and typing keep editing text.
            let file_focus = source_focus_id(&source_id);
            let current_symbol = doc.doc.as_ref().and_then(|_| {
                let line = u32::try_from(doc.engine.cursor().line).ok()?;
                native_lsp::innermost_symbol(&outline, line)
                    .and_then(|index| outline.get(index))
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
                if doc.crlf {
                    ui.weak("CRLF")
                        .on_hover_text("Windows line endings; saving keeps them");
                }
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
            let on_disk = doc.disk_mtime_cached(&services);
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
            let squiggles = terminator_native_edit::view::squiggles(&*buffer, &problems);
            let inlay_hints = if let Some((revision, hints)) = &inlay_answer {
                doc.inlay.adopt(&*buffer, *revision, hints);
                doc.inlay.visible(&*buffer)
            } else {
                doc.inlay.clear();
                std::sync::Arc::new(Vec::new())
            };
            let revision_before = buffer.revision();
            // Gutter changes against HEAD, once per edit (or new base).
            let (gutter_hunks, change_marks) = match &git_base {
                Some(base) => {
                    let key = (
                        buffer.revision(),
                        std::sync::Arc::as_ptr(base).cast::<u8>() as usize,
                    );
                    match &doc.gutter {
                        Some((revision, ptr, hunks, marks)) if (*revision, *ptr) == key => {
                            (std::sync::Arc::clone(hunks), std::sync::Arc::clone(marks))
                        }
                        _ => {
                            let base_text = if doc.crlf {
                                base.replace("\r\n", "\n")
                            } else {
                                base.to_string()
                            };
                            let hunks =
                                std::sync::Arc::new(gutter::hunks(&base_text, &buffer.text()));
                            let marks = std::sync::Arc::new(
                                hunks.iter().map(|hunk| hunk.mark).collect::<Vec<_>>(),
                            );
                            doc.gutter = Some((
                                key.0,
                                key.1,
                                std::sync::Arc::clone(&hunks),
                                std::sync::Arc::clone(&marks),
                            ));
                            (hunks, marks)
                        }
                    }
                }
                None => (std::sync::Arc::default(), std::sync::Arc::default()),
            };
            let popup_keys = completion
                .as_ref()
                .is_some_and(|popup| !popup.visible.is_empty())
                || actions_popup
                    .as_ref()
                    .is_some_and(|popup| !popup.items.is_empty());
            let mut find_matches = std::sync::Arc::new(Vec::new());
            let mut find_current = None;
            if let Some(bar) = find.as_mut() {
                // Split views of one file share the bar: the first view
                // this frame draws it, the others only highlight.
                let pass = ui.ctx().cumulative_pass_nr();
                let (outcome, matches) = if bar.drawn == Some(pass) {
                    (find::FindOutcome::default(), bar.matches(buffer))
                } else {
                    bar.drawn = Some(pass);
                    find::find_bar(find::FindBarInput {
                        ui,
                        bar,
                        buffer,
                        engine: &mut doc.engine,
                        read_only: rename_applying || remote_snapshot,
                    })
                };
                find_matches =
                    std::sync::Arc::new(matches.iter().map(|m| (m.line, m.start, m.end)).collect());
                find_current = bar.current;
                if outcome.close {
                    find = None;
                    ui.memory_mut(|memory| memory.request_focus(source_focus_id(&source_id)));
                }
            }
            let source = |ui: &mut egui::Ui| {
                show_source(
                    ui,
                    buffer,
                    &mut doc.engine,
                    ShowSourceInput {
                        options: &SourceOptions {
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
                            squiggles,
                            find_matches,
                            find_current,
                            popup_keys,
                            change_marks,
                            change_colors,
                            inlay_hints,
                            reference: Some(reference),
                            annotations,
                            blame: blame_offer,
                            ..SourceOptions::default()
                        },
                        highlight: &mut doc.highlight,
                        language,
                        id: &source_id,
                    },
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
            caret_rect = outcome.as_ref().and_then(|outcome| outcome.caret_rect);
            if let Some(outcome) = outcome.as_ref() {
                blame_toggle = outcome.toggle_blame;
                blame_click = outcome.annotation_clicked;
            }
            if let Some((index, at)) = outcome.as_ref().and_then(|outcome| outcome.change_clicked) {
                gutter_popup = Some(GutterPopup {
                    path: path.to_owned(),
                    index,
                    at,
                });
            }
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
                        Effect::Hierarchy { types } => {
                            let cursor = doc.engine.cursor();
                            let row = doc
                                .doc
                                .as_ref()
                                .map(|live| live.line_text(cursor.line))
                                .unwrap_or_default();
                            hierarchy_at = Some((
                                native_lsp::Position {
                                    line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                                    character: native_lsp::char_col_to_utf16(&row, cursor.col),
                                },
                                if types {
                                    native_lsp::HierarchyKind::Type
                                } else {
                                    native_lsp::HierarchyKind::Call
                                },
                            ));
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
                        Effect::Complete => complete_now = true,
                        Effect::Format => format_now = true,
                        Effect::CodeActions => {
                            // The selection, else the caret, as an LSP range.
                            if let Some(live) = doc.doc.as_ref() {
                                let caret = doc.engine.cursor();
                                let (from, to) = doc.engine.selection().unwrap_or((caret, caret));
                                let position = |cursor: terminator_native_edit::vim::Cursor| {
                                    native_lsp::Position {
                                        line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
                                        character: native_lsp::char_col_to_utf16(
                                            &live.line_text(cursor.line),
                                            cursor.col,
                                        ),
                                    }
                                };
                                actions_at = Some(native_lsp::Range {
                                    start: position(from),
                                    end: position(to),
                                });
                            }
                        }
                        Effect::FindBar { replace } => {
                            // Seed with a one-line selection, IntelliJ-style.
                            let seed = doc
                                .doc
                                .as_ref()
                                .filter(|_| doc.engine.selection().is_some())
                                .map(|live| doc.engine.copy_text(live))
                                .filter(|text| !text.contains('\n'))
                                .unwrap_or_default();
                            match find.as_mut() {
                                Some(bar) => {
                                    if !seed.is_empty() {
                                        bar.query = seed;
                                    }
                                    bar.replacing |= replace;
                                    bar.focus = true;
                                }
                                None => {
                                    find = Some(find::FindBar::new(path.to_owned(), seed, replace));
                                }
                            }
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
            if let Some(popup) = actions_popup.as_mut() {
                if caret_rect.is_some() {
                    popup.anchor = caret_rect;
                }
                match actions::actions_ui(ui, popup) {
                    actions::ActionPick::Pick(index) => {
                        picked_action = popup.items.get(index).cloned();
                        actions_popup = None;
                    }
                    actions::ActionPick::Close => actions_popup = None,
                    actions::ActionPick::None => {}
                }
            }
            let rollback_index = gutter_popup.as_ref().map(|popup| popup.index);
            if gutter::popup_ui(ui, &mut gutter_popup, &gutter_hunks, change_colors[2])
                && let Some(hunk) = rollback_index.and_then(|index| gutter_hunks.get(index))
                && let Some(live) = doc.doc.as_mut()
            {
                gutter::rollback(live, &mut doc.engine, hunk);
            }
            if let Some(live) = doc.doc.as_ref() {
                signature_request =
                    complete::signature_after_frame(complete::SignatureAfterFrameInput {
                        popup: &mut signature,
                        buffer: live,
                        engine: &doc.engine,
                        revision_before,
                        server: native_lsp::server_language(language).is_some(),
                    });
                if let Some(shown) = signature.as_mut() {
                    if caret_rect.is_some() {
                        shown.anchor = caret_rect;
                    }
                    complete::signature_ui(ui, shown);
                }
            }
            if let Some(live) = doc.doc.as_mut() {
                completion_request = complete::after_frame(complete::AfterFrameInput {
                    ui,
                    popup: &mut completion,
                    buffer: live,
                    engine: &mut doc.engine,
                    revision_before,
                    manual: complete_now,
                    caret_rect,
                    server: native_lsp::server_language(language).is_some(),
                });
            }
            // The view the user clicked or types in owns the live caret
            // between frames; another view only borrowed it to draw.
            if activated_editor || ui.memory(|memory| memory.has_focus(source_focus_id(&source_id)))
            {
                doc.focused_view = Some(source_id.clone());
            }
            doc.restore_focused_view();
            post
        };
        if find.is_some() {
            self.find_bar = find;
        }
        if completion.is_some() {
            self.completion = completion;
        }
        if gutter_popup.is_some() {
            self.gutter_popup = gutter_popup;
            if blame_toggle {
                self.toggle_editor_blame(path);
            }
            if let Some(line) = blame_click {
                self.open_blame_commit(path, line);
            }
        }
        if signature.is_some() {
            self.signature = signature;
        }
        if actions_popup.is_some() {
            self.code_actions = actions_popup;
        }
        if let Some(item) = picked_action {
            self.run_code_action(path, &item);
        }
        if (format_now || actions_at.is_some())
            && let Some((revision, text)) = self
                .native_docs
                .get(path)
                .and_then(|doc| doc.buffer())
                .map(|buffer| (buffer.revision(), buffer.text()))
        {
            // Both act on the text as typed: send it past the debounce.
            self.lsp.submit_text(SubmitTextInput {
                path,
                language,
                revision,
                text: &text,
            });
            if format_now && !self.lsp.request_format(path, revision) {
                self.info = Some("The language server is not ready to format".into());
            }
            if let Some(range) = actions_at {
                match self.lsp.request_code_actions(path, range) {
                    Some(generation) => {
                        self.code_actions = Some(ActionsPopup {
                            path: path.to_owned(),
                            generation: Some(generation),
                            items: Vec::new(),
                            selected: 0,
                            anchor: caret_rect,
                        });
                    }
                    None => self.info = Some("The language server is not ready".into()),
                }
            }
        }
        self.drain_edit_answers();
        if let Some((position, line, open)) = signature_request
            && let Some((revision, text)) = self
                .native_docs
                .get(path)
                .and_then(|doc| doc.buffer())
                .map(|buffer| (buffer.revision(), buffer.text()))
        {
            self.lsp.submit_text(SubmitTextInput {
                path,
                language,
                revision,
                text: &text,
            });
            if let Some(generation) = self.lsp.request_signature_help(path, position) {
                // Keep showing the previous signature until the update lands.
                let help = self
                    .signature
                    .take()
                    .filter(|shown| shown.path == path && shown.open == open)
                    .and_then(|shown| shown.help);
                self.signature = Some(SignaturePopup {
                    path: path.to_owned(),
                    generation,
                    help,
                    line,
                    open,
                    anchor: caret_rect,
                });
            }
        }
        if let Some((answer_path, generation, help)) = self.lsp.take_signature()
            && let Some(shown) = self.signature.as_mut()
            && shown.generation == generation
            && shown.path == answer_path
        {
            shown.help = help;
        }
        // Completion asks with the live text: push it past the sync
        // debounce first so the server sees what was just typed.
        if let Some((position, line, start)) = completion_request
            && let Some((revision, text)) = self
                .native_docs
                .get(path)
                .and_then(|doc| doc.buffer())
                .map(|buffer| (buffer.revision(), buffer.text()))
        {
            self.lsp.submit_text(SubmitTextInput {
                path,
                language,
                revision,
                text: &text,
            });
            if let Some(generation) = self.lsp.request_completion(path, position) {
                self.completion = Some(CompletionPopup {
                    path: path.to_owned(),
                    generation: Some(generation),
                    items: Vec::new(),
                    line,
                    start,
                    visible: Vec::new(),
                    selected: 0,
                    anchor: caret_rect,
                });
            }
        }
        if let Some(results) = self.lsp.take_completion()
            && let Some(popup) = self.completion.as_mut()
            && popup.generation == Some(results.generation)
            && popup.path == results.path
        {
            popup.items = results.items;
            popup.generation = None;
        }
        if is_markdown {
            self.native_markdown_preview(NativeMarkdownPreviewInput {
                ui,
                path,
                preview_key: &preview_key,
                preview_hovered,
                source_scroll,
                remote_snapshot,
            });
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
        self.native_lsp_sync(path, language);
        self.request_editor_blame(ui.ctx(), path);
        if self.state.settings.inlay_hints
            && let Some(lines) = self
                .native_docs
                .get(path)
                .and_then(|doc| doc.doc.as_ref())
                .map(|buffer| buffer.line_count())
        {
            self.lsp.request_inlay_hints(path, lines);
        }
        if let Some((position, cursor)) = hover_at {
            self.lsp.request_hover(path, position, cursor);
        }
        if let Some(position) = goto_at {
            self.lsp.request_definition(path, position);
        }
        if let Some(position) = references_at {
            self.lsp.request_references(path, position);
        }
        if let Some((position, kind)) = hierarchy_at {
            self.start_hierarchy(path, position, kind);
        }
        if let Some(position) = rename_at {
            self.begin_native_rename(path, project, position);
        }
        if let Some(results) = self.lsp.take_references() {
            self.show_usages("Find Usages", results.targets);
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

    /// Per-frame load/poll for one native view, plus the language-server
    /// pump and any pending goto cursor placement.
    fn native_view_load(&mut self, path: &Path, vim: bool, services: &gui_services::Services) {
        {
            let doc = self.native_doc(path, vim);
            doc.ensure_loading(services);
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
                    let chars = native_lsp::utf16_to_char_col(&buffer.line_text(line), col);
                    doc.engine.place_cursor(buffer, line, chars);
                } else if doc.load_error.is_none() {
                    keep = true;
                }
            }
            if keep {
                self.native_pending_line.insert(path.to_owned(), line);
                self.native_pending_col.insert(path.to_owned(), col);
            }
        }
    }

    /// Queue the deferred close for a settled save-then-close (`:wq` or
    /// Save and close). Returns true when the view should stop rendering.
    fn native_close_after_save_settled(&mut self, path: &Path) -> bool {
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
                return true;
            }
        }
        false
    }

    /// In-pane unsaved-changes bar for this view's pending tab close.
    /// Returns true when the view should stop rendering this frame.
    fn native_close_prompt_bar(&mut self, input: NativeClosePromptBarInput<'_>) -> bool {
        let NativeClosePromptBarInput {
            ui,
            path,
            project,
            tab,
            node,
        } = input;
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
                    return true;
                }
            }
            ui.separator();
        }
        false
    }

    /// Markdown preview for a native view in Preview or Split mode.
    fn native_markdown_preview(&mut self, input: NativeMarkdownPreviewInput<'_>) {
        let NativeMarkdownPreviewInput {
            ui,
            path,
            preview_key,
            preview_hovered,
            source_scroll,
            remote_snapshot,
        } = input;
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
                    preview_key,
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
                    link = preview.show_synced(ShowSyncedInput {
                        ui,
                        sid: preview_key,
                        status_override: Some(&status),
                        scroll_fraction: sync,
                    });
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
                            self.open_file(OpenFileInput {
                                path: target,
                                line: None,
                                split: None,
                                external: false,
                            });
                        }
                        markdown::Link::Web(url) => {
                            let _ = self.jobs.send(Job::Browser(url));
                        }
                    }
                }
            }
        }
    }

    fn native_lsp_sync(&mut self, path: &Path, language: Language) {
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
            self.lsp.submit_text(SubmitTextInput {
                path,
                language,
                revision,
                text: &text,
            });
        }
    }

    /// Open one finished goto jump: same file places the cursor, other
    /// files open at the target line and column through the pending
    /// machinery (`open_native` carries the line, `native_pending_col`
    /// the UTF-16 column). Runs after the workspace is checked back in, when
    /// the originating project resolves again.
    fn apply_goto_jump(&mut self, jump: lsp_manager::GotoJump) {
        // Several declarations: let the user pick (`IntelliJ`).
        if jump.targets.len() > 1 {
            self.show_usages(navigation::DECLARATION_TITLE, jump.targets);
            return;
        }
        let Some(target) = jump.targets.first() else {
            return;
        };
        self.record_jump_origin();
        let Some(target_path) = native_lsp::uri_to_path(&target.uri) else {
            return;
        };
        let line = usize::try_from(target.range.start.line).unwrap_or(usize::MAX);
        if target_path == jump.from
            && let Some(doc) = self.native_docs.get_mut(&target_path)
            && let Some(buffer) = doc.doc.as_mut()
        {
            // The live buffer converts unsaved edits exactly.
            let col = native_lsp::utf16_to_char_col(
                &buffer.line_text(line),
                target.range.start.character,
            );
            doc.engine.place_cursor(buffer, line, col);
            return;
        }
        if target_path == jump.from {
            return;
        }
        let Some(project) = self.project_with_native_tab(&jump.from) else {
            return;
        };
        self.open_native(OpenNativeInput {
            project: &project,
            path: target_path.clone(),
            line: target.range.start.line.checked_add(1),
            split: None,
        });
        // The column converts once the target buffer has loaded.
        let absolute = std::path::absolute(&target_path).unwrap_or(target_path);
        self.native_pending_col
            .insert(absolute, target.range.start.character);
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
    fn crlf_files_edit_as_lf_and_save_as_crlf() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("win.txt");
        std::fs::write(&path, "a\r\nb\r\n").unwrap();
        let ctx = egui::Context::default();
        let app = App::with_context(&ctx, Paths::at(dir.path().join("state")));
        let mut native = NativeDoc::new(path.clone(), false);
        native.ensure_loading(&app.services);
        let start = std::time::Instant::now();
        while native.doc.is_none() {
            assert!(start.elapsed() < Duration::from_secs(10), "load");
            native.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(native.crlf);
        assert_eq!(native.doc.as_ref().unwrap().text(), "a\nb\n");
        assert!(!native.dirty());
        if let Some(buffer) = native.doc.as_mut() {
            let end = buffer.len_chars();
            buffer.insert(end, "c\n");
        }
        native.start_save(&app.services);
        while native.saving {
            assert!(start.elapsed() < Duration::from_secs(10), "save");
            native.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(native.save_error, None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\r\nb\r\nc\r\n");
        assert!(!native.dirty());
    }

    #[test]
    fn split_views_keep_their_own_caret_and_the_focused_one_rests_live() {
        let mut native = NativeDoc::new(PathBuf::from("/tmp/views.txt"), false);
        let mut buffer = Doc::new("zero\none\ntwo\n");
        native.enter_view("left");
        let _ = native.engine.place_cursor(&mut buffer, 2, 0);
        // A new view starts at the same caret, then moves on its own.
        native.enter_view("right");
        assert_eq!(native.engine.cursor().line, 2);
        let _ = native.engine.place_cursor(&mut buffer, 0, 1);
        native.focused_view = Some("left".into());
        native.restore_focused_view();
        assert_eq!(native.engine.cursor().line, 2);
        native.enter_view("right");
        assert_eq!(
            native.engine.cursor(),
            terminator_native_edit::vim::Cursor { line: 0, col: 1 }
        );
    }

    #[test]
    fn undoing_back_to_the_saved_text_is_clean() {
        let mut native = NativeDoc::new(PathBuf::from("/tmp/clean.txt"), true);
        let mut buffer = Doc::new("saved");
        native.saved_hash = text_hash("saved");
        native.saved_revision = buffer.revision();
        native.engine.press_key(&mut buffer, VimKey::Char('i'));
        native.engine.type_text(&mut buffer, "x");
        native.engine.press_key(&mut buffer, VimKey::Escape);
        native.doc = Some(buffer);
        assert!(native.dirty());
        if let Some(buffer) = native.doc.as_mut() {
            native.engine.undo(buffer);
        }
        assert!(!native.dirty());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_save_keeps_permissions_and_symlinks() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("run.sh");
        std::fs::write(&target, "old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let link = dir.path().join("link.sh");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_atomically(&link, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o755
        );
        // No temporary file is left behind.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

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
    fn disk_mtime_reads_are_throttled_and_off_thread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("throttled.txt");
        std::fs::write(&path, "one").unwrap();
        let mut native = NativeDoc::new(path, true);
        native.loaded_mtime = Some(SystemTime::UNIX_EPOCH);
        // No reading yet: a probe is due and the loaded time stands in.
        assert!(native.mtime_probe_due());
        // A probe in flight is not started twice.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        native.mtime_rx = Some(rx);
        assert!(!native.mtime_probe_due());
        let fresh = SystemTime::UNIX_EPOCH + Duration::from_secs(5);
        tx.send(Some(fresh)).unwrap();
        native.poll_mtime();
        assert!(native.mtime_rx.is_none());
        assert_eq!(native.mtime_check.map(|(_, m)| m), Some(Some(fresh)));
        // Inside the TTL the recorded reading stands in for a fresh stat.
        assert!(!native.mtime_probe_due());
        native.mtime_check = Some((
            Instant::now()
                .checked_sub(MTIME_TTL + Duration::from_millis(1))
                .expect("instant inside the clock range"),
            Some(fresh),
        ));
        assert!(native.mtime_probe_due(), "an expired reading is re-checked");
        // A save or load drops a stat that may predate the write.
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        native.mtime_rx = Some(rx);
        native.reset_mtime();
        assert!(native.mtime_rx.is_none() && native.mtime_check.is_none());
    }
}
