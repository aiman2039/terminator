//! Revision-keyed highlight cache shared by a buffer's views.
use super::{
    Language, MAX_HIGHLIGHT_BYTES, SYNC_HIGHLIGHT_BYTES, Span, highlight_lf_kept, rainbow,
};

/// The buffer's last highlighted text and syntax tree, shared with the
/// highlight worker, so an edit re-parses only what changed.
type Syntax = std::sync::Arc<std::sync::Mutex<Option<crate::treesitter::SyntaxState>>>;
use crate::doc::Buffer;
use crate::doc::normalize_breaks;
use crate::rows::{RowCache, VisualRow};

/// Revision-keyed highlight cache: one per open buffer, owned by the app
/// next to the buffer it derives from. The row layout lives here too: the
/// app owns exactly one cache per buffer, and both derivations share the
/// same revision key.
#[derive(Default)]
pub struct HighlightCache {
    language: Option<Language>,
    dark: bool,
    revision: u64,
    ready: bool,
    rainbow: bool,
    /// Highlight even over [`MAX_HIGHLIGHT_BYTES`] (large-file mode
    /// turned off by the user).
    unlimited: bool,
    /// Shared with the other views of the same buffer (see
    /// [`HighlightCache::share_spans`]).
    lines: std::sync::Arc<Vec<Vec<Span>>>,
    rows: RowCache,
    folds: crate::fold::Folds,
    /// The worker highlight in flight, if any (see [`HighlightCache::ensure_async`]).
    running: Option<(HighlightKey, std::sync::mpsc::Receiver<Vec<Vec<Span>>>)>,
    /// Kept text and tree for incremental parsing (see [`Syntax`]).
    syntax: Syntax,
}

/// What one highlight result was computed for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct HighlightKey {
    language: Language,
    dark: bool,
    revision: u64,
    rainbow: bool,
}

/// Spans for `text`, with rainbow brackets when asked; empty when plain.
fn compute_lines_capped(
    language: Language,
    text: &str,
    (dark, rainbow): (bool, bool),
    (cap, syntax): (usize, &Syntax),
) -> Vec<Vec<Span>> {
    if text.len() > cap {
        return Vec::new();
    }
    let text = &*normalize_breaks(text);
    let mut kept = syntax
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match highlight_lf_kept(language, text, dark, Some(&mut kept)) {
        Some(mut lines) => {
            if rainbow {
                rainbow::apply(&mut lines, text, dark);
            }
            lines
        }
        None => Vec::new(),
    }
}

pub struct EnsureInput<'a> {
    pub language: Language,
    pub dark: bool,
    pub revision: u64,
    /// Owned text moves to the worker without a copy.
    pub text: std::borrow::Cow<'a, str>,
}

impl HighlightCache {
    /// True when the cache already holds this exact revision: the view
    /// can skip rebuilding the buffer text entirely.
    #[must_use]
    pub fn is_current(&self, language: Language, dark: bool, revision: u64) -> bool {
        self.ready
            && self.language == Some(language)
            && self.dark == dark
            && self.revision == revision
    }

    /// Color brackets by nesting depth. A change recomputes the spans.
    pub fn set_rainbow(&mut self, rainbow: bool) {
        if self.rainbow != rainbow {
            self.rainbow = rainbow;
            self.ready = false;
        }
    }

    /// Recompute when the key changed; over-cap and plain buffers cache
    /// as empty (every row paints plain).
    pub fn ensure(&mut self, input: EnsureInput<'_>) {
        let EnsureInput {
            language,
            dark,
            revision,
            text,
        } = input;
        if self.is_current(language, dark, revision) {
            return;
        }
        self.running = None;
        self.lines = std::sync::Arc::new(compute_lines_capped(
            language,
            &text,
            (dark, self.rainbow),
            (self.cap(), &self.syntax),
        ));
        self.adopt(HighlightKey {
            language,
            dark,
            revision,
            rainbow: self.rainbow,
        });
    }

    fn adopt(&mut self, key: HighlightKey) {
        self.language = Some(key.language);
        self.dark = key.dark;
        self.revision = key.revision;
        self.ready = key.rainbow == self.rainbow;
    }

