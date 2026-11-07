//! The working-tree side of an editable diff as a real native editor: its
//! buffer, the live re-diff of the left text against that buffer, and the
//! filler rows and tints that keep the editor aligned with the left pane.
use super::*;
use crate::native_editor::LineEnding;
use eframe::egui::Color32;
use std::sync::{Arc, mpsc};
use terminator_core::async_service::CancellationToken;
use terminator_native_edit::doc::{Buffer, Doc};
use terminator_native_edit::highlight::HighlightCache;
use terminator_native_edit::view::{LineFill, RangeFill};
use terminator_native_edit::vim::VimEngine;

/// A buffer up to this size builds its first live diff on the frame that
/// shows it (no spinner); every later re-diff runs on the shared CPU pool.
const SYNC_BYTES: usize = 16 * 1024;
/// Typing pause before a small buffer re-diffs, in seconds.
const SMALL_DEBOUNCE: f64 = 0.03;
/// Typing pause before a large buffer re-diffs, in seconds.
const DEBOUNCE: f64 = 0.15;
/// Pause before a build that ended without a result runs again, in seconds.
const RETRY_PAUSE: f64 = 1.0;

/// The native editor state behind the diff's right pane.
pub(crate) struct LiveEditor {
    pub doc: Doc,
    pub engine: VimEngine,
    pub highlight: HighlightCache,
    /// The file's line breaks: uniform CRLF edits as LF and saves as CRLF.
    line_ending: LineEnding,
    /// Buffer revision `Edit::text` mirrors.
    synced: u64,
    /// When the buffer last changed (UI time), for the re-diff debounce.
    edited_at: f64,
}

impl LiveEditor {
    pub fn new(text: &str) -> Self {
        let (text, line_ending) = LineEnding::decode(text.to_owned());
        let doc = Doc::new(text);
        Self {
            synced: doc.revision(),
            doc,
            engine: VimEngine::new(),
            highlight: HighlightCache::default(),
            line_ending,
            edited_at: 0.0,
        }
    }

    /// The buffer as it would be saved.
    pub fn text(&self) -> String {
        self.line_ending.encode(self.doc.text())
    }
}

/// A live diff document and the build that will replace it.
#[derive(Default)]
pub(crate) struct LiveDiff {
    /// `(buffer revision, document)` currently shown.
    pub shown: Option<(u64, Arc<DiffDocument>)>,
    /// The one pool build in flight, for one revision. Replacing it
    /// cancels the old build, and its late result is dropped with the
    /// receiver.
    pending: Option<Pending>,
    /// Editor marks for `(document generation, palette)`.
    marks: Option<(DiffGeneration, MarkColors, Arc<EditorMarks>)>,
    /// UI time before which a build that ended without a result is not
    /// started again.
    retry_at: f64,
}

impl review::Edit {
    /// Mirror the editor buffer into `text` after an edit.
    pub fn sync_text(&mut self, now: f64) {
        let revision = self.editor.doc.revision();
        if revision != self.editor.synced {
            self.editor.synced = revision;
            self.editor.edited_at = now;
            self.text = self.editor.text();
        }
    }

