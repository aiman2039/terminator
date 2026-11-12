//! Disk sync for native buffers: background loads (with retry and
//! undo-preserving reloads), modification-time probes, and saves.
use super::doc::{LoadResult, MTIME_TTL, NativeDoc, SaveResult, vimrc_options};
use super::file_io::{Fingerprint, LineEnding, LoadedFile, NativeFileIo, SaveError, SaveRequest};
use crate::gui_services;
use std::ops::Range;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use terminator_native_edit::doc::{Buffer, Doc};
use terminator_native_edit::indent::{EditorConfig, IndentStyle};
use terminator_native_edit::vim::{Key as VimKey, ModalEngine};
use tokio::sync::mpsc::{UnboundedReceiver, error::TryRecvError};

/// Failed loads retry this many times, doubling the wait each time.
const MAX_RETRIES: u32 = 8;
const FIRST_RETRY: Duration = Duration::from_millis(250);
const MAX_RETRY_WAIT: Duration = Duration::from_secs(5);

/// One replacement of the buffer: chars `range` become `text`.
pub(super) type TextEdit = (Range<usize>, String);

/// A finished load: the file, and for a reload the edits that turn the
/// buffer at `revision` into the disk text.
pub(super) struct Loaded {
    pub(super) file: LoadedFile,
    pub(super) line_ending: LineEnding,
    pub(super) edits: Option<(u64, Vec<TextEdit>)>,
    /// The file's `.editorconfig` settings and its resolved indentation.
    pub(super) config: EditorConfig,
    pub(super) indent: IndentStyle,
}

/// Disk-side state of one buffer beyond its modification time.
#[derive(Default)]
pub(crate) struct DiskSync {
    /// Size and hash of the bytes on disk as last loaded or saved.
    pub(super) fingerprint: Option<Fingerprint>,
    /// A reload of the loaded buffer is wanted (disk changed, or Reload).
    pub(super) reload_wanted: bool,
    /// The next reload replaces unsaved changes too (explicit Reload).
    pub(super) reload_discards: bool,
    /// Disk modification time a reload was last requested for, so one
    /// change asks once even while the file stays missing.
    pub(super) reload_seen: Option<Stamp>,
    /// When the next attempt of a failed load is due, and the count.
    pub(super) retry: Option<(Instant, u32)>,
    /// The last reload failed every retry (file gone, unreadable).
    pub(super) missing: Option<String>,
    /// "Keep mine" on the conflict bar, for this disk modification time.
    pub(super) keep_mine: Option<Stamp>,
}

/// A disk modification time as read (`None`: the file was missing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stamp(pub(super) Option<SystemTime>);

/// Run `job` on the file worker pool; the receiver gets its result.
/// `None` when the pool did not take the job.
pub(super) fn run_on_files<T: Send + 'static>(
    services: &gui_services::Services,
    (name, key, policy): (&'static str, &Path, Policy),
    job: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Option<UnboundedReceiver<anyhow::Result<T>>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let service = services.clone();
    let context = OperationContext::new(name, key.to_string_lossy().into(), policy);
    services
        .handle()
        .submit(context, CancellationToken::new(), async move {
            let token = CancellationToken::new();
            let _ = tx.send(service.fs().run(&token, job).await);
            Ok(Vec::new())
        })
        .is_ok()
        .then_some(rx)
}

/// Read `path` and, for a reload, diff it against `base` (the buffer
/// text at a revision): a line diff turned into char-range edits. With
/// `editorconfig`, the file's `.editorconfig` is read here too, so the
/// GUI thread never walks the folders above the file.
fn load(
    path: &Path,
    (limit, new_file): (u64, bool),
    base: Option<(u64, String)>,
    editorconfig: bool,
) -> anyhow::Result<Loaded> {
    let mut file = NativeFileIo::read_or_new(path, limit, new_file)?;
    let (text, line_ending) = LineEnding::decode(std::mem::take(&mut file.text));
    file.text = text;
    let config = if editorconfig {
        EditorConfig::for_file(path)
    } else {
        EditorConfig::default()
    };
    let indent = config.indent(path, &file.text, IndentStyle::from_vimrc(vimrc_options()));
    let line_ending = line_ending.or_configured(&file.text, config.end_of_line);
    let edits = base.map(|(revision, old)| (revision, text_edits(&old, &file.text)));
    Ok(Loaded {
        file,
        line_ending,
        edits,
        config,
        indent,
    })
}

