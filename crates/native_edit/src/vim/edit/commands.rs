//! Normal-mode editing keys (`x`, `D`, `p`, `~`, `u`, `.`, inserts, …).
use super::super::super::doc::Buffer;
use super::super::helpers::{grapheme_right, range_text};
use super::super::mode::{Bit, Effect, Key, PendingOp, Place, Prefix, Repeat};
use super::super::motion::Mo;
use super::super::state::VimEngine;
use super::super::words::line_len;
use super::case;

impl VimEngine {
    /// Editing commands; `None` leaves the key to [`Self::normal_command`].
    pub(crate) fn edit_command<B: Buffer>(
        &mut self,
        doc: &mut B,
        key: Key,
        count: usize,
    ) -> Option<Vec<Effect>> {
        let op = match key {
            Key::Char('d') => Some(PendingOp::Delete),
            Key::Char('c') => Some(PendingOp::Change),
            Key::Char('y') => Some(PendingOp::Yank),
            Key::Char('>') => Some(PendingOp::Indent),
            Key::Char('<') => Some(PendingOp::Outdent),
            _ => None,
        };
        if let Some(op) = op {
            return Some(self.operator_key(doc, op));
        }
        if let Some(place) = insert_place(key) {
            self.insert_key_at(doc, (key, place), count);
            return Some(Vec::new());
        }
        Some(match key {
            // The Delete key deletes like `x`, including under counts.
            Key::Char('x') | Key::Delete => {
                self.rec_bit(Bit::Key(Key::Char('x')));
                self.delete_chars_forward(doc, count);
                self.rec_commit();
                Vec::new()
            }
            Key::Char('X') => self.op_motion(doc, (PendingOp::Delete, Mo::Left), count),
            Key::Char('D') => self.op_motion(doc, (PendingOp::Delete, Mo::LineEnd), count),
            Key::Char('C') => self.op_motion(doc, (PendingOp::Change, Mo::LineEnd), count),
            Key::Char('s') => self.op_motion(doc, (PendingOp::Change, Mo::Right), count),
            Key::Char('S') => {
                self.operator_key(doc, PendingOp::Change);
                self.operator_key(doc, PendingOp::Change)
            }
            Key::Char('Y') => self.doubled_operator(doc, PendingOp::Yank, count),
            Key::Char(c @ ('p' | 'P')) => {
                self.rec_bit(Bit::Key(key));
                self.put(doc, c == 'P', count);
                self.clear_pending();
                self.rec_commit();
                Vec::new()
            }
            Key::Char('r') => {
                self.x.prefix = Some(Prefix::Replace);
                Vec::new()
            }
            Key::Char('R') => self.start_replace_mode(doc),
            Key::Char('J') => self.join_key(doc, count, true),
            Key::Char('~') => self.tilde(doc, count),
            Key::Ctrl('a') => self.increment(doc, count, true),
            Key::Ctrl('x') => self.increment(doc, count, false),
            Key::Char('u') => {
                for _ in 0..count {
                    self.undo(doc);
                }
                Vec::new()
            }
            Key::Ctrl('r') => {
                for _ in 0..count {
                    self.redo(doc);
                }
                Vec::new()
            }
            Key::Char('.') => self.repeat_key(doc, count),
            _ => return None,
        })
    }

    /// An operator applied to a fixed motion (`D` is `d$`, `s` is `cl`).
    fn op_motion<B: Buffer>(
        &mut self,
        doc: &mut B,
        (op, mo): (PendingOp, Mo),
        count: usize,
    ) -> Vec<Effect> {
        self.operator_key(doc, op);
        let typed = self.has_count();
        self.run_motion_counted(doc, mo, super::super::motion::Count { n: count, typed })
    }

    /// `.` replays the last change; a count replaces the original one.
    fn repeat_key<B: Buffer>(&mut self, doc: &mut B, count: usize) -> Vec<Effect> {
        let typed = self.has_count();
        let Some(change) = self.last_change.clone() else {
            return self.finish_simple(false);
        };
        let change = if typed {
            change.with_count(count)
        } else {
            change
        };
        self.clear_pending();
        self.exec_repeat(doc, change)
    }

