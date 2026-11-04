//! The jump list (Ctrl-O / Ctrl-I) and marks (`m`, `'`, `` ` ``).
use super::super::super::doc::Buffer;
use super::super::mode::Cursor;
use super::super::state::VimEngine;
use super::{MKind, Motion};

/// Jump list entries kept, like vim's 100.
const MAX_JUMPS: usize = 100;

impl VimEngine {
    /// Remember the cursor before a jump; it also becomes the `''` mark.
    pub(crate) fn push_jump(&mut self) {
        let here = self.cursor;
        let jumps = &mut self.x.jumps;
        jumps.retain(|c| c.line != here.line);
        jumps.push(here);
        if jumps.len() > MAX_JUMPS {
            jumps.remove(0);
        }
        self.x.jump_idx = jumps.len();
        self.x.marks.insert('\'', here);
    }

    /// Ctrl-O (`older`) / Ctrl-I: walk the jump list `count` steps.
    pub(crate) fn walk_jumps<B: Buffer>(&mut self, doc: &B, older: bool, count: usize) -> bool {
        if older && self.x.jump_idx >= self.x.jumps.len() {
            // Leaving the newest spot: keep it so Ctrl-I can return.
            self.push_jump();
            self.x.jump_idx = self.x.jumps.len().saturating_sub(1);
        }
        let target = if older {
            self.x.jump_idx.checked_sub(count)
        } else {
            self.x
                .jump_idx
                .checked_add(count)
                .filter(|idx| *idx < self.x.jumps.len())
        };
        let Some(idx) = target else { return false };
        let Some(spot) = self.x.jumps.get(idx).copied() else {
            return false;
        };
        self.x.jump_idx = idx;
        self.cursor = spot;
        self.preferred_col = spot.col;
        self.clamp(doc);
        self.scroll_request = true;
        true
    }

    /// `m{a-zA-Z}`: remember the cursor.
    pub(crate) fn set_mark(&mut self, name: char) -> bool {
        if !name.is_ascii_alphabetic() {
            return false;
        }
        self.x.marks.insert(name, self.cursor);
        true
    }

    /// Position of mark `name`: letters, `'` (before the last jump) and
    /// `<` / `>` (last visual selection).
    pub(crate) fn mark_pos(&self, name: char) -> Option<Cursor> {
        let ordered = |(_, a, b): (super::super::mode::Mode, Cursor, Cursor)| {
            if (a.line, a.col) <= (b.line, b.col) {
                (a, b)
            } else {
                (b, a)
            }
        };
        match name {
            '`' | '\'' => self.x.marks.get(&'\'').copied(),
            '<' => self.x.last_visual.map(|v| ordered(v).0),
            '>' => self.x.last_visual.map(|v| ordered(v).1),
            _ => self.x.marks.get(&name).copied(),
        }
    }

    pub(super) fn mark_motion<B: Buffer>(
        &self,
        doc: &B,
        name: char,
        exact: bool,
    ) -> Option<Motion> {
        let pos = self.mark_pos(name)?;
        let line = pos.line.min(Self::last_addressable(doc));
        Some(if exact {
            Motion::new(Cursor { line, col: pos.col }, MKind::Exclusive).jump()
        } else {
            let col = Self::first_non_blank(doc, line);
            Motion::new(Cursor { line, col }, MKind::Linewise).jump()
        })
    }
}