/// Char-range edits turning `old` into `new`, last first (each edit
/// leaves the positions of the ones after it valid).
pub(super) fn text_edits(old: &str, new: &str) -> Vec<TextEdit> {
    let diff = similar::TextDiff::configure()
        .timeout(crate::diff::DIFF_TIMEOUT)
        .diff_lines(old, new);
    let mut offsets = vec![0usize];
    for slice in diff.iter_old_slices() {
        let last = offsets.last().copied().unwrap_or(0);
        offsets.push(last.saturating_add(slice.chars().count()));
    }
    let mut edits = Vec::new();
    for op in diff.ops().iter().rev() {
        let (tag, old_range, new_range) = op.as_tag_tuple();
        if tag == similar::DiffTag::Equal {
            continue;
        }
        let start = offsets.get(old_range.start).copied().unwrap_or(0);
        let end = offsets.get(old_range.end).copied().unwrap_or(start);
        let text: String = new_range.filter_map(|at| diff.new_slice(at)).collect();
        edits.push((start..end, text));
    }
    edits
}

/// Apply [`text_edits`] as one undo step: undo restores the old text.
pub(super) fn apply_edits(doc: &mut Doc, edits: &[TextEdit]) {
    doc.end_run();
    doc.begin_undo_group();
    for (range, text) in edits {
        doc.delete(range.clone());
        doc.insert(range.start, text);
    }
    doc.end_undo_group();
}

