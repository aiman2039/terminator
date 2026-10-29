//! The editable split diff's right pane is the native editor: the same
//! widget, keymap (Vim or modeless), undo and search as a file tab, with
//! filler rows and diff tints keeping it aligned with the painted left
//! pane. The unified view stays read-only; its Edit button opens this one.
use super::super::*;

mod right;

pub(super) use right::{
    DiffEditorOptionsInput, PaintRightEditorInput, diff_editor_options, paint_right_editor,
};

impl App {
    pub(crate) fn block_unsaved_diff(&mut self, key: Option<&str>) -> bool {
        let dirty = self.diff_reviews.iter().any(|(id, state)| {
            key.is_none_or(|key| key == id)
                && (state.unsaved_edits() || state.saving_edits && state.busy)
        });
        if dirty {
            self.error = Some("Save or discard edits in the diff viewer before closing.".into());
            return true;
        }
        let merge_dirty = self
            .merge
            .iter()
            .any(|(id, state)| key.is_none_or(|key| key == id) && (state.dirty || state.busy()));
        if merge_dirty {
            self.error = Some("Save the merge result before closing.".into());
        }
        merge_dirty
    }
}
