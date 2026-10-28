//! The native editor's GUI-owned buffer: load, dirty tracking, disk
//! modification-time probes, and atomic saves.
use super::gutter;
use crate::gui_services;
use crate::markdown;
use anyhow::Context as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use terminator_native_edit::doc::{Buffer, Doc};
use terminator_native_edit::highlight::HighlightCache;
use terminator_native_edit::lsp as native_lsp;
use terminator_native_edit::vim::{
    Key as VimKey, ModalEngine, VimEngine, VimrcOptions, parse_vimrc,
};
pub(super) const MAX_NATIVE_BYTES: u64 = 1024 * 1024;

pub struct LoadedFile {
    pub(super) text: String,
    pub(super) mtime: Option<SystemTime>,
}

pub(super) fn read_native_file(path: &Path, limit: u64) -> anyhow::Result<LoadedFile> {
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

pub(super) fn disk_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

/// Options from `~/.vimrc`, read once per process. Only `set`/`setlocal`
/// names the native engine implements are applied (wrap, ignorecase);
/// mappings, plugins, and unknown options are ignored, so a large vimrc
/// can never break the editor.
pub(super) fn vimrc_options() -> VimrcOptions {
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

pub(super) type LoadResult = anyhow::Result<LoadedFile>;
pub(super) type SaveResult = anyhow::Result<Option<SystemTime>>;

/// How long a disk modification-time reading stays valid. The header
/// re-checks the file on every frame; stat-ing a network or cold volume
/// that often is wasted work, and a sub-second delay before a clean buffer
/// follows an external edit is invisible.
pub(super) const MTIME_TTL: Duration = Duration::from_millis(900);

pub struct NativeDoc {
    pub path: PathBuf,
    pub(super) doc: Option<Doc>,
    pub(super) markdown_mode: markdown::Mode,
    pub(super) engine: VimEngine,
    pub(super) highlight: HighlightCache,
    pub(super) insert_first: bool,
    /// Buffer revision that matches the bytes on disk: the dirty check
    /// compares revisions instead of whole texts, so typing costs O(1)
    /// per frame instead of one full-text copy per frame.
    pub(super) saved_revision: u64,
    /// Revision of the text handed to the in-flight save. Edits typed
    /// while the write runs keep the buffer dirty when it lands.
    pub(super) save_revision: u64,
    /// Hash of the text on disk (as loaded or last saved) and of the
    /// in-flight save's text: an edit undone back to the saved text is
    /// clean again, like `IntelliJ`.
    pub(super) saved_hash: u64,
    pub(super) save_hash: u64,
    /// Engines and layout caches of the other views of this buffer (split
    /// panes), keyed by view id. `engine`/`highlight` belong to `holder`;
    /// between frames the holder is the focused view, so LSP, find, and
    /// navigation act on the caret the user is working with.
    pub(super) views: std::collections::HashMap<String, (VimEngine, HighlightCache)>,
    /// Gutter change blocks against the `HEAD` text, keyed by buffer
    /// revision and base identity, with their marks for the view.
    pub(super) gutter: Option<GutterCache>,
    /// Language-server inlay hints adopted for this buffer.
    pub(super) inlay: terminator_native_edit::inlay::InlayCache,
    pub(super) holder: Option<String>,
    pub(super) focused_view: Option<String>,
    /// The file uses CRLF line endings: the buffer holds `\n` only (so
    /// carets never land on a `\r`) and saves convert back.
    pub(super) crlf: bool,
    /// `(revision, dirty)` for the last revision checked against
    /// `saved_hash`, so the full-text hash runs once per edit.
    pub(super) dirty_memo: std::cell::Cell<(u64, bool)>,
    pub(super) loaded_mtime: Option<SystemTime>,
    pub(super) read_limit: u64,
    pub(super) mtime_check: Option<(Instant, Option<SystemTime>)>,
    /// In-flight background stat; the GUI thread never stats the file.
    pub(super) mtime_rx: Option<tokio::sync::mpsc::UnboundedReceiver<Option<SystemTime>>>,
    pub(super) load_error: Option<String>,
    pub(super) save_error: Option<String>,
    pub(super) loading: bool,
    pub(super) saving: bool,
    pub(super) force_save: bool,
    pub(super) load_rx: Option<tokio::sync::mpsc::UnboundedReceiver<LoadResult>>,
    pub(super) save_rx: Option<tokio::sync::mpsc::UnboundedReceiver<SaveResult>>,
}

impl NativeDoc {
    /// Make view `id`'s caret and layout the live ones, stashing the
    /// previous holder's. A view seen for the first time starts as a fork
    /// of the current caret.
    pub(super) fn enter_view(&mut self, id: &str) {
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
    pub(super) fn restore_focused_view(&mut self) {
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
    pub(super) fn disk_mtime_cached(
        &mut self,
        services: &gui_services::Services,
    ) -> Option<SystemTime> {
        self.poll_mtime();
        if self.mtime_probe_due() {
            self.start_mtime_probe(services);
        }
        self.mtime_check
            .map_or(self.loaded_mtime, |(_, mtime)| mtime)
    }

    pub(super) fn mtime_probe_due(&self) -> bool {
        self.mtime_rx.is_none()
            && self
                .mtime_check
                .is_none_or(|(checked, _)| checked.elapsed() >= MTIME_TTL)
    }

    pub(super) fn poll_mtime(&mut self) {
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
    pub(super) fn reset_mtime(&mut self) {
        self.mtime_check = None;
        self.mtime_rx = None;
    }

    pub(super) fn start_mtime_probe(&mut self, services: &gui_services::Services) {
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

    pub(super) fn ensure_loading(&mut self, services: &gui_services::Services) {
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

    pub(super) fn poll(&mut self) {
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

    pub(super) fn start_save(&mut self, services: &gui_services::Services) {
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

pub(super) type GutterCache = (
    u64,
    usize,
    std::sync::Arc<Vec<gutter::GutterHunk>>,
    std::sync::Arc<Vec<terminator_native_edit::view::ChangeMark>>,
);

pub(super) fn text_hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Replace a file's contents without a window where it is truncated or
/// half-written: write a sibling temp file, give it the target's
/// permissions, then rename it over the target. A symlink is followed so
/// the link itself survives.
pub(super) fn write_atomically(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
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

#[cfg(test)]
impl NativeDoc {
    pub(crate) fn dirty_for_test(path: PathBuf) -> Self {
        let mut doc = Self::new(path, true);
        // The buffer carries an edit the saved revision never saw.
        let mut buffer = Doc::new("unsaved");
        buffer.insert(7, " edit");
        doc.doc = Some(buffer);
        doc
    }
}
