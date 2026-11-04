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

/// Revealed unchanged-line ranges of the left text `old`, moved to the
/// same lines of `new` (a refresh after staging shifts the index). Lines
/// that changed are no longer unchanged context and are dropped.
pub(crate) fn map_revealed(old: &str, new: &str, revealed: &[Range<usize>]) -> Vec<Range<usize>> {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let deadline = std::time::Instant::now().checked_add(super::DIFF_TIMEOUT);
    let ops = similar::capture_diff_slices_deadline(
        similar::Algorithm::Myers,
        &old_lines,
        &new_lines,
        deadline,
    );
    let mut out = Vec::new();
    for op in ops.iter().filter(|op| op.tag() == similar::DiffTag::Equal) {
        let (old_range, new_range) = (op.old_range(), op.new_range());
        for range in revealed {
            let start = range.start.max(old_range.start);
            let end = range.end.min(old_range.end);
            if start < end {
                let shift = |line: usize| {
                    line.saturating_sub(old_range.start)
                        .saturating_add(new_range.start)
                };
                out.push(shift(start)..shift(end));
            }
        }
    }
    out
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
    /// The last picked line `(old side, line)`; Shift+arrows extend from it.
    pub pick_anchor: Option<(bool, usize)>,
    /// A press-and-drag line pick in progress.
    pub drag: Option<super::picks::PickDrag>,
    /// The changed line under the pointer last frame: `(hunk, old side, line)`.
    pub hover_line: Option<(usize, bool, usize)>,
    /// `]` or `[` typed; `c` next steps to the next or previous change.
    pub bracket: Option<bool>,
    /// The last action was a discard: the notice offers Undo.
    pub offer_undo: bool,
    /// Row context menu: screen position and the file line it opens.
    pub row_menu: Option<([f32; 2], u32)>,
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
    /// A new document replaced the old one: hunk numbers may have changed,
    /// so line picks and a pending discard no longer apply.
    pub fn reloaded(&mut self, hunks: usize) {
        self.confirm_discard = None;
        self.clear_picks();
        self.selected = hunks
            .checked_sub(1)
            .map(|last| self.selected.unwrap_or(0).min(last));
    }

    /// An action (stage, unstage, discard, save, undo) finished or failed.
    pub fn action_finished(&mut self) {
        self.busy = false;
        self.confirm_discard = None;
        self.clear_picks();
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

#[cfg(test)]
mod tests {
    use super::State;

    #[test]
    fn reload_clears_line_picks_and_pending_discard() {
        let mut state = State::default();
        state.toggle_line(1, false, 4);
        state.confirm_discard = Some(1);
        state.selected = Some(5);
        state.reloaded(2);
        assert!(state.picked.is_none());
        assert!(state.confirm_discard.is_none());
        assert_eq!(state.selected, Some(1));
    }

    #[test]
    fn revealed_lines_follow_their_text_when_lines_are_added_above() {
        let old = (0..30).fold(String::new(), |mut text, n| {
            use std::fmt::Write;
            let _ = writeln!(text, "line {n}");
            text
        });
        let new = format!("new a\nnew b\n{}", old.replace("line 12\n", "changed\n"));
        let mapped = super::map_revealed(&old, &new, std::slice::from_ref(&(10..15)));
        // Lines 10..12 and 13..15 move down by two; changed line 12 drops.
        assert_eq!(mapped, [12..14, 15..17]);
    }

    #[test]
    fn finished_action_clears_line_picks() {
        let mut state = State::default();
        state.toggle_line(0, true, 2);
        state.busy = true;
        state.action_finished();
        assert!(state.picked.is_none());
        assert!(!state.busy);
    }
}
