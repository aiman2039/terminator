use super::super::doc::Buffer;
use super::mode::{Bit, BlockPending, Cursor, Effect, Key, ModalEngine, PendingOp, Place, Repeat};
use super::operators::DeleteBlockRangesInput;
use super::state::VimEngine;
use super::words::line_len;

/// A recorded rectangle edit to replay from the cursor line.
struct BlockReplay {
    height: usize,
    left: usize,
    right: usize,
    append: bool,
}

#[derive(PartialEq)]
enum Tok {
    Count(usize),
    Op(char),
    Motion(char),
    Key(Key),
}

/// Recorded bits as tokens, with the leading count folded and the typed
/// text joined (only key order matters for the tokens).
struct RepeatTokens {
    toks: Vec<Tok>,
    typed: String,
    /// Index of the first token after the leading count.
    pos: usize,
    count: usize,
}

impl RepeatTokens {
    fn new(bits: &[Bit]) -> Self {
        let mut toks: Vec<Tok> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        for bit in bits {
            match bit {
                Bit::Key(Key::Char(c)) if c.is_ascii_digit() => Self::push_digit(&mut toks, *c),
                Bit::Key(key) => toks.push(Tok::Key(*key)),
                Bit::Op(op) => toks.push(Tok::Op(*op)),
                Bit::Motion(motion) => toks.push(Tok::Motion(*motion)),
                Bit::Type(chunk) => texts.push(chunk.clone()),
            }
        }
        let mut pos = 0usize;
        let mut count = 1usize;
        while let Some(Tok::Count(n)) = toks.get(pos) {
            count = count.saturating_mul(*n).max(1);
            pos = pos.saturating_add(1);
        }
        Self {
            toks,
            typed: texts.concat(),
            pos,
            count,
        }
    }

    /// A leading `0` is the line-start motion (the count parser never
    /// emits one); digits after an operator or motion are a second count.
    /// Both multiply vim-style.
    fn push_digit(toks: &mut Vec<Tok>, c: char) {
        if c == '0' && !matches!(toks.last(), Some(Tok::Count(_))) {
            toks.push(Tok::Motion('0'));
            return;
        }
        let digit = (c as usize).saturating_sub('0' as usize);
        if let Some(Tok::Count(n)) = toks.last_mut() {
            *n = n.saturating_mul(10).saturating_add(digit);
        } else {
            toks.push(Tok::Count(digit));
        }
    }

    /// The LAST visual marker wins, so entering the block from charwise
    /// visual still replays as a block: `Some(true)` for Ctrl-V,
    /// `Some(false)` for `v`/`V`.
    fn visual_marker(&self) -> Option<bool> {
        self.toks
            .iter()
            .rfind(|tok| matches!(tok, Tok::Key(Key::CtrlV | Key::Char('v' | 'V'))))
            .map(|tok| matches!(tok, Tok::Key(Key::CtrlV)))
    }

    /// A block session carries the Ctrl-V toggle; dims ride along from the
    /// live pending state.
    fn block(self, block: Option<BlockPending>) -> Option<Repeat> {
        let block = block?;
        let height = block.bottom.saturating_sub(block.top);
        let is_change = self
            .toks
            .iter()
            .any(|tok| matches!(tok, Tok::Key(Key::Char('c'))));
        if self.typed.is_empty() {
            return None;
        }
        Some(if is_change {
            Repeat::BlockChange {
                text: self.typed,
                height,
                left: block.left,
                right: block.right,
                append: block.append,
            }
        } else {
            Repeat::BlockInsert {
                text: self.typed,
                height,
                left: block.left,
                append: block.append,
            }
        })
    }

    /// Non-visual changes: `x`, `p`, inserts, and `d`/`c` operators.
    fn change(self) -> Option<Repeat> {
        // Tokens after the leading count use indexed `.get()`, never slicing.
        let tail_len = self.toks.len().saturating_sub(self.pos);
        let count = self.count;
        match self.toks.get(self.pos)? {
            Tok::Key(Key::Char('x')) if tail_len == 1 => Some(Repeat::DeleteChars { count }),
            Tok::Key(Key::Char('p')) if tail_len == 1 => Some(Repeat::Paste { count }),
            Tok::Key(Key::Char(place @ ('i' | 'a' | 'A' | 'I' | 'o' | 'O'))) => {
                if tail_len != 1 || self.typed.is_empty() {
                    return None;
                }
                let place = Self::insert_place(*place);
                Some(Repeat::Insert {
                    text: self.typed,
                    place,
                })
            }
            Tok::Op(op @ ('d' | 'c')) => {
                let op = *op;
                self.operator(op)
            }
            _ => None,
        }
    }

    fn insert_place(key: char) -> Place {
        match key {
            'i' => Place::AtCursor,
            'a' => Place::AfterCursor,
            'A' => Place::LineEnd,
            'I' => Place::FirstBlank,
            'o' => Place::LineBelow,
            _ => Place::LineAbove,
        }
    }

