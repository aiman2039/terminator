//! Host filler rows: a diff viewer aligns this buffer with another pane by
//! asking for empty rows before some lines (`(line, rows)`, sorted by
//! line). The rows exist only in the painted layout; the buffer never
//! sees them, and they never own the cursor.
use crate::fold::Folds;
use crate::rows::VisualRow;

/// Cache key of a spacer list; `0` for none.
#[must_use]
pub fn spacer_key(spacers: &[(usize, usize)]) -> u64 {
    use std::hash::{Hash, Hasher};
    if spacers.is_empty() {
        return 0;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    spacers.hash(&mut hasher);
    hasher.finish().max(1)
}

/// `rows` with `count` filler rows before the first row of each listed
/// line. Lines past the end get their fillers after the last row; lines a
/// fold hides get none.
#[must_use]
pub fn insert_spacers(
    rows: Vec<VisualRow>,
    spacers: &[(usize, usize)],
    folds: Option<&Folds>,
) -> Vec<VisualRow> {
    if spacers.is_empty() {
        return rows;
    }
    let extra: usize = spacers.iter().map(|(_, count)| *count).sum();
    let mut out = Vec::with_capacity(rows.len().saturating_add(extra));
    let mut pending = spacers.iter().peekable();
    let mut last_line = 0;
    for row in rows {
        if row.start == 0 {
            while let Some((line, count)) = pending.next_if(|(line, _)| *line <= row.line) {
                if !folds.is_some_and(|folds| folds.hidden(*line)) {
                    push_fillers(&mut out, row.line, *count);
                }
            }
        }
        last_line = row.line;
        out.push(row);
    }
    for (_, count) in pending {
        push_fillers(&mut out, last_line, *count);
    }
    out
}

fn push_fillers(out: &mut Vec<VisualRow>, line: usize, count: usize) {
    out.extend((0..count).map(|_| VisualRow {
        line,
        start: 0,
        end: 0,
        line_len: 0,
        text: String::new(),
        folded: false,
        spacer: true,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::{Buffer, Doc};
    use crate::rows::visual_rows;

    fn layout(text: &str, spacers: &[(usize, usize)]) -> Vec<VisualRow> {
        insert_spacers(visual_rows(&Doc::new(text), None), spacers, None)
    }

    #[test]
    fn spacers_push_later_lines_down_by_their_count() {
        let rows = layout("a\nb\nc", &[(1, 2), (2, 1)]);
        let shape: Vec<(usize, bool)> = rows.iter().map(|row| (row.line, row.spacer)).collect();
        assert_eq!(
            shape,
            [
                (0, false),
                (1, true),
                (1, true),
                (1, false),
                (2, true),
                (2, false)
            ]
        );
    }

    #[test]
    fn spacers_past_the_end_follow_the_last_row() {
        let doc = Doc::new("a\nb");
        let rows = insert_spacers(visual_rows(&doc, None), &[(doc.line_count(), 3)], None);
        assert_eq!(rows.len(), doc.line_count() + 3);
        assert!(rows.iter().skip(doc.line_count()).all(|row| row.spacer));
    }

    #[test]
    fn spacer_rows_never_own_the_cursor() {
        let rows = layout("", &[(0, 1)]);
        let owners = rows.iter().filter(|row| row.owns_cursor(0, 0)).count();
        assert_eq!(owners, 1, "only the real row owns the caret");
        assert!(rows.first().is_some_and(|row| row.spacer));
    }

    #[test]
    fn spacer_key_changes_with_the_list() {
        assert_eq!(spacer_key(&[]), 0);
        assert_ne!(spacer_key(&[(1, 2)]), spacer_key(&[(1, 3)]));
        assert_eq!(spacer_key(&[(4, 1)]), spacer_key(&[(4, 1)]));
    }
}
