//! Per-tab review state. No repository writes run on the GUI thread.
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
    pub unified_row: usize,
    pub split_row: usize,
}

impl Hunk {
    pub fn row(&self, split: bool) -> usize {
        if split {
            self.split_row
        } else {
            self.unified_row
        }
    }

    pub fn rows(&self, split: bool) -> Range<usize> {
        let start = self.row(split);
        let len = if split {
            self.old.len().max(self.new.len())
        } else {
            self.old.len().saturating_add(self.new.len())
        };
        start..start.saturating_add(len)
    }

    pub fn replacement(&self) -> bool {
        !self.old.is_empty() && !self.new.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Gap {
    pub old: Range<usize>,
    pub new: Range<usize>,
    pub unified_row: usize,
    pub split_row: usize,
}

impl Gap {
    pub fn row(&self, split: bool) -> usize {
        if split {
            self.split_row
        } else {
            self.unified_row
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    /// Always-on working-tree buffer for the split diff's right pane. The
    /// diff view itself is the editor; there is no separate edit mode.
    pub buffer: Option<Edit>,
    pub saving_edits: bool,
    pub selected: Option<usize>,
    pub jump: bool,
    pub confirm_discard: Option<usize>,
    pub busy: bool,
    pub undo: Vec<super::actions::Undo>,
    pub context: usize,
    pub message: Option<String>,
    /// Compare lines ignoring whitespace (a view option; staging still
    /// applies the real hunks).
    pub ignore_whitespace: bool,
    /// Lines picked inside one hunk for line-level stage/unstage/discard.
    pub picked: Option<(usize, super::actions::LineSelection)>,
}

/// One row of the live unified editor, recomputed from the left text vs the
/// current buffer on every revision: the whole file, not just hunk windows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LiveRow {
    /// A line present in the buffer; `changed` marks insert/replace lines.
    Edit {
        old: Option<usize>,
        new: usize,
        changed: Option<LiveTint>,
    },
    /// A line only in the left text, painted read-only between buffer lines.
    Gone { old: usize, text: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LiveTint {
    Added,
    Changed,
}

/// `(key, per-line syntax spans, per-line span validity)`.
pub(crate) type SpanCache = (
    u64,
    std::sync::Arc<Vec<Vec<super::DiffSpan>>>,
    std::sync::Arc<Vec<bool>>,
);

pub(crate) struct Edit {
    pub document: std::sync::Arc<super::DiffDocument>,
    pub text: String,
    /// Syntax spans for `text`, keyed by a hash of the text and tint colors.
    /// The `bool` per line says the spans reproduce that line exactly, so the
    /// layouter never has to re-validate them on the hot path.
    pub right_spans: Option<SpanCache>,
    /// Diff tints recomputed live from `document.left_text` vs `text`,
    /// under the same key as `right_spans`.
    pub live_tints: Option<(u64, std::sync::Arc<Vec<Option<eframe::egui::Color32>>>)>,
    /// Buffer-level undo/redo for the unified editor (split uses the text
    /// widget's own undoer). Snapshots are whole-buffer text; `burst` is the
    /// last snapshot time so a typing burst is one undo unit.
    pub undo: Vec<String>,
    pub redo: Vec<String>,
    pub burst: f64,
    /// Live unified rows for the current revision, same key as `right_spans`.
    pub live_rows: Option<(u64, std::sync::Arc<Vec<LiveRow>>)>,
    /// Syntax spans for the immutable left text, computed once.
    pub left_spans: Option<std::sync::Arc<Vec<Vec<super::DiffSpan>>>>,
    /// The text changed since `key` was computed: only then is the whole
    /// buffer rehashed, instead of on every frame.
    pub stale: bool,
    /// Cache key of the current text plus tint palette.
    pub key: u64,
    /// Key of the tint palette alone, so a theme change rehashes too.
    pub palette: u64,
    /// Left pane aligned to the buffer's lines (editable split view),
    /// under the same key as `right_spans`.
    pub aligned_left: Option<(u64, std::sync::Arc<super::AlignedLeft>)>,
}

impl Edit {
    pub fn new(document: std::sync::Arc<super::DiffDocument>) -> Self {
        Self {
            text: document.right_text.clone(),
            document,
            right_spans: None,
            live_tints: None,
            live_rows: None,
            undo: Vec::new(),
            redo: Vec::new(),
            burst: 0.0,
            left_spans: None,
            stale: true,
            key: 0,
            palette: 0,
            aligned_left: None,
        }
    }
}

impl State {
    pub fn unsaved_edits(&self) -> bool {
        self.buffer
            .as_ref()
            .is_some_and(|edit| edit.text != edit.document.right_text)
    }
}

impl State {
    /// Take the picked lines when they belong to `hunk`.
    pub fn picked_lines(&mut self, hunk: usize) -> Option<super::actions::LineSelection> {
        match self.picked.take() {
            Some((picked_hunk, lines)) if picked_hunk == hunk && !lines.is_empty() => Some(lines),
            other => {
                self.picked = other;
                None
            }
        }
    }

    /// Toggle one changed line (`old` side or new side) of `hunk`; picks
    /// in another hunk start over.
    pub fn toggle_line(&mut self, hunk: usize, old: bool, line: usize) {
        let (picked_hunk, lines) = self
            .picked
            .get_or_insert_with(|| (hunk, super::actions::LineSelection::default()));
        if *picked_hunk != hunk {
            *picked_hunk = hunk;
            *lines = super::actions::LineSelection::default();
        }
        let set = if old { &mut lines.old } else { &mut lines.new };
        if !set.remove(&line) {
            set.insert(line);
        }
    }

    pub fn select(&mut self, index: usize) {
        self.selected = Some(index);
        self.jump = true;
        self.confirm_discard = None;
    }

    pub fn advance(&mut self, forward: bool, count: usize) {
        if count == 0 {
            return;
        }
        let index = match self.selected {
            None => {
                if forward {
                    0
                } else {
                    count.saturating_sub(1)
                }
            }
            Some(index) if forward => index.saturating_add(1).min(count.saturating_sub(1)),
            Some(index) => index.saturating_sub(1),
        };
        self.select(index);
    }
}