    /// `dd`/`cc` and `d{motion}`/`c{motion}`, with an optional second
    /// count after the operator.
    fn operator(self, op: char) -> Option<Repeat> {
        let mut off = 1usize;
        let mut count2 = 1usize;
        while let Some(Tok::Count(n)) = self.toks.get(self.pos.saturating_add(off)) {
            count2 = count2.saturating_mul(*n).max(1);
            off = off.saturating_add(1);
        }
        let total = self.count.saturating_mul(count2).max(1);
        // The change must end exactly here: no trailing tokens.
        let at = self.pos.saturating_add(off);
        let ends_here = self.toks.len() == at.saturating_add(1);
        match self.toks.get(at) {
            Some(Tok::Op(second)) if *second == op && ends_here => {
                if op == 'd' {
                    Some(Repeat::DeleteLines { count: total })
                } else {
                    // `cc` + immediate Escape deletes the lines; an empty
                    // insert replays as nothing to type.
                    Some(Repeat::ChangeLines {
                        count: total,
                        text: self.typed,
                    })
                }
            }
            Some(Tok::Motion(motion)) if ends_here => {
                // A bare `G` (count 0) means last line; every other motion
                // defaults an absent count to 1.
                let count = if *motion == 'G' && self.count == 1 && count2 == 1 {
                    0
                } else {
                    total
                };
                let motion = *motion;
                Some(if op == 'd' {
                    Repeat::DeleteMotion { motion, count }
                } else {
                    Repeat::ChangeMotion {
                        motion,
                        count,
                        text: self.typed,
                    }
                })
            }
            _ => None,
        }
    }
}

impl VimEngine {
    /// Parse recorded bits into the last change. Returns `None` for
    /// not-yet-repeatable shapes (charwise visual ops): the repeat then
    /// bells instead of replaying stale input.
    pub(super) fn parse_repeat(bits: &[Bit], block: Option<BlockPending>) -> Option<Repeat> {
        let tokens = RepeatTokens::new(bits);
        match tokens.visual_marker() {
            Some(true) => tokens.block(block),
            Some(false) => None,
            None => tokens.change(),
        }
    }

    /// Replay a parsed change at the current cursor. Recording stays off
    /// (`replaying`), so the replay never becomes the new last change;
    /// undo grouping still applies, so a replayed change undoes as one.
    pub(super) fn exec_repeat<B: Buffer>(&mut self, doc: &mut B, change: Repeat) -> Vec<Effect> {
        self.replaying = true;
        self.replay_change(doc, change);
        self.replaying = false;
        self.clear_pending();
        Vec::new()
    }

    fn replay_change<B: Buffer>(&mut self, doc: &mut B, change: Repeat) {
        match change {
            Repeat::Insert { text, place } => {
                self.start_insert_place(doc, place);
                self.type_text(doc, &text);
                self.enter_normal(doc);
            }
            Repeat::DeleteChars { count } => self.delete_chars_forward(doc, count),
            Repeat::DeleteMotion { motion, count } => {
                self.pending = Some(PendingOp::Delete);
                self.replay_motion(doc, motion, count);
            }
            Repeat::DeleteLines { count } => self.delete_lines(doc, count),
            Repeat::ChangeMotion {
                motion,
                count,
                text,
            } => {
                self.begin_change_group(doc);
                self.pending = Some(PendingOp::Change);
                self.replay_motion(doc, motion, count);
                self.type_text(doc, &text);
                self.enter_normal(doc);
            }
            Repeat::ChangeLines { count, text } => {
                self.begin_change_group(doc);
                self.change_line(doc, count);
                self.type_text(doc, &text);
                self.enter_normal(doc);
            }
            Repeat::BlockDelete { .. }
            | Repeat::BlockInsert { .. }
            | Repeat::BlockChange { .. } => {
                self.replay_block(doc, change);
            }
            Repeat::Paste { count } => {
                for _ in 0..count.max(1) {
                    self.paste(doc);
                }
            }
        }
    }

    /// Blockwise delete, insert, or change (delete then insert) from the
    /// cursor line, as one undo unit; the caret parks at the top-left.
    fn replay_block<B: Buffer>(&mut self, doc: &mut B, change: Repeat) {
        let (replay, text, delete) = match change {
            Repeat::BlockDelete {
                height,
                left,
                right,
            } => (
                BlockReplay {
                    height,
                    left,
                    right,
                    append: false,
                },
                None,
                true,
            ),
            Repeat::BlockInsert {
                text,
                height,
                left,
                append,
            } => (
                BlockReplay {
                    height,
                    left,
                    right: left,
                    append,
                },
                Some(text),
                false,
            ),
            Repeat::BlockChange {
                text,
                height,
                left,
                right,
                append,
            } => (
                BlockReplay {
                    height,
                    left,
                    right,
                    append,
                },
                Some(text),
                true,
            ),
            _ => return,
        };
        let block = self.block_at_cursor(doc, replay);
        doc.begin_undo_group();
        if delete {
            self.delete_block_rect(doc, &block);
        }
        if let Some(text) = &text {
            self.apply_block_insert(doc, &block, text);
        }
        doc.end_undo_group();
        self.cursor = Cursor {
            line: block.top,
            col: block.left.min(line_len(doc, block.top)),
        };
        if text.is_none() {
            self.preferred_col = self.cursor.col;
        }
        self.clamp(doc);
    }

    /// The replayed rectangle: `height` rows below the cursor line
    /// (clamped to the buffer).
    fn block_at_cursor<B: Buffer>(&self, doc: &B, replay: BlockReplay) -> BlockPending {
        let BlockReplay {
            height,
            left,
            right,
            append,
        } = replay;
        let top = self.cursor.line;
        let bottom = top
            .saturating_add(height)
            .min(doc.line_count().saturating_sub(1));
        BlockPending {
            top,
            bottom,
            left,
            right,
            append,
            skip_first: false,
        }
    }

    /// Delete the rectangle's columns `left..=right` on every row.
    fn delete_block_rect<B: Buffer>(&mut self, doc: &mut B, block: &BlockPending) {
        self.delete_block_ranges(
            doc,
            DeleteBlockRangesInput {
                top: block.top,
                bottom: block.bottom,
                left: block.left,
                right_excl: block.right.saturating_add(1),
            },
        );
    }
}
