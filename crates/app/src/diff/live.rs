//! The working-tree side of an editable diff as a real native editor: its
//! buffer, the live re-diff of the left text against that buffer, and the
//! filler rows and tints that keep the editor aligned with the left pane.
use super::*;
use eframe::egui::Color32;
use std::sync::{Arc, mpsc};
use terminator_native_edit::doc::{Buffer, Doc};
use terminator_native_edit::highlight::HighlightCache;
use terminator_native_edit::view::{LineFill, RangeFill};
use terminator_native_edit::vim::VimEngine;

/// Buffers up to this size re-diff on the frame that edited them; larger
/// ones re-diff on a worker thread once typing pauses.
const SYNC_BYTES: usize = 16 * 1024;
/// Typing pause before a large buffer re-diffs, in seconds.
const DEBOUNCE: f64 = 0.15;

/// The native editor state behind the diff's right pane.
pub(crate) struct LiveEditor {
    pub doc: Doc,
    pub engine: VimEngine,
    pub highlight: HighlightCache,
    /// The file is uniformly CRLF: it edits as LF and saves as CRLF.
    crlf: bool,
    /// Buffer revision `Edit::text` mirrors.
    synced: u64,
    /// When the buffer last changed (UI time), for the re-diff debounce.
    edited_at: f64,
}

impl LiveEditor {
    pub fn new(text: &str) -> Self {
        let lf = text.replace("\r\n", "\n");
        let crlf = lf.len() != text.len() && lf.replace('\n', "\r\n") == text;
        let doc = Doc::new(if crlf { lf } else { text.to_owned() });
        Self {
            synced: doc.revision(),
            doc,
            engine: VimEngine::new(),
            highlight: HighlightCache::default(),
            crlf,
            edited_at: 0.0,
        }
    }

    /// The buffer as it would be saved.
    pub fn text(&self) -> String {
        let text = self.doc.text();
        if self.crlf {
            text.replace('\n', "\r\n")
        } else {
            text
        }
    }
}

/// A live diff document and the build that will replace it.
#[derive(Default)]
pub(crate) struct LiveDiff {
    /// `(buffer revision, document)` currently shown.
    pub shown: Option<(u64, Arc<DiffDocument>)>,
    /// A worker build for one revision.
    pending: Option<(u64, mpsc::Receiver<DiffDocument>)>,
    /// Editor marks for `(document generation, palette)`.
    marks: Option<(DiffGeneration, MarkColors, Arc<EditorMarks>)>,
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
    /// large buffer re-diffs. `true` in the second value asks for another
    /// frame.
    pub fn live_document(&mut self, path: &Path, now: f64) -> (Arc<DiffDocument>, bool) {
        let revision = self.editor.synced;
        let live = &mut self.live;
        if let Some((built, receiver)) = &live.pending
            && let Ok(doc) = receiver.try_recv()
        {
            live.shown = Some((*built, Arc::new(doc)));
            live.pending = None;
        }
        let current = live.shown.as_ref().is_some_and(|(at, _)| *at == revision);
        let small = self
            .text
            .len()
            .saturating_add(self.document.left_text.len())
            <= SYNC_BYTES;
        let mut waiting = false;
        if !current && (small || live.shown.is_none()) {
            live.pending = None;
            live.shown = Some((
                revision,
                Arc::new(build_live(&self.document, &self.text, path)),
            ));
        } else if !current {
            let idle = now - self.editor.edited_at >= DEBOUNCE;
            let building = live.pending.as_ref().is_some_and(|(at, _)| *at == revision);
            if idle && !building {
                live.pending = Some((revision, spawn_build(&self.document, &self.text, path)));
            }
            waiting = true;
        }
        let shown = live
            .shown
            .as_ref()
            .map_or_else(|| Arc::clone(&self.document), |(_, doc)| Arc::clone(doc));
        (shown, waiting)
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

fn spawn_build(
    document: &Arc<DiffDocument>,
    text: &str,
    path: &Path,
) -> mpsc::Receiver<DiffDocument> {
    let (sender, receiver) = mpsc::channel();
    let (document, text, path) = (Arc::clone(document), text.to_owned(), path.to_owned());
    std::thread::spawn(move || {
        let _ = sender.send(build_live(&document, &text, &path));
    });
    receiver
}

/// `document.left_text` against `text`, every unchanged line shown (the
/// editor holds the whole file), under the document's compare options.
pub(crate) fn build_live(document: &DiffDocument, text: &str, path: &Path) -> DiffDocument {
    let view = DiffView {
        context: usize::MAX / 4,
        revealed: Vec::new(),
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