    /// The left text diffed against the buffer with every line shown, for
    /// the current revision when it is ready; the previous one while a
    /// large buffer re-diffs. A large buffer's first build also runs on a
    /// worker (see [`review::Edit::live_loading`]). `true` in the second
    /// value asks for another frame.
    pub fn live_document(
        &mut self,
        path: &Path,
        now: f64,
        pool: &crate::gui_services::Submit,
    ) -> (Arc<DiffDocument>, bool) {
        let revision = self.editor.synced;
        let live = &mut self.live;
        if let Some(pending) = &live.pending {
            match pending.receiver.try_recv() {
                Ok(doc) => {
                    live.shown = Some((pending.revision, Arc::new(doc)));
                    live.pending = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                // The build ended without a document (timed out, panicked,
                // replaced): build again after a pause.
                Err(mpsc::TryRecvError::Disconnected) => {
                    live.pending = None;
                    live.retry_at = now + RETRY_PAUSE;
                }
            }
        }
        let current = live.shown.as_ref().is_some_and(|(at, _)| *at == revision);
        let small = self
            .text
            .len()
            .saturating_add(self.document.left_text.len())
            <= SYNC_BYTES;
        let mut waiting = false;
        if !current && small && live.shown.is_none() {
            live.shown = Some((
                revision,
                Arc::new(build_live(&self.document, &self.text, path)),
            ));
        } else if !current {
            // Nothing shown yet: build at once, without the typing pause.
            let pause = if small { SMALL_DEBOUNCE } else { DEBOUNCE };
            let idle = (live.shown.is_none() || now - self.editor.edited_at >= pause)
                && now >= live.retry_at;
            let building = live
                .pending
                .as_ref()
                .is_some_and(|p| p.revision == revision);
            if idle && !building {
                live.pending = spawn_build(SpawnBuildInput {
                    pool,
                    document: &self.document,
                    text: &self.text,
                    path,
                    revision,
                });
            }
            waiting = true;
        }
        let shown = live
            .shown
            .as_ref()
            .map_or_else(|| Arc::clone(&self.document), |(_, doc)| Arc::clone(doc));
        (shown, waiting)
    }

    /// No live document is built yet: a large buffer's first build is
    /// still on its worker.
    pub fn live_loading(&self) -> bool {
        self.live.shown.is_none()
    }

    /// Filler rows and tints for the editor beside `doc`, cached per
    /// document and palette.
    pub fn editor_marks(&mut self, doc: &DiffDocument, colors: MarkColors) -> Arc<EditorMarks> {
        if let Some((generation, cached_colors, marks)) = &self.live.marks
            && *generation == doc.generation
            && *cached_colors == colors
        {
            return Arc::clone(marks);
        }
        let marks = Arc::new(editor_marks(doc, colors));
        self.live.marks = Some((doc.generation, colors, Arc::clone(&marks)));
        marks
    }
}

#[cfg(test)]
impl LiveDiff {
    /// A build is in flight.
    pub(crate) fn building(&self) -> bool {
        self.pending.is_some()
    }

    /// Make the build in flight end without a document, as a timed-out
    /// or panicked pool job does.
    pub(crate) fn drop_build_result(&mut self) {
        if let Some(pending) = &mut self.pending {
            pending.receiver = mpsc::channel().1;
        }
    }
}

/// One live diff build on the shared CPU pool.
struct Pending {
    revision: u64,
    receiver: mpsc::Receiver<DiffDocument>,
    cancel: CancellationToken,
}

impl Drop for Pending {
    /// A replaced or abandoned build is cancelled; a queued one never runs.
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

struct SpawnBuildInput<'a> {
    pool: &'a crate::gui_services::Submit,
    document: &'a Arc<DiffDocument>,
    text: &'a str,
    path: &'a Path,
    revision: u64,
}

/// Queue a build on the shared CPU pool. `None` when the pool refused it
/// (overloaded or closed); the caller retries on a later frame.
fn spawn_build(input: SpawnBuildInput<'_>) -> Option<Pending> {
    use terminator_core::async_service::{OperationContext, Policy};
    let SpawnBuildInput {
        pool,
        document,
        text,
        path,
        revision,
    } = input;
    let (sender, receiver) = mpsc::channel();
    let cancel = CancellationToken::new();
    let (document, text, path) = (Arc::clone(document), text.to_owned(), path.to_owned());
    let cpu = pool.cpu().clone();
    let token = cancel.clone();
    let context = OperationContext::new(
        "diff-live",
        path.display().to_string(),
        Policy::ReplaceableRead,
    );
    pool.handle()
        .submit(context, cancel.clone(), async move {
            let built = cpu
                .run(&token, move || {
                    anyhow::Ok(build_live(&document, &text, &path))
                })
                .await?;
            let _ = sender.send(built);
            Ok(Vec::new())
        })
        .ok()?;
    Some(Pending {
        revision,
        receiver,
        cancel,
    })
}

/// `document.left_text` against `text`, every unchanged line shown (the
/// editor holds the whole file), under the document's compare options.
pub(crate) fn build_live(document: &DiffDocument, text: &str, path: &Path) -> DiffDocument {
    let view = DiffView {
        context: usize::MAX / 4,
        revealed: Vec::new(),
        revealed_in: None,
        options: document.options,
    };
    let mut doc = build_with_options(BuildWithOptionsInput {
        path,
        left: &document.left_text,
        right: text,
        staged: false,
        view: &view,
    });
    if doc.split.is_empty() {
        // No change at all: every left line pairs with its buffer line.
        let highlighted = highlight_file(path, &doc.left_text);
        let right: Vec<&str> = text.split_inclusive('\n').collect();
        for (index, line) in doc.left_text.split_inclusive('\n').enumerate() {
            let number = Some(u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1));
            let side = |spans| DiffLine {
                kind: LineKind::Equal,
                old_no: number,
                new_no: number,
                spans,
                moved: false,
                end: LineEnd::default(),
            };
            doc.split.push(SplitRow {
                left: Some(side(line_spans(&highlighted, Some(index), line))),
                right: Some(side(line_spans(
                    &[],
                    Some(index),
                    right.get(index).copied().unwrap_or_default(),
                ))),
            });
        }
    }
    doc.left_label.clone_from(&document.left_label);
    doc.right_label.clone_from(&document.right_label);
    doc
}

