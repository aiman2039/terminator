use super::super::doc::Buffer;
use super::mode::{Bit, BlockPending, Cursor, Effect, Key, ModalEngine, PendingOp, Place, Repeat};
use super::state::VimEngine;
use super::words::line_len;
impl VimEngine {
    /// Parse recorded bits into the last change. Returns `None` for
    /// not-yet-repeatable shapes (charwise visual ops): the repeat then
    /// bells instead of replaying stale input.
    pub(super) fn parse_repeat(bits: &[Bit], block: Option<BlockPending>) -> Option<Repeat> {
        #[derive(PartialEq)]
        enum Tok {
            Count(usize),
            Op(char),
            Motion(char),
            Key(Key),
        }
        let mut toks: Vec<Tok> = Vec::new();
        // Typed text joins separately below; only key order matters here.
        let mut texts: Vec<String> = Vec::new();
        for bit in bits {
            match bit {
                Bit::Key(Key::Char(c)) if c.is_ascii_digit() => {
                    // A leading `0` is the line-start motion (the count
                    // parser never emits one); digits after an operator or
                    // motion are a second count. Both multiply vim-style.
                    if *c == '0' && !matches!(toks.last(), Some(Tok::Count(_))) {
                        toks.push(Tok::Motion('0'));
                    } else {
                        let digit = (*c as usize).saturating_sub('0' as usize);
                        if let Some(Tok::Count(n)) = toks.last_mut() {
                            *n = n.saturating_mul(10).saturating_add(digit);
                        } else {
                            toks.push(Tok::Count(digit));
                        }
                    }
                }
                Bit::Key(key) => toks.push(Tok::Key(*key)),
                Bit::Op(op) => toks.push(Tok::Op(*op)),
                Bit::Motion(motion) => toks.push(Tok::Motion(*motion)),
                Bit::Type(chunk) => texts.push(chunk.clone()),
            }
        }
        let typed = texts.concat();
        let mut pos = 0usize;
        let mut count = 1usize;
        while let Some(Tok::Count(n)) = toks.get(pos) {
            count = count.saturating_mul(*n).max(1);
            pos = pos.saturating_add(1);
        }
        // Tokens after the leading count use indexed `.get()`, never slicing.
        let tail_len = toks.len().saturating_sub(pos);
        // A block session carries the Ctrl-V toggle; dims ride along from
        // the live pending state. The LAST visual marker wins, so entering
        // the block from charwise visual still replays as a block.
        let marker = toks
            .iter()
            .rfind(|tok| matches!(tok, Tok::Key(Key::CtrlV | Key::Char('v' | 'V'))));
        if marker.is_some() && !matches!(marker, Some(Tok::Key(Key::CtrlV))) {
            return None;
        }
        if matches!(marker, Some(Tok::Key(Key::CtrlV))) {
            let block = block?;
            let height = block.bottom.saturating_sub(block.top);
            let is_change = toks
                .iter()
                .any(|tok| matches!(tok, Tok::Key(Key::Char('c'))));
            if typed.is_empty() {
                return None;
            }
            return if is_change {
                Some(Repeat::BlockChange {
                    text: typed,
                    height,
                    left: block.left,
                    right: block.right,
                    append: block.append,
                })
            } else {
                Some(Repeat::BlockInsert {
                    text: typed,
                    height,
                    left: block.left,
                    append: block.append,
                })
            };
        }
        let head = toks.get(pos)?;
        match head {
            Tok::Key(Key::Char('x')) if tail_len == 1 => Some(Repeat::DeleteChars { count }),
            Tok::Key(Key::Char('p')) if tail_len == 1 => Some(Repeat::Paste { count }),
            Tok::Key(Key::Char(place @ ('i' | 'a' | 'A' | 'I' | 'o' | 'O'))) => {
                if tail_len != 1 || typed.is_empty() {
                    return None;
                }
                let place = match place {
                    'i' => Place::AtCursor,
                    'a' => Place::AfterCursor,
                    'A' => Place::LineEnd,
                    'I' => Place::FirstBlank,
                    'o' => Place::LineBelow,
                    _ => Place::LineAbove,
                };
                Some(Repeat::Insert { text: typed, place })
            }
            Tok::Op(op @ ('d' | 'c')) => {
                let mut off = 1usize;
                let mut count2 = 1usize;
                while let Some(Tok::Count(n)) = toks.get(pos.saturating_add(off)) {
                    count2 = count2.saturating_mul(*n).max(1);
                    off = off.saturating_add(1);
                }
                let total = count.saturating_mul(count2).max(1);
                // The change must end exactly here: no trailing tokens.
                let ends_here = toks.len() == pos.saturating_add(off).saturating_add(1);
                match toks.get(pos.saturating_add(off)) {
                    Some(Tok::Op(second)) if second == op && ends_here => {
                        if *op == 'd' {
                            Some(Repeat::DeleteLines { count: total })
                        } else {
                            // `cc` + immediate Escape deletes the lines; an
                            // empty insert replays as nothing to type.
                            Some(Repeat::ChangeLines {
                                count: total,
                                text: typed,
                            })
                        }
                    }
                    Some(Tok::Motion(motion)) if ends_here => {
                        // A bare `G` (count 0) means last line; every other
                        // motion defaults an absent count to 1.
                        let for_g = if *motion == 'G' && count == 1 && count2 == 1 {
                            0
                        } else {
                            total
                        };
                        if *op == 'd' {
                            Some(Repeat::DeleteMotion {
                                motion: *motion,
                                count: for_g,
                            })
                        } else {
                            Some(Repeat::ChangeMotion {
                                motion: *motion,
                                count: for_g,
                                text: typed,
                            })
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Replay a parsed change at the current cursor. Recording stays off
    /// (`replaying`), so the replay never becomes the new last change;
    /// undo grouping still applies, so a replayed change undoes as one.
    pub(super) fn exec_repeat<B: Buffer>(&mut self, doc: &mut B, change: Repeat) -> Vec<Effect> {
        self.replaying = true;
        let out = match change {
            Repeat::Insert { text, place } => {
                self.start_insert_place(doc, place);
                self.type_text(doc, &text);
                self.enter_normal(doc);
                Vec::new()
            }
            Repeat::DeleteChars { count } => {
                self.delete_chars_forward(doc, count);
                Vec::new()
            }
            Repeat::DeleteMotion { motion, count } => {
                self.pending = Some(PendingOp::Delete);
                self.replay_motion(doc, motion, count);
                Vec::new()
            }
            Repeat::DeleteLines { count } => {
                self.delete_lines(doc, count);
                Vec::new()
            }
            Repeat::BlockDelete {
                height,
                left,
                right,
            } => {
                let top = self.cursor.line;
                let bottom = top
                    .saturating_add(height)
                    .min(doc.line_count().saturating_sub(1));
                doc.begin_undo_group();
                self.delete_block_ranges(doc, top, bottom, left, right.saturating_add(1));
                doc.end_undo_group();
                self.cursor = Cursor {
                    line: top,
                    col: left.min(line_len(doc, top)),
                };
                self.preferred_col = self.cursor.col;
                self.clamp(doc);
                Vec::new()
            }
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
                Vec::new()
            }
            Repeat::ChangeLines { count, text } => {
                self.begin_change_group(doc);
                self.change_line(doc, count);
                self.type_text(doc, &text);
                self.enter_normal(doc);
                Vec::new()
            }
            Repeat::BlockInsert {
                text,
                height,
                left,
                append,
            } => {
                let top = self.cursor.line;
                let bottom = top
                    .saturating_add(height)
                    .min(doc.line_count().saturating_sub(1));
                let block = BlockPending {
                    top,
                    bottom,
                    left,
                    right: left,
                    append,
                    skip_first: false,
                };
                doc.begin_undo_group();
                self.apply_block_insert(doc, &block, &text);
                doc.end_undo_group();
                self.cursor = Cursor {
                    line: top,
                    col: left.min(line_len(doc, top)),
                };
                self.clamp(doc);
                Vec::new()
            }
            Repeat::BlockChange {
                text,
                height,
                left,
                right,
                append,
            } => {
                let top = self.cursor.line;
                let bottom = top
                    .saturating_add(height)
                    .min(doc.line_count().saturating_sub(1));
                let block = BlockPending {
                    top,
                    bottom,
                    left,
                    right,
                    append,
                    skip_first: false,
                };
                doc.begin_undo_group();
                self.delete_block_ranges(doc, top, bottom, left, right.saturating_add(1));
                self.apply_block_insert(doc, &block, &text);
                doc.end_undo_group();
                self.cursor = Cursor {
                    line: top,
                    col: left.min(line_len(doc, top)),
                };
                self.clamp(doc);
                Vec::new()
            }
            Repeat::Paste { count } => {
                for _ in 0..count.max(1) {
                    self.paste(doc);
                }
                Vec::new()
            }
        };
        self.replaying = false;
        self.clear_pending();
        out
    }
}
