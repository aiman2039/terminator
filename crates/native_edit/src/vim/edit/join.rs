//! `J` / `gJ` (and Visual `J`, `:join`): join lines.
use super::super::super::doc::Buffer;
use super::super::mode::{Cursor, Effect};
use super::super::state::VimEngine;
use super::super::words::line_len;

impl VimEngine {
    /// `J` joins `count` lines (at least two) at the cursor line.
    pub(crate) fn join_key<B: Buffer>(
        &mut self,
        doc: &mut B,
        count: usize,
        spaces: bool,
    ) -> Vec<Effect> {
        let first = self.cursor.line;
        let last = first
            .saturating_add(count.max(2).saturating_sub(1))
            .min(Self::last_addressable(doc));
        if last <= first {
            return self.finish_simple(false);
        }
        self.x.keys_change = true;
        self.join_lines(doc, (first, last), spaces);
        self.clear_pending();
        self.rec_commit();
        Vec::new()
    }

    /// Join lines `first..=last` into `first`. With `spaces`, the next
    /// line's indent goes and one space separates (none after a blank or
    /// before `)`). The cursor lands on the last join point.
    pub(crate) fn join_lines<B: Buffer>(
        &mut self,
        doc: &mut B,
        (first, last): (usize, usize),
        spaces: bool,
    ) {
        doc.begin_undo_group();
        let mut col = self.cursor.col;
        for _ in first..last {
            if first.saturating_add(1) >= doc.line_count() {
                break;
            }
            let current = doc.line_text(first);
            let len = current.chars().count();
            let next = doc.line_text(first.saturating_add(1));
            let indent = if spaces {
                next.chars().take_while(|c| c.is_whitespace()).count()
            } else {
                0
            };
            let rest = next.chars().skip(indent).collect::<String>();
            let sep = if !spaces
                || current.is_empty()
                || current.ends_with([' ', '\t'])
                || rest.is_empty()
                || rest.starts_with(')')
            {
                ""
            } else {
                " "
            };
            let from = doc.char_at_line_col(first, len);
            let to = doc.char_at_line_col(first.saturating_add(1), indent);
            doc.delete(from..to);
            doc.insert(from, sep);
            col = len;
        }
        doc.end_undo_group();
        doc.end_run();
        self.cursor = Cursor {
            line: first,
            col: col.min(line_len(doc, first).saturating_sub(1)),
        };
        self.preferred_col = self.cursor.col;
        self.clamp(doc);
    }
}
