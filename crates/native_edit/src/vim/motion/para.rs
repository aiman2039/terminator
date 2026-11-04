//! `{` and `}`: paragraph boundaries are empty lines.
use super::super::super::doc::Buffer;
use super::super::mode::Cursor;
use super::super::state::VimEngine;
use super::super::words::line_len;
use super::{MKind, Motion};

impl VimEngine {
    pub(super) fn para_motion<B: Buffer>(&self, doc: &B, forward: bool, count: usize) -> Motion {
        let last = Self::last_addressable(doc);
        let empty = |line: usize| doc.line_text(line).is_empty();
        let mut line = self.cursor.line;
        for _ in 0..count {
            if forward {
                while line < last && empty(line) {
                    line = line.saturating_add(1);
                }
                while line < last && !empty(line) {
                    line = line.saturating_add(1);
                }
            } else {
                while line > 0 && empty(line) {
                    line = line.saturating_sub(1);
                }
                while line > 0 && !empty(line) {
                    line = line.saturating_sub(1);
                }
            }
        }
        // Past the last paragraph `}` lands on the end of the last line.
        let col = if forward && line == last && !empty(line) {
            line_len(doc, line)
        } else {
            0
        };
        Motion::new(Cursor { line, col }, MKind::Exclusive).jump()
    }
}
