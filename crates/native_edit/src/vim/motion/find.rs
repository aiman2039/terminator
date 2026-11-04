//! `f`/`t`/`F`/`T` and their repeats `;` and `,` (within the line).
use super::super::mode::Cursor;
use super::super::state::VimEngine;
use super::{MKind, Motion};

/// Column of the `count`th `ch` after (`f`/`t`) or before (`F`/`T`)
/// `col`; `t`/`T` stop one short. `None` when the line has too few.
pub(crate) fn find_in_line(
    line: &str,
    col: usize,
    (kind, ch): (char, char),
    count: usize,
) -> Option<usize> {
    let chars: Vec<char> = line.chars().collect();
    let forward = matches!(kind, 'f' | 't');
    let hit = if forward {
        (col.saturating_add(1)..chars.len())
            .filter(|i| chars.get(*i) == Some(&ch))
            .nth(count.saturating_sub(1))?
    } else {
        (0..col)
            .rev()
            .filter(|i| chars.get(*i) == Some(&ch))
            .nth(count.saturating_sub(1))?
    };
    Some(match kind {
        't' => hit.saturating_sub(1),
        'T' => hit.saturating_add(1),
        _ => hit,
    })
}

impl VimEngine {
    pub(super) fn find_motion(
        &mut self,
        line: &str,
        (kind, ch): (char, char),
        (count, remember): (usize, bool),
    ) -> Option<Motion> {
        if remember {
            self.x.last_find = Some((kind, ch));
        }
        let col = find_in_line(line, self.cursor.col, (kind, ch), count)?;
        let kind = if matches!(kind, 'f' | 't') {
            MKind::Inclusive
        } else {
            MKind::Exclusive
        };
        Some(Motion::new(
            Cursor {
                line: self.cursor.line,
                col,
            },
            kind,
        ))
    }

    /// `;` repeats the last find, `,` reverses it. A repeated `t`/`T`
    /// skips the char it already stands before, like vim.
    pub(super) fn find_again(&mut self, line: &str, reverse: bool, count: usize) -> Option<Motion> {
        let (kind, ch) = self.x.last_find?;
        let kind = if reverse {
            match kind {
                'f' => 'F',
                'F' => 'f',
                't' => 'T',
                _ => 't',
            }
        } else {
            kind
        };
        let saved = self.cursor.col;
        let nudge = match kind {
            't' => saved.saturating_add(1),
            'T' => saved.saturating_sub(1),
            _ => saved,
        };
        self.cursor.col = nudge;
        let motion = self.find_motion(line, (kind, ch), (count, false));
        self.cursor.col = saved;
        motion
    }
}