impl NativeDoc {
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
        self.last_known_mtime()
    }

    /// The last disk modification time read, else the loaded one.
    pub(super) fn last_known_mtime(&self) -> Option<SystemTime> {
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
            Ok(Ok(mtime)) => {
                self.mtime_check = Some((Instant::now(), mtime));
                self.mtime_rx = None;
            }
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => self.mtime_rx = None,
            Err(TryRecvError::Empty) => {}
        }
    }

    /// Forget the last reading after a load or save moved `loaded_mtime`;
    /// a stat still in flight may predate the write.
    pub(super) fn reset_mtime(&mut self) {
        self.mtime_check = None;
        self.mtime_rx = None;
    }

    pub(super) fn start_mtime_probe(&mut self, services: &gui_services::Services) {
        let path = self.path.clone();
        let started = run_on_files(
            services,
            ("native-mtime", &self.path, Policy::ReplaceableRead),
            move || Ok(NativeFileIo::disk_mtime(&path)),
        );
        if started.is_some() {
            self.mtime_rx = started;
        } else {
            // Busy pool: keep the last reading and retry after the TTL.
            self.mtime_check = Some((Instant::now(), self.last_known_mtime()));
        }
    }

    /// Ask for a reload of the loaded buffer; `discard` also replaces
    /// unsaved changes (an explicit Reload).
    pub(super) fn request_reload(&mut self, discard: bool) {
        self.disk.reload_wanted = true;
        self.disk.reload_discards |= discard;
        self.disk.retry = None;
        self.disk.missing = None;
        self.load_error = None;
    }

    /// Start a load when one is due: the first load, a wanted reload, or
    /// the next retry of a failed one.
    pub(super) fn ensure_loading(&mut self, services: &gui_services::Services) {
        if self.loading || self.saving {
            return;
        }
        if self.doc.is_some() && !self.disk.reload_wanted {
            return;
        }
        match self.disk.retry {
            Some((due, _)) if Instant::now() < due => return,
            // Retries used up: wait for an explicit Reload.
            None if self.load_error.is_some() => return,
            _ => {}
        }
        let path = self.path.clone();
        let (limit, editorconfig) = (self.read_limit, self.read_editorconfig);
        let new_file = self.new_file && self.doc.is_none();
        let base = self.doc.as_ref().map(|doc| (doc.revision(), doc.text()));
        match run_on_files(
            services,
            ("native-load", &self.path, Policy::ReplaceableRead),
            move || load(&path, (limit, new_file), base, editorconfig),
        ) {
            Some(rx) => {
                self.load_rx = Some(rx);
                self.loading = true;
            }
            None => self.load_failed("Could not start the file read".into()),
        }
    }

    pub(super) fn poll(&mut self) {
        if let Some(rx) = self.load_rx.as_mut() {
            match rx.try_recv() {
                Ok(result) => self.settle_load(result),
                Err(TryRecvError::Disconnected) => {
                    self.settle_load(Err(anyhow::anyhow!("The file read stopped")));
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(received) = self.save_rx.as_mut().map(UnboundedReceiver::try_recv) {
            self.settle_save(received);
        }
    }

    fn settle_load(&mut self, result: LoadResult) {
        self.load_rx = None;
        self.loading = false;
        match result {
            Ok(loaded) => self.adopt_loaded(loaded),
            Err(error) => self.load_failed(format!("{error:#}")),
        }
    }

    /// Schedule the next retry; a buffer already loaded stays as it is.
    fn load_failed(&mut self, error: String) {
        let attempts = self.disk.retry.map_or(0, |(_, n)| n).saturating_add(1);
        if attempts <= MAX_RETRIES {
            let wait = FIRST_RETRY
                .saturating_mul(1u32 << attempts.min(5).saturating_sub(1))
                .min(MAX_RETRY_WAIT);
            let due = Instant::now()
                .checked_add(wait)
                .unwrap_or_else(Instant::now);
            self.disk.retry = Some((due, attempts));
        } else {
            self.disk.retry = None;
            self.disk.reload_wanted = false;
            self.disk.reload_discards = false;
        }
        if self.doc.is_some() {
            self.disk.missing = self.disk.retry.is_none().then_some(error);
        } else {
            self.load_error = Some(error);
        }
    }

    fn adopt_loaded(&mut self, loaded: Loaded) {
        let Loaded {
            file,
            line_ending,
            edits,
            config,
            indent,
        } = loaded;
        let discard = std::mem::take(&mut self.disk.reload_discards);
        self.disk.reload_wanted = false;
        self.disk.retry = None;
        self.disk.missing = None;
        if self.doc.is_some() && self.dirty() && !discard {
            // Never replace unsaved edits behind the user's back.
            return;
        }
        match (self.doc.as_mut(), edits) {
            (Some(doc), Some((revision, edits))) if doc.revision() == revision => {
                apply_edits(doc, &edits);
            }
            (Some(_), _) => {
                // Edited while the read ran: read again against the new text.
                self.disk.reload_wanted = true;
                self.disk.reload_discards = discard;
                return;
            }
            (None, _) => self.doc = Some(Doc::new(file.text.as_str())),
        }
        if let Some(doc) = self.doc.as_mut() {
            doc.set_indent_style(indent);
        }
        self.file_config = config;
        self.loaded_mtime = file.mtime;
        self.reset_mtime();
        self.disk.fingerprint = file.fingerprint;
        self.disk.reload_seen = Some(Stamp(file.mtime));
        self.line_ending = line_ending;
        self.encoding = file.encoding;
        self.saved_format = self.format();
        self.load_error = None;
        self.mark_saved_now();
        self.clamp_carets();
        if self.insert_first {
            self.insert_first = false;
            if let Some(doc) = self.doc.as_mut() {
                self.engine.press_key(doc, VimKey::Char('i'));
            }
        }
    }

    /// Record the buffer as matching the disk.
    pub(super) fn mark_saved_now(&mut self) {
        if let Some(doc) = self.doc.as_ref() {
            self.saved_revision = doc.revision();
            self.save_revision = self.saved_revision;
            self.saved_hash = doc.content_hash();
            self.saved_len = doc.len_chars();
            self.save_len = self.saved_len;
        }
    }

    /// Keep every view's caret inside a buffer a reload shortened.
    fn clamp_carets(&mut self) {
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        let engines = std::iter::once(&mut self.engine)
            .chain(self.views.values_mut().map(|(engine, _)| engine));
        for engine in engines {
            let cursor = engine.cursor();
            let line = cursor.line.min(doc.line_count().saturating_sub(1));
            let _ = engine.place_cursor(doc, line, cursor.col);
        }
    }

    /// Settle the in-flight save. A worker that stopped without an answer
    /// (cancelled or timed out) ends the save with an error, so Save and
    /// Save-and-close never wait forever.
    pub(super) fn settle_save(&mut self, received: Result<SaveResult, TryRecvError>) {
        let result = match received {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err(anyhow::anyhow!("The save stopped before it finished"))
            }
        };
        self.save_rx = None;
        self.saving = false;
        match result {
            Ok(saved) => {
                // The written text was the snapshot taken at save start:
                // edits typed while the write ran keep the buffer dirty.
                self.saved_revision = self.save_revision;
                self.saved_hash = saved.text_hash;
                self.saved_len = self.save_len;
                self.saved_format = self.save_format;
                self.loaded_mtime = saved.mtime;
                self.disk.fingerprint = Some(saved.fingerprint);
                self.disk.reload_seen = Some(Stamp(saved.mtime));
                self.disk.keep_mine = None;
                self.reset_mtime();
                self.save_error = None;
                self.force_save = false;
                self.lsp_saved = true;
            }
            Err(error) => {
                self.save_error = Some(SaveError::of(&error));
            }
        }
    }

    /// Revision and text to announce to the language servers as saved:
    /// once per finished write, after any reload it asked for landed (a
    /// rename rewrites the file on disk, then reloads the buffer).
    pub(super) fn take_lsp_saved(&mut self) -> Option<(u64, String)> {
        if !self.lsp_saved || self.loading || self.disk.reload_wanted {
            return None;
        }
        self.lsp_saved = false;
        self.doc.as_ref().map(|doc| (doc.revision(), doc.text()))
    }

    pub(super) fn start_save(&mut self, services: &gui_services::Services) {
        if self.saving || self.loading {
            return;
        }
        self.apply_save_rules();
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let text = doc.text();
        self.save_revision = doc.revision();
        self.save_len = doc.len_chars();
        self.save_format = self.format();
        let request = SaveRequest {
            path: self.path.clone(),
            text,
            line_ending: self.line_ending,
            encoding: self.encoding,
            expected: self.disk.fingerprint,
            force: self.force_save,
            accepted: self.disk.keep_mine,
        };
        self.save_error = None;
        // A write is a mutation: no read deadline cancels it half-way.
        match run_on_files(
            services,
            ("native-save", &self.path, Policy::OrderedMutation),
            move || NativeFileIo::save(request),
        ) {
            Some(rx) => {
                self.save_rx = Some(rx);
                self.saving = true;
            }
            None => self.save_error = Some(SaveError::Failed("Could not start the write".into())),
        }
    }

    /// The `.editorconfig` save rules, as one undo step: a final newline,
    /// and no trailing spaces or tabs except on lines with a caret on
    /// screen (the user may be typing there). A save never drops undone
    /// steps: while
    /// one waits to be redone (an autosave after Undo), the rules wait.
    /// They also wait while a vim Insert change is open, so they never
    /// join the typed change.
    fn apply_save_rules(&mut self) {
        if self
            .doc
            .as_ref()
            .is_none_or(|doc| doc.can_redo() || doc.undo_group_open())
        {
            return;
        }
        // Views that stopped drawing (closed, split away) keep no line.
        let shown = self
            .views
            .iter()
            .filter(|(id, _)| self.shows_view(id))
            .map(|(_, (engine, _))| engine);
        let engines = std::iter::once(&self.engine).chain(shown);
        let mut keep = Vec::new();
        for engine in engines {
            keep.push(engine.cursor().line);
            keep.extend(engine.extra_carets().iter().map(|caret| caret.cursor.line));
        }
        let Some(doc) = self.doc.as_mut() else {
            return;
        };
        let edits = self.file_config.save_edits(doc, &keep);
        if !edits.is_empty() {
            apply_edits(doc, &edits);
        }
    }

    /// The file changed on disk since it was loaded or saved (and the
    /// change is not one "Keep mine" already answered).
    pub(super) fn disk_conflict(&self, on_disk: Option<SystemTime>) -> bool {
        self.doc.is_some()
            && on_disk != self.loaded_mtime
            && self.disk.keep_mine != Some(Stamp(on_disk))
            && !self.saving
    }
}