    /// `~`: toggle the case of `count` chars and step past them.
    fn tilde<B: Buffer>(&mut self, doc: &mut B, count: usize) -> Vec<Effect> {
        let text = doc.line_text(self.cursor.line);
        if text.is_empty() {
            return self.finish_simple(false);
        }
        self.x.keys_change = true;
        let end = grapheme_right(&text, self.cursor.col, count);
        let from = doc.char_at_line_col(self.cursor.line, self.cursor.col);
        let to = doc.char_at_line_col(self.cursor.line, end);
        let old = range_text(doc, from, to);
        self.replace_text(doc, (from, to), &case::convert(&old, PendingOp::Toggle));
        self.cursor.col = end;
        self.preferred_col = end;
        self.clear_pending();
        self.clamp(doc);
        self.rec_commit();
        Vec::new()
    }

    /// Forward delete of `count` graphemes (`x`), shared by the live key
    /// and `.` replay. One buffer op: already a single undo step.
    pub(crate) fn delete_chars_forward<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        let text = doc.line_text(self.cursor.line);
        let len = line_len(doc, self.cursor.line);
        if self.cursor.col < len {
            let end = grapheme_right(&text, self.cursor.col, count.max(1));
            let from = self.char_idx(doc, self.cursor);
            let to = doc.char_at_line_col(self.cursor.line, end);
            // Vim parity: `x` fills the unnamed register, so `xp` swaps.
            self.write_delete(range_text(doc, from, to), false);
            self.cursor = self.delete_range(doc, from, to);
        }
        self.clear_pending();
        self.clamp(doc);
    }
}

impl Repeat {
    /// The same change with a new count (`3.`).
    pub(crate) fn with_count(self, count: usize) -> Self {
        match self {
            Repeat::DeleteChars { .. } => Repeat::DeleteChars { count },
            Repeat::DeleteMotion { motion, .. } => Repeat::DeleteMotion { motion, count },
            Repeat::DeleteLines { .. } => Repeat::DeleteLines { count },
            Repeat::ChangeMotion { motion, text, .. } => Repeat::ChangeMotion {
                motion,
                count,
                text,
            },
            Repeat::ChangeLines { text, .. } => Repeat::ChangeLines { count, text },
            Repeat::Paste { before, .. } => Repeat::Paste { count, before },
            Repeat::Insert { text, place } => Repeat::Insert {
                text: repeat_text(&text, place, count),
                place,
            },
            Repeat::Keys(inputs) => Repeat::Keys(super::super::input::with_count(inputs, count)),
            other => other,
        }
    }
}

/// Inserted text typed `count` times; opened lines repeat as lines.
pub(crate) fn repeat_text(text: &str, place: Place, count: usize) -> String {
    let count = count.max(1);
    if matches!(place, Place::LineBelow | Place::LineAbove) {
        vec![text; count].join("\n")
    } else {
        text.repeat(count)
    }
}

fn insert_place(key: Key) -> Option<Place> {
    Some(match key {
        Key::Char('i') => Place::AtCursor,
        Key::Char('a') => Place::AfterCursor,
        Key::Char('A') => Place::LineEnd,
        Key::Char('I') => Place::FirstBlank,
        Key::Char('o') => Place::LineBelow,
        Key::Char('O') => Place::LineAbove,
        _ => return None,
    })
}

/// What `3ihi<Esc>` adds after the typed `hi`: `count - 1` more copies,
/// each on its own new line for `o`/`O`.
pub(crate) fn repeat_more(text: &str, place: Place, count: usize) -> String {
    let more = count.saturating_sub(1);
    if matches!(place, Place::LineBelow | Place::LineAbove) {
        format!("\n{text}").repeat(more)
    } else {
        text.repeat(more)
    }
}
