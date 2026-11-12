//! Motions: one resolver serves plain moves, operator targets and Visual.
pub(crate) mod delims;
mod find;
mod jumps;
pub(crate) mod objects;
mod para;
mod screen;
pub(crate) mod stream;

use super::super::doc::Buffer;
use super::helpers::{grapheme_left, grapheme_right, last_grapheme};
use super::mode::Cursor;
use super::state::VimEngine;
use super::words::{line_len, word_backward, word_end, word_end_back, word_forward};
use stream::Stream;

/// One motion command, before it is resolved against the buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mo {
    Left,
    Right,
    Down,
    Up,
    Word {
        big: bool,
    },
    Back {
        big: bool,
    },
    End {
        big: bool,
    },
    EndBack {
        big: bool,
    },
    /// `cw` on a word: to the end of the current word.
    ChangeWord {
        big: bool,
    },
    LineStart,
    LineEnd,
    FirstNonBlank,
    /// `_`: `count - 1` lines down, linewise.
    CurrentLine,
    /// `+` / Enter and `-`: first non-blank of a line below / above.
    NextLine,
    PrevLine,
    /// `|`: screen column `count`.
    Column,
    /// `G` (`last`) or `gg`.
    GotoLine {
        last: bool,
    },
    Find {
        kind: char,
        ch: char,
    },
    FindAgain {
        reverse: bool,
    },
    MatchPair,
    ParaFwd,
    ParaBack,
    ScreenTop,
    ScreenMid,
    ScreenBottom,
    /// `n` / `N` (`reverse` flips the last search direction).
    Search {
        reverse: bool,
    },
    /// `*` / `#` (`whole`: `\<word\>`), and `g*` / `g#`.
    Star {
        forward: bool,
        whole: bool,
    },
    Mark {
        name: char,
        exact: bool,
    },
}

impl Mo {
    /// The recorded motion char for `.` replay of the classic shapes;
    /// `None` makes the change repeat key by key.
    pub(crate) fn legacy(self) -> Option<char> {
        Some(match self {
            Mo::Left => 'h',
            Mo::Right => 'l',
            Mo::Down => 'j',
            Mo::Up => 'k',
            // `cw` records as `w`: the replay goes through the same `cw`
            // special case, so `.` on a word's last char changes only it.
            Mo::Word { big: false } | Mo::ChangeWord { big: false } => 'w',
            Mo::Back { big: false } => 'b',
            Mo::End { big: false } => 'e',
            Mo::LineStart => '0',
            Mo::LineEnd => '$',
            Mo::FirstNonBlank => '^',
            Mo::GotoLine { last: true } => 'G',
            Mo::GotoLine { last: false } => 'g',
            _ => return None,
        })
    }

    /// Motion for a recorded legacy char (the inverse of [`Mo::legacy`]).
    pub(crate) fn from_legacy(c: char) -> Option<Self> {
        Some(match c {
            'h' => Mo::Left,
            'l' => Mo::Right,
            'j' => Mo::Down,
            'k' => Mo::Up,
            'w' => Mo::Word { big: false },
            'b' => Mo::Back { big: false },
            'e' => Mo::End { big: false },
            '0' => Mo::LineStart,
            '$' => Mo::LineEnd,
            '^' => Mo::FirstNonBlank,
            'G' => Mo::GotoLine { last: true },
            'g' => Mo::GotoLine { last: false },
            _ => return None,
        })
    }
}

/// How an operator treats a motion's span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MKind {
    Exclusive,
    Inclusive,
    Linewise,
}

/// A resolved motion: where it lands and how operators read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Motion {
    pub(crate) target: Cursor,
    pub(crate) kind: MKind,
    /// Adds to the jump list (`G`, `/`, `%`, …).
    pub(crate) jump: bool,
    /// Vertical move: keep the preferred column.
    pub(crate) keep_col: bool,
}

impl Motion {
    fn new(target: Cursor, kind: MKind) -> Self {
        Self {
            target,
            kind,
            jump: false,
            keep_col: false,
        }
    }

    fn jump(mut self) -> Self {
        self.jump = true;
        self
    }
}

/// Count and whether it was typed, for a resolve.
#[derive(Clone, Copy)]
pub(crate) struct Count {
    pub(crate) n: usize,
    pub(crate) typed: bool,
}