    /// Take a finished worker result. A result for an older revision is
    /// still painted (closer than nothing) but does not mark the cache
    /// current, so the next frame starts the newest revision.
    fn poll(&mut self) {
        let Some((key, rx)) = self.running.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(lines) => {
                let key = *key;
                self.running = None;
                self.lines = std::sync::Arc::new(lines);
                self.adopt(key);
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.running = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    /// True when the caller must hand this revision's text to
    /// [`HighlightCache::ensure_async`]: the cache is not current and no
    /// worker is running. While one runs, the caller skips building the
    /// text; edits made meanwhile highlight when it lands, newest only.
    pub fn wants_text(&mut self, language: Language, dark: bool, revision: u64) -> bool {
        self.poll();
        !self.is_current(language, dark, revision) && self.running.is_none()
    }

    /// [`HighlightCache::ensure`], but text over [`SYNC_HIGHLIGHT_BYTES`]
    /// is highlighted on a worker thread; `ctx` repaints when it lands.
    /// Until then the last spans stay painted.
    pub fn ensure_async(&mut self, input: EnsureInput<'_>, ctx: &egui::Context) {
        if input.text.len() <= SYNC_HIGHLIGHT_BYTES || input.text.len() > self.cap() {
            self.ensure(input);
            return;
        }
        self.poll();
        if self.is_current(input.language, input.dark, input.revision) || self.running.is_some() {
            return;
        }
        let key = HighlightKey {
            language: input.language,
            dark: input.dark,
            revision: input.revision,
            rainbow: self.rainbow,
        };
        let ctx = ctx.clone();
        let cap = self.cap();
        let syntax = std::sync::Arc::clone(&self.syntax);
        let (tx, rx) = std::sync::mpsc::channel();
        // The text follows the spawn, so a failed spawn still has it.
        let (text_tx, text_rx) = std::sync::mpsc::channel::<String>();
        let spawned = std::thread::Builder::new()
            .name("highlight".into())
            .spawn(move || {
                let Ok(text) = text_rx.recv() else {
                    return;
                };
                let lines = compute_lines_capped(
                    key.language,
                    &text,
                    (key.dark, key.rainbow),
                    (cap, &syntax),
                );
                let _ = tx.send(lines);
                ctx.request_repaint();
            });
        if spawned.is_ok() {
            let _ = text_tx.send(input.text.into_owned());
            self.running = Some((key, rx));
        } else {
            self.ensure(input);
        }
    }

    /// Size cap for this cache's highlighting.
    fn cap(&self) -> usize {
        if self.unlimited {
            usize::MAX
        } else {
            MAX_HIGHLIGHT_BYTES
        }
    }

    /// Highlight (and fold) past the large-file cap. A change re-derives.
    pub fn set_unlimited(&mut self, unlimited: bool) {
        if self.unlimited != unlimited {
            self.unlimited = unlimited;
            self.ready = false;
        }
    }

    /// Whether the text needed for this revision is worth building: false
    /// for a buffer of `bytes` over the cap, which paints plain anyway.
    /// Marks such a cache current (empty spans) so the next frames skip
    /// it as well.
    pub fn skip_large(
        &mut self,
        (language, dark): (Language, bool),
        revision: u64,
        bytes: usize,
    ) -> bool {
        if bytes <= self.cap() {
            return false;
        }
        self.running = None;
        self.lines = std::sync::Arc::default();
        self.adopt(HighlightKey {
            language,
            dark,
            revision,
            rainbow: self.rainbow,
        });
        true
    }

    /// Take the spans of `other` (another view of the same buffer) when
    /// they were computed for exactly what this view needs: split panes
    /// then highlight once per edit, not once per pane.
    pub fn share_spans(&mut self, other: &HighlightCache) {
        let Some(language) = other.language else {
            return;
        };
        if other.ready
            && other.rainbow == self.rainbow
            && other.unlimited == self.unlimited
            && !self.is_current(language, other.dark, other.revision)
        {
            self.running = None;
            self.lines = std::sync::Arc::clone(&other.lines);
            self.adopt(HighlightKey {
                language,
                dark: other.dark,
                revision: other.revision,
                rainbow: other.rainbow,
            });
        }
    }

    /// Highlight spans for one row; empty when plain or out of range.
    #[must_use]
    pub fn line(&self, line: usize) -> &[Span] {
        match self.lines.get(line) {
            Some(spans) => spans,
            None => &[],
        }
    }

    /// Lay out the painted rows for this revision and wrap width, reusing
    /// the previous frame's rows until either changes.
    pub fn ensure_rows<B: Buffer>(&mut self, doc: &B, wrap_columns: Option<usize>) {
        self.ensure_rows_spaced(doc, wrap_columns, &[]);
    }

    /// [`HighlightCache::ensure_rows`] with host filler rows before some
    /// lines (see [`crate::spacers`]).
    pub fn ensure_rows_spaced<B: Buffer>(
        &mut self,
        doc: &B,
        wrap_columns: Option<usize>,
        spacers: &[(usize, usize)],
    ) {
        // Large files skip the full fold scan unless the user opted in.
        let foldable = doc.len_chars() <= self.cap();
        self.folds
            .ensure_enabled(doc, self.language.unwrap_or(Language::Plain), foldable);
        self.rows.ensure_spaced(
            doc,
            crate::rows::RowLayout {
                wrap_columns,
                folds: Some(&self.folds),
                spacers,
            },
        );
    }

    /// This view's code folds.
    #[must_use]
    pub fn folds(&self) -> &crate::fold::Folds {
        &self.folds
    }

    pub fn folds_mut(&mut self) -> &mut crate::fold::Folds {
        &mut self.folds
    }

    /// Fold the regions that start on `starts` (a remembered view) before
    /// the first frame: this revision's regions are found first, for
    /// `language`. Starts that begin no region are ignored, and large
    /// files get no folds, as in [`HighlightCache::ensure_rows_spaced`].
    pub fn restore_folds<B: Buffer>(&mut self, doc: &B, language: Language, starts: &[usize]) {
        let foldable = doc.len_chars() <= self.cap();
        self.folds.ensure_enabled(doc, language, foldable);
        self.folds.restore(starts);
    }

    /// A cache for a new view of the same buffer that starts with this
    /// view's folds (a split, or the file opened again).
    #[must_use]
    pub fn fork_folds(&self) -> Self {
        Self {
            folds: self.folds.clone(),
            ..Self::default()
        }
    }

    /// Rows laid out by the last [`HighlightCache::ensure_rows`].
    #[must_use]
    pub fn rows(&self) -> &[VisualRow] {
        self.rows.rows()
    }

    /// Widest laid-out row in display cells, for the horizontal extent.
    #[must_use]
    pub fn max_cells(&self) -> usize {
        self.rows.max_cells()
    }
}