/// Colors the editor marks are painted in: the change colors and the
/// row / word tint alphas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MarkColors {
    pub added: Color32,
    pub modified: Color32,
    pub row: u8,
    pub word: u8,
}

/// What the editor pane paints beside the left pane of a live diff.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct EditorMarks {
    /// `(buffer line, rows)`: fillers where the left side has rows the
    /// buffer does not (deleted lines, longer replaced blocks).
    pub spacers: Arc<Vec<(usize, usize)>>,
    pub line_fills: Arc<Vec<LineFill>>,
    pub range_fills: Arc<Vec<RangeFill>>,
}

/// Map the right side of `doc`'s split rows onto buffer lines: every
/// right-side filler becomes an editor filler row before the next buffer
/// line, every inserted line a row tint, and every changed word a range
/// tint. Replaced blocks use the modified color, like the painted panes.
pub(crate) fn editor_marks(doc: &DiffDocument, colors: MarkColors) -> EditorMarks {
    let tint = |color: Color32, alpha: u8| {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
    };
    let highlight = doc.options.highlight != Highlight::None;
    let (mut spacers, mut line_fills, mut range_fills) = (Vec::new(), Vec::new(), Vec::new());
    let mut pending = 0usize;
    for (row, split) in doc.split.iter().enumerate() {
        let Some(line) = split
            .right
            .as_ref()
            .filter(|line| line.kind != LineKind::Hunk)
        else {
            pending = pending.saturating_add(1);
            continue;
        };
        let Some(index) = line
            .new_no
            .and_then(|n| usize::try_from(n).ok()?.checked_sub(1))
        else {
            continue;
        };
        if pending > 0 {
            spacers.push((index, std::mem::take(&mut pending)));
        }
        if !highlight || line.kind != LineKind::Insert {
            continue;
        }
        let replaced = doc
            .change_at(row, true)
            .is_some_and(review::Hunk::replacement);
        let color = if replaced {
            colors.modified
        } else {
            colors.added
        };
        line_fills.push(LineFill {
            line: index,
            color: tint(color, colors.row),
        });
        let mut col = 0usize;
        for span in &line.spans {
            let end = col.saturating_add(span.text.chars().count());
            if span.intra == Intra::Change && end > col {
                range_fills.push(RangeFill {
                    line: index,
                    start: col,
                    end,
                    color: tint(color, colors.word),
                });
            }
            col = end;
        }
    }
    if pending > 0 {
        let lines = doc.right_text.split_inclusive('\n').count();
        spacers.push((lines, pending));
    }
    EditorMarks {
        spacers: Arc::new(spacers),
        line_fills: Arc::new(line_fills),
        range_fills: Arc::new(range_fills),
    }
}