impl VimEngine {
    /// Where `mo` lands from the cursor; `None` when it cannot move.
    pub(crate) fn resolve<B: Buffer>(&mut self, doc: &B, mo: Mo, count: Count) -> Option<Motion> {
        let n = count.n.max(1);
        let here = self.cursor;
        let line_text = doc.line_text(here.line);
        let at = |col| Cursor {
            line: here.line,
            col,
        };
        Some(match mo {
            Mo::Left => Motion::new(at(grapheme_left(&line_text, here.col, n)), MKind::Exclusive),
            Mo::Right => Motion::new(
                at(grapheme_right(&line_text, here.col, n)),
                MKind::Exclusive,
            ),
            Mo::Down | Mo::Up => return self.vertical(doc, mo == Mo::Down, n),
            Mo::Word { .. } | Mo::Back { .. } | Mo::End { .. } | Mo::EndBack { .. } => {
                self.word_motion(doc, mo, n)
            }
            Mo::ChangeWord { big } => {
                let mut s = Stream::new(doc);
                let from = self.char_idx(doc, here);
                let end = super::words::change_word_end(&mut s, from, n, big);
                Motion::new(Self::cursor_at(doc, end), MKind::Inclusive)
            }
            Mo::LineStart => Motion::new(at(0), MKind::Exclusive),
            Mo::LineEnd => {
                let line = self.line_down(doc, n.saturating_sub(1));
                let col = last_grapheme(&doc.line_text(line));
                Motion::new(Cursor { line, col }, MKind::Inclusive)
            }
            Mo::FirstNonBlank => {
                Motion::new(at(Self::first_non_blank(doc, here.line)), MKind::Exclusive)
            }
            Mo::CurrentLine | Mo::NextLine | Mo::PrevLine => return self.line_motion(doc, mo, n),
            Mo::Column => Motion::new(
                at(n.saturating_sub(1).min(line_len(doc, here.line))),
                MKind::Exclusive,
            ),
            Mo::GotoLine { last } => {
                let max = Self::last_addressable(doc);
                let line = match (count.typed, last) {
                    (true, _) => n.saturating_sub(1).min(max),
                    (false, true) => max,
                    (false, false) => 0,
                };
                let col = Self::first_non_blank(doc, line);
                Motion::new(Cursor { line, col }, MKind::Linewise).jump()
            }
            Mo::Find { kind, ch } => return self.find_motion(&line_text, (kind, ch), (n, true)),
            Mo::FindAgain { reverse } => return self.find_again(&line_text, reverse, n),
            Mo::MatchPair => return self.match_pair_motion(doc, count),
            Mo::ParaFwd | Mo::ParaBack => self.para_motion(doc, mo == Mo::ParaFwd, n),
            Mo::ScreenTop | Mo::ScreenMid | Mo::ScreenBottom => self.screen_motion(doc, mo, n),
            Mo::Search { reverse } => return self.search_motion(doc, reverse, n),
            Mo::Star { forward, whole } => return self.star_motion(doc, (forward, whole), n),
            Mo::Mark { name, exact } => return self.mark_motion(doc, name, exact),
        })
    }

    /// `count` lines below the cursor, clamped to the last real line.
    fn line_down<B: Buffer>(&self, doc: &B, count: usize) -> usize {
        self.cursor
            .line
            .saturating_add(count)
            .min(Self::last_addressable(doc))
    }

    pub(crate) fn cursor_at<B: Buffer>(doc: &B, idx: usize) -> Cursor {
        let (line, col) = doc.line_col_at(idx.min(doc.len_chars()));
        Cursor { line, col }
    }

    fn vertical<B: Buffer>(&self, doc: &B, down: bool, n: usize) -> Option<Motion> {
        let line = if down {
            self.line_down(doc, n)
        } else {
            self.cursor.line.saturating_sub(n)
        };
        // Under an operator a move that cannot happen fails (`dj` on the
        // last line deletes nothing).
        if line == self.cursor.line && self.pending.is_some() {
            return None;
        }
        let col = self.preferred_col.min(line_len(doc, line));
        let mut motion = Motion::new(Cursor { line, col }, MKind::Linewise);
        motion.keep_col = true;
        Some(motion)
    }

    fn line_motion<B: Buffer>(&self, doc: &B, mo: Mo, n: usize) -> Option<Motion> {
        let line = match mo {
            Mo::CurrentLine => self.line_down(doc, n.saturating_sub(1)),
            Mo::NextLine => {
                let line = self.line_down(doc, n);
                if line == self.cursor.line {
                    return None;
                }
                line
            }
            _ => self.cursor.line.checked_sub(n)?,
        };
        let col = Self::first_non_blank(doc, line);
        Some(Motion::new(Cursor { line, col }, MKind::Linewise))
    }

    fn word_motion<B: Buffer>(&self, doc: &B, mo: Mo, n: usize) -> Motion {
        let mut s = Stream::new(doc);
        let from = self.char_idx(doc, self.cursor);
        let (idx, kind) = match mo {
            Mo::Word { big } => (word_forward(&mut s, from, n, big), MKind::Exclusive),
            Mo::Back { big } => (word_backward(&mut s, from, n, big), MKind::Exclusive),
            Mo::End { big } => (word_end(&mut s, from, n, big), MKind::Inclusive),
            Mo::EndBack { big } => (word_end_back(&mut s, from, n, big), MKind::Inclusive),
            _ => (from, MKind::Exclusive),
        };
        Motion::new(Self::cursor_at(doc, idx.min(s.len())), kind)
    }
}
