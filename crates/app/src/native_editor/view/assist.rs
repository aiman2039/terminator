//! Language-server assists around the caret: document highlights (they
//! replace the plain caret-word match when the server supports them) and
//! on-type formatting after an advertised trigger character.
use super::*;
use terminator_native_edit::view::{SymbolRanges, set_symbol_highlights};

impl App {
    /// The loaded buffer revision of `path`, read before a view pass.
    pub(super) fn native_revision(&self, path: &Path) -> Option<u64> {
        self.native_docs
            .get(path)
            .and_then(NativeDoc::buffer)
            .map(Buffer::revision)
    }

    /// Ask for the caret symbol's document highlights and publish the
    /// answer for this source view. Without server support nothing is
    /// published and the view keeps its local caret-word match.
    pub(super) fn native_view_highlights(&mut self, ctx: &egui::Context, path: &Path, id: &str) {
        let Some(doc) = self.native_docs.get(path) else {
            return;
        };
        let Some(buffer) = doc.buffer() else {
            return;
        };
        let revision = buffer.revision();
        let ranges = self
            .lsp
            .document_highlights(path, revision)
            .map(|ranges| symbol_ranges(buffer, &ranges));
        set_symbol_highlights(ctx, id, ranges);
        if doc.engine.selection().is_none()
            && let Some((line, character)) = doc.caret_position()
        {
            self.lsp.request_document_highlight(
                path,
                revision,
                native_lsp::Position { line, character },
            );
        }
    }

    /// After a pass that typed into the buffer: when the char before the
    /// caret (or a fresh line) is an advertised on-type trigger, sync the
    /// text and ask the server to format.
    pub(super) fn native_view_on_type(
        &mut self,
        path: &Path,
        language: Language,
        revision_before: Option<u64>,
    ) {
        let Some(doc) = self.native_docs.get(path) else {
            return;
        };
        let Some(buffer) = doc.buffer() else {
            return;
        };
        if revision_before.is_none_or(|before| before == buffer.revision())
            || doc.engine.selection().is_some()
        {
            return;
        }
        let triggers = self.lsp.on_type_triggers(path);
        let cursor = doc.engine.cursor();
        let Some(typed) = typed_trigger(&buffer.line_text(cursor.line), cursor, &triggers) else {
            return;
        };
        let Some((line, character)) = doc.caret_position() else {
            return;
        };
        if let Some(revision) = self.push_live_text(path, language) {
            self.lsp.request_on_type_format(
                path,
                revision,
                (native_lsp::Position { line, character }, typed),
            );
        }
    }
}

/// The trigger just typed before the caret: the char before it, or a
/// newline when the caret sits in the indentation of a line after the
/// first.
fn typed_trigger(
    text: &str,
    cursor: terminator_native_edit::vim::Cursor,
    triggers: &[char],
) -> Option<char> {
    if triggers.is_empty() {
        return None;
    }
    let before: Vec<char> = text.chars().take(cursor.col).collect();
    if let Some(last) = before.last().filter(|c| !c.is_whitespace())
        && triggers.contains(last)
    {
        return Some(*last);
    }
    let fresh_line = cursor.line > 0 && before.iter().all(|c| c.is_whitespace());
    (fresh_line && triggers.contains(&'\n')).then_some('\n')
}

/// LSP (UTF-16) ranges as `(line, start, end)` char columns; a range
/// over several lines covers each line it touches.
fn symbol_ranges(buffer: &Doc, ranges: &[native_lsp::Range]) -> SymbolRanges {
    let mut out = Vec::new();
    for range in ranges {
        let first = usize::try_from(range.start.line).unwrap_or(usize::MAX);
        let last = usize::try_from(range.end.line).unwrap_or(usize::MAX);
        for line in first..=last.min(buffer.line_count().saturating_sub(1)) {
            let text = buffer.line_text(line);
            let start = if line == first {
                native_lsp::utf16_to_char_col(&text, range.start.character)
            } else {
                0
            };
            let end = if line == last {
                native_lsp::utf16_to_char_col(&text, range.end.character)
            } else {
                text.chars().count()
            };
            if start < end {
                out.push((line, start, end));
            }
        }
    }
    out.sort_unstable();
    Arc::new(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminator_native_edit::vim::Cursor;

    #[test]
    fn typed_trigger_needs_an_advertised_character() {
        let at = |line, col| Cursor { line, col };
        assert_eq!(typed_trigger("  }", at(1, 3), &['}', ';']), Some('}'));
        assert_eq!(typed_trigger("  x", at(1, 3), &['}', ';']), None);
        assert_eq!(typed_trigger("    ", at(2, 4), &['\n']), Some('\n'));
        assert_eq!(typed_trigger("    ", at(2, 4), &['}']), None);
        assert_eq!(typed_trigger("  }", at(1, 3), &[]), None);
    }

    #[test]
    fn server_ranges_become_char_columns_per_line() {
        let buffer = Doc::new("let é = 1;\né + é\n");
        let pos = |line, character| native_lsp::Position { line, character };
        let ranges = symbol_ranges(
            &buffer,
            &[
                native_lsp::Range {
                    start: pos(1, 4),
                    end: pos(1, 5),
                },
                native_lsp::Range {
                    start: pos(0, 4),
                    end: pos(0, 5),
                },
            ],
        );
        assert_eq!(ranges.as_slice(), [(0, 4, 5), (1, 4, 5)]);
    }
}
