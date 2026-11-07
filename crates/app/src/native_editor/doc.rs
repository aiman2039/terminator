//! The native editor's GUI-owned buffer: load, dirty tracking, disk
//! modification-time probes, and atomic saves.
use super::disk::{DiskSync, Loaded};
use super::file_io::{LineEnding, SavedFile, TextEncoding};
use super::gutter;
use crate::markdown;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use terminator_native_edit::doc::{Buffer, Doc};
use terminator_native_edit::highlight::HighlightCache;
use terminator_native_edit::indent::EditorConfig;
use terminator_native_edit::lsp as native_lsp;
use terminator_native_edit::vim::{ModalEngine, VimEngine, VimrcOptions, parse_vimrc};
/// Largest file the native editor opens. Saves have no cap: a buffer
/// that grew past it while editing still saves.
pub(super) const MAX_NATIVE_BYTES: u64 = 1024 * 1024;
/// Above this many bytes a buffer opens in large-file mode: no
/// highlighting, folds or underlines until the user enables them.
pub(super) const LARGE_FILE_BYTES: usize = terminator_native_edit::highlight::MAX_HIGHLIGHT_BYTES;

/// Options from `~/.vimrc`, read once per process. Only `set`/`setlocal`
/// names the native engine implements are applied (wrap, ignorecase,
/// indentation); plugins and unknown options are ignored, so a large
/// vimrc can never break the editor.
pub(super) fn vimrc_options() -> &'static VimrcOptions {
    static OPTIONS: std::sync::OnceLock<VimrcOptions> = std::sync::OnceLock::new();
    OPTIONS.get_or_init(|| {
        let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
        else {
            return VimrcOptions::default();
        };
        std::fs::read_to_string(Path::new(&home).join(".vimrc"))
            .map(|text| parse_vimrc(&text))
            .unwrap_or_default()
    })
}

pub(super) type LoadResult = anyhow::Result<Loaded>;
pub(super) type SaveResult = anyhow::Result<SavedFile>;

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
    /// Char counts of the saved text and of the in-flight save's: a
    /// buffer of another length is dirty without hashing it.
    pub(super) saved_len: usize,
    pub(super) save_len: usize,
    /// Encoding and line breaks the next save writes, and the ones on
    /// disk (as saved, and as the in-flight save writes them): a
    /// conversion alone makes the buffer dirty.
    pub(super) encoding: TextEncoding,
    pub(super) saved_format: (TextEncoding, bool),
    pub(super) save_format: (TextEncoding, bool),
    pub(super) disk: DiskSync,
    /// Per-buffer derivations cached across frames.
    pub(super) extras: super::extras::DocExtras,
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
    /// When a view of this buffer last had keyboard focus: Find Action's
    /// editor commands act on the editor focused last.
    pub(super) focused_at: Option<Instant>,
    /// The frame pass each view last drew in: a view that stopped drawing
    /// (closed, split away, or in a hidden tab) is not on screen.
    pub(super) drawn: std::collections::HashMap<String, u64>,
    /// The file's line breaks (see [`LineEnding`]).
    pub(super) line_ending: LineEnding,
    /// `(revision, dirty)` for the last revision checked against
    /// `saved_hash`, so the full-text hash runs once per edit.
    pub(super) dirty_memo: std::cell::Cell<(u64, bool)>,
    pub(super) loaded_mtime: Option<SystemTime>,
    pub(super) read_limit: u64,
    pub(super) mtime_check: Option<(Instant, Option<SystemTime>)>,
    /// In-flight background stat; the GUI thread never stats the file.
    pub(super) mtime_rx:
        Option<tokio::sync::mpsc::UnboundedReceiver<anyhow::Result<Option<SystemTime>>>>,
    pub(super) load_error: Option<String>,
    pub(super) save_error: Option<super::file_io::SaveError>,
    /// A vim message (`:q` refused, `E492`), with the buffer revision it
    /// was set at: the next edit clears it. Not a save failure, so it
    /// does not pause autosave.
    pub(crate) notice: Option<(String, u64)>,
    pub(super) loading: bool,
    pub(super) saving: bool,
    pub(super) force_save: bool,
    /// A finished write the language servers have not heard about yet
    /// (see `take_lsp_saved`).
    pub(super) lsp_saved: bool,
    pub(super) load_rx: Option<tokio::sync::mpsc::UnboundedReceiver<LoadResult>>,
    /// Loads read the file's `.editorconfig` (local files only; remote
    /// snapshots detect their indentation from the text).
    pub(super) read_editorconfig: bool,
    /// `:e newfile`: a missing file loads as an empty new buffer (only
    /// the first load; later reloads report it missing as usual).
    pub(super) new_file: bool,
    /// The `.editorconfig` settings of the last load: its save rules.
    pub(super) file_config: EditorConfig,
    pub(super) save_rx: Option<tokio::sync::mpsc::UnboundedReceiver<SaveResult>>,
    /// The caret, top line and folds the file was left with, put back
    /// once the buffer loads (see `reopen`).
    pub(super) restore: Option<crate::preferences::EditorState>,
    /// Top line the next frame that draws the source scrolls to.
    pub(super) top_line: Option<usize>,
}

