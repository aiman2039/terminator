//! Reopen a file where it was left (`JetBrains`): a buffer's caret, top
//! line and folds go into the UI preferences when the buffer is dropped
//! and at exit, and come back when the file opens again without a
//! target line.
use super::*;
use crate::preferences::EditorState;
use std::collections::HashSet;
use terminator_native_edit::highlight::HighlightCache;

impl NativeDoc {
    /// The live view's caret, top line and folds. `None` until the buffer
    /// has loaded and its remembered state was put back: a buffer that
    /// never showed must not replace what was remembered.
    pub(super) fn editor_state(&self) -> Option<EditorState> {
        if self.restore.is_some() {
            return None;
        }
        self.doc.as_ref()?;
        let cursor = self.engine.cursor();
        Some(EditorState {
            path: self.path.clone(),
            line: cursor.line,
            column: cursor.col,
            top: self.top_line.unwrap_or_else(|| self.engine.scroll_line()),
            folds: self.highlight.folds().folded_starts(),
        })
    }

    /// Put `state` back on the loaded buffer: the folds that still start
    /// a region, the caret, and the top line for the first drawn frame.
    fn apply_editor_state(&mut self, state: &EditorState) {
        let Some(buffer) = self.doc.as_mut() else {
            return;
        };
        let language = language_for_path(&self.path.to_string_lossy());
        self.highlight
            .restore_folds(&*buffer, language, &state.folds);
        let _ = self.engine.place_cursor(buffer, state.line, state.column);
        self.top_line = Some(state.top);
    }

    /// Parts for a view seen for the first time (a split, or the file
    /// opened again while its buffer stayed): the live view's caret and
    /// folds, and its top line for the first frame. A jump that still
    /// waits to scroll into view moves to the new view instead.
    pub(super) fn fork_view(&mut self) -> (VimEngine, HighlightCache) {
        let mut engine = self.engine.fork_view();
        if self.engine.take_scroll_request() {
            engine.request_scroll();
        } else if self.doc.is_some() {
            self.top_line.get_or_insert(self.engine.scroll_line());
        }
        (engine, self.highlight.fork_folds())
    }
}

impl App {
    /// Put back `path`'s remembered state once its buffer has loaded. A
    /// target line that waits for the buffer (go to definition, a search
    /// hit, `file:line`) wins: then nothing is restored.
    pub(super) fn restore_native_state(&mut self, path: &Path) {
        let explicit = self.native_pending_line.contains_key(path);
        let Some(doc) = self.native_docs.get_mut(path) else {
            return;
        };
        if doc.doc.is_none() {
            return;
        }
        if let Some(state) = doc.restore.take()
            && !explicit
        {
            doc.apply_editor_state(&state);
        }
    }

    /// Keep `path`'s state before its buffer is dropped.
    pub(super) fn remember_native_state(&mut self, path: &Path) {
        if let Some(state) = self.native_docs.get(path).and_then(NativeDoc::editor_state) {
            self.preferences.editor_states.remember(state);
        }
    }

    /// Keep the state of every buffer whose path is not in `live`.
    pub(super) fn remember_dropped_native(&mut self, live: &HashSet<PathBuf>) {
        for (path, doc) in &self.native_docs {
            if !live.contains(path)
                && let Some(state) = doc.editor_state()
            {
                self.preferences.editor_states.remember(state);
            }
        }
    }

    /// The preferences the exit checkpoint saves: with the state of every
    /// open buffer, so files reopen where they were after a restart.
    pub(crate) fn preferences_for_exit(&self) -> UiPreferences {
        let mut preferences = self.preferences.clone();
        for doc in self.native_docs.values() {
            if let Some(state) = doc.editor_state() {
                preferences.editor_states.remember(state);
            }
        }
        preferences
    }
}
