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
    pub editor: Option<Edit>,
    pub saving_edits: bool,
    pub selected: Option<usize>,
    pub jump: bool,
    pub confirm_discard: Option<usize>,
    pub busy: bool,
    pub undo: Vec<super::actions::Undo>,
    pub context: usize,
    pub message: Option<String>,
}

pub(crate) struct Edit {
    pub document: std::sync::Arc<super::DiffDocument>,
    pub text: String,
}

impl State {
    pub fn unsaved_edits(&self) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|edit| edit.text != edit.document.right_text)
    }
}

impl State {
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
