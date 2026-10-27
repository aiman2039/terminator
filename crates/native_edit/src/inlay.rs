//! Inlay hints (`IntelliJ` / rust-analyzer inline type and parameter
//! names): virtual text painted inside a line without being part of it.
//!
//! A server answer describes one buffer revision. [`InlayCache`] adopts
//! it only while the buffer still has that revision, remembering the text
//! of every hinted line; after later edits a hint keeps showing exactly
//! while its line is unchanged, so typing hides the hints of the edited
//! line instead of painting them at shifted columns.

use crate::doc::Buffer;
use std::collections::HashMap;
use std::sync::Arc;

/// One hint in char columns: `label` is painted before the char at `col`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlayHint {
    pub line: usize,
    pub col: usize,
    pub label: String,
}

#[derive(Default)]
pub struct InlayCache {
    /// Buffer revision the adopted hints describe.
    source: Option<u64>,
    hints: Vec<InlayHint>,
    /// Text of each hinted line at `source`.
    lines: HashMap<usize, String>,
    /// Hints still valid for the buffer revision `shown_for`.
    shown_for: Option<u64>,
    shown: Arc<Vec<InlayHint>>,
}

impl InlayCache {
    /// Adopt a server answer for buffer revision `revision`. Ignored when
    /// the buffer has moved on (the next sync asks again) or the answer
    /// is already adopted.
    pub fn adopt<B: Buffer>(&mut self, doc: &B, revision: u64, hints: &[crate::lsp::InlayHint]) {
        if doc.revision() != revision || self.source == Some(revision) {
            return;
        }
        let mut lines: HashMap<usize, String> = HashMap::new();
        let mut adopted = Vec::with_capacity(hints.len());
        for hint in hints {
            let Ok(line) = usize::try_from(hint.position.line) else {
                continue;
            };
            if line >= doc.line_count() {
                continue;
            }
            let text = lines.entry(line).or_insert_with(|| doc.line_text(line));
            let col = crate::lsp::utf16_to_char_col(text, hint.position.character);
            adopted.push(InlayHint {
                line,
                col,
                label: hint.label.clone(),
            });
        }
        adopted.sort_by_key(|hint| (hint.line, hint.col));
        self.source = Some(revision);
        self.hints = adopted;
        self.lines = lines;
        self.shown_for = None;
    }

    /// Drop every hint (hints turned off, or the server went away).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Hints whose line is unchanged since the answer, sorted by line and
    /// column. Recomputed once per buffer revision.
    pub fn visible<B: Buffer>(&mut self, doc: &B) -> Arc<Vec<InlayHint>> {
        if self.shown_for != Some(doc.revision()) {
            self.shown_for = Some(doc.revision());
            let still = |line: usize| {
                self.lines
                    .get(&line)
                    .is_some_and(|text| line < doc.line_count() && doc.line_text(line) == *text)
            };
            let mut valid: HashMap<usize, bool> = HashMap::new();
            self.shown = Arc::new(
                self.hints
                    .iter()
                    .filter(|hint| *valid.entry(hint.line).or_insert_with(|| still(hint.line)))
                    .cloned()
                    .collect(),
            );
        }
        Arc::clone(&self.shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;
    use crate::lsp::Position;

    fn hint(line: u32, character: u32, label: &str) -> crate::lsp::InlayHint {
        crate::lsp::InlayHint {
            position: Position { line, character },
            label: label.into(),
        }
    }

    #[test]
    fn hints_hide_on_edited_lines_and_stay_on_untouched_ones() {
        let mut doc = Doc::new("let a = 1;\nlet b = 2;\n");
        let mut cache = InlayCache::default();
        let answer = [hint(0, 5, ": i32"), hint(1, 5, ": u8")];
        cache.adopt(&doc, doc.revision(), &answer);
        assert_eq!(cache.visible(&doc).len(), 2);
        assert_eq!(cache.visible(&doc)[0].col, 5);
        // Typing on line 0 hides its hint; line 1 keeps its own.
        doc.insert(9, "0");
        let shown = cache.visible(&doc);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].line, 1);
        // A stale answer (buffer moved on) is not adopted.
        let stale = doc.revision();
        doc.insert(0, "\n");
        cache.adopt(&doc, stale, &[hint(0, 0, "x")]);
        assert!(cache.visible(&doc).is_empty(), "lines shifted down");
    }

    #[test]
    fn hint_columns_convert_from_utf16() {
        let doc = Doc::new("let 😀x = 1;\n");
        let mut cache = InlayCache::default();
        cache.adopt(&doc, doc.revision(), &[hint(0, 7, ": i32")]);
        assert_eq!(cache.visible(&doc)[0].col, 6);
    }
}
