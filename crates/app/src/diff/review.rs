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
    /// Compare options last requested for this tab, so a document built
    /// with older options is rebuilt once instead of on every frame.
    pub options_requested: Option<super::DiffOptions>,
    /// Lines picked inside one hunk for line-level stage/unstage/discard.
    pub picked: Option<(usize, super::actions::LineSelection)>,
}

/// The working-tree side of an editable diff: a native editor buffer.
pub(crate) struct Edit {
    pub document: std::sync::Arc<super::DiffDocument>,
    /// The buffer as it would be saved, mirrored from `editor` after each
    /// edit; unsaved edits are `text != document.right_text`.
    pub text: String,
    pub editor: super::live::LiveEditor,
    /// The left text re-diffed against the buffer, and the editor marks
    /// derived from it.
    pub live: super::live::LiveDiff,
}

impl Edit {
    pub fn new(document: std::sync::Arc<super::DiffDocument>) -> Self {
        Self {
            text: document.right_text.clone(),
            editor: super::live::LiveEditor::new(&document.right_text),
            live: super::live::LiveDiff::default(),
            document,
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