impl NativeDoc {
    /// Make view `id`'s caret and layout the live ones, stashing the
    /// previous holder's. A view seen for the first time starts as a fork
    /// of the current caret, folds and top line.
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
        let (engine, highlight) = match self.views.remove(id) {
            Some(parts) => parts,
            None => self.fork_view(),
        };
        let old_engine = std::mem::replace(&mut self.engine, engine);
        let old_highlight = std::mem::replace(&mut self.highlight, highlight);
        // Split views highlight once per edit: the next view reuses spans
        // the previous one computed for the same revision.
        self.highlight.share_spans(&old_highlight);
        self.views.insert(previous, (old_engine, old_highlight));
    }

    /// View `id` drew in this buffer's latest frame: it is on screen.
    pub(super) fn shows_view(&self, id: &str) -> bool {
        let latest = self.drawn.values().max();
        self.drawn
            .get(id)
            .zip(latest)
            .is_some_and(|(pass, latest)| pass.saturating_add(1) >= *latest)
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
        doc.mark_saved_now();
        doc
    }

    /// Loaded buffer, if any (outline and caret readers).
    pub(crate) fn buffer(&self) -> Option<&Doc> {
        self.doc.as_ref()
    }

    /// Loaded buffer text with the file's original line endings (what a
    /// save writes to disk).
    pub(super) fn disk_text(&self) -> Option<String> {
        let doc = self.doc.as_ref()?;
        Some(self.line_ending.encode(doc.text()))
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

    /// Show a vim message until the buffer next changes.
    pub(crate) fn set_notice(&mut self, message: impl Into<String>) {
        let revision = self.doc.as_ref().map_or(0, Buffer::revision);
        self.notice = Some((message.into(), revision));
    }

    /// The current vim message; `None` once the buffer changed after it.
    pub(crate) fn current_notice(&mut self) -> Option<String> {
        let revision = self.doc.as_ref().map_or(0, Buffer::revision);
        if self.notice.as_ref().is_some_and(|(_, at)| *at != revision) {
            self.notice = None;
        }
        self.notice.as_ref().map(|(message, _)| message.clone())
    }

    pub(crate) fn new(path: PathBuf, insert_first: bool) -> Self {
        let mut engine = VimEngine::new();
        engine.apply_vimrc(vimrc_options().clone());
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
            saved_len: 0,
            save_len: 0,
            encoding: TextEncoding::default(),
            saved_format: (TextEncoding::default(), false),
            save_format: (TextEncoding::default(), false),
            disk: DiskSync::default(),
            extras: super::extras::DocExtras::default(),
            read_editorconfig: true,
            file_config: EditorConfig::default(),
            line_ending: LineEnding::Lf,
            views: std::collections::HashMap::new(),
            gutter: None,
            inlay: terminator_native_edit::inlay::InlayCache::default(),
            holder: None,
            focused_view: None,
            focused_at: None,
            drawn: std::collections::HashMap::new(),
            dirty_memo: std::cell::Cell::new((0, false)),
            loaded_mtime: None,
            read_limit: MAX_NATIVE_BYTES,
            new_file: false,
            mtime_check: None,
            mtime_rx: None,
            load_error: None,
            save_error: None,
            notice: None,
            loading: false,
            saving: false,
            force_save: false,
            lsp_saved: false,
            load_rx: None,
            save_rx: None,
            restore: None,
            top_line: None,
        }
    }

    /// Unsaved changes: the text differs from the saved text, or a
    /// pending encoding / line-break conversion. Typing costs O(1): only
    /// an edit that restores the saved length hashes the buffer.
    pub fn dirty(&self) -> bool {
        let Some(doc) = self.doc.as_ref() else {
            return false;
        };
        if self.format() != self.saved_format {
            return true;
        }
        if doc.revision() == self.saved_revision {
            return false;
        }
        if doc.len_chars() != self.saved_len {
            return true;
        }
        let (revision, dirty) = self.dirty_memo.get();
        if revision == doc.revision() {
            return dirty;
        }
        let dirty = doc.content_hash() != self.saved_hash;
        self.dirty_memo.set((doc.revision(), dirty));
        dirty
    }

    /// Encoding and whether saves convert breaks to CRLF: what
    /// `saved_format` records.
    pub(super) fn format(&self) -> (TextEncoding, bool) {
        (self.encoding, self.line_ending == LineEnding::Crlf)
    }

    /// Large-file mode: no highlighting, folds or underlines.
    pub(super) fn large_file(&self) -> bool {
        !self.extras.large_override
            && self
                .doc
                .as_ref()
                .is_some_and(|doc| doc.len_bytes() > LARGE_FILE_BYTES)
    }

    /// Indentation the buffer formats with: its indent style (chosen at
    /// load from `.editorconfig`, the text, or the language).
    pub(super) fn format_options(&self) -> native_lsp::FormatOptions {
        self.doc
            .as_ref()
            .map(|doc| doc.indent_style().format_options())
            .unwrap_or_default()
    }
}

pub(super) type GutterCache = gutter::GutterState;

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

#[cfg(test)]
mod line_ending_tests {
    use super::*;

    #[test]
    fn crlf_document_disk_text_matches_crlf_file_bytes() {
        let mut doc = NativeDoc::loaded(PathBuf::from("crlf.txt"), "one\ntwo\n");
        doc.line_ending = LineEnding::Crlf;
        assert_eq!(doc.disk_text().as_deref(), Some("one\r\ntwo\r\n"));
    }

    #[test]
    fn lf_document_disk_text_keeps_lf() {
        let doc = NativeDoc::loaded(PathBuf::from("lf.txt"), "one\ntwo\n");
        assert_eq!(doc.disk_text().as_deref(), Some("one\ntwo\n"));
    }
}
