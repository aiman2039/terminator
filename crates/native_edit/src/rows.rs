//! Display layout for the source view: one painted row per logical line,
//! or the wrapped pieces of a long line, plus the display-cell math the
//! painter and the pointer mapping share.
//!
//! Rows derive from the buffer revision and the pane's wrap width only, so
//! [`RowCache`] keeps them across frames: slicing every line and scanning
//! the wrap breaks is the per-frame cost on a large file, and it changes
//! only when the text or the pane does.
use crate::doc::Buffer;

/// One painted row: a logical line, or the slice of one that fits the
/// wrap width. `start`/`end` are char columns inside `line`; `text` is
/// the slice itself, so painting never re-slices the buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisualRow {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub line_len: usize,
    pub text: String,
}

impl VisualRow {
    /// True when this row owns the cursor at `line`/`col`.
    #[must_use]
    pub fn owns_cursor(&self, line: usize, col: usize) -> bool {
        self.line == line
            && col >= self.start
            && (col < self.end || (col == self.end && self.end == self.line_len))
    }
}

/// Revision-keyed row layout cache: one per open buffer, owned next to the
/// buffer it derives from.
#[derive(Default)]
pub struct RowCache {
    revision: u64,
    columns: Option<usize>,
    ready: bool,
    rows: Vec<VisualRow>,
    max_cells: usize,
}

impl RowCache {
    /// Recompute when the buffer revision or the wrap width changed; every
    /// other frame reuses the rows as-is.
    pub fn ensure<B: Buffer>(&mut self, doc: &B, wrap_columns: Option<usize>) {
        if self.ready && self.revision == doc.revision() && self.columns == wrap_columns {
            return;
        }
        self.rows = visual_rows(doc, wrap_columns);
        self.max_cells = self.rows.iter().map(row_cells).max().unwrap_or(0);
        self.revision = doc.revision();
        self.columns = wrap_columns;
        self.ready = true;
    }

    /// Laid-out rows for the current revision and wrap width.
    #[must_use]
    pub fn rows(&self) -> &[VisualRow] {
        &self.rows
    }

    /// Widest row in display cells, for the horizontal scroll extent.
    #[must_use]
    pub fn max_cells(&self) -> usize {
        self.max_cells
    }
}

/// Display cells one row occupies, its wrap offset included.
fn row_cells(row: &VisualRow) -> usize {
    row.start
        .saturating_add(display_x(&row.text, row.text.chars().count()))
}

/// Lay out buffer rows: one row per line, or the wrapped pieces of a line
/// when `wrap_columns` is set. A break prefers the last space that fits, so
/// words stay whole when the pane is wide enough for them.
pub fn visual_rows<B: Buffer>(doc: &B, wrap_columns: Option<usize>) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    for line in 0..doc.line_count() {
        let text = doc.line_text(line);
        let chars: Vec<char> = text.chars().collect();
        let line_len = chars.len();
        if line_len == 0 || wrap_columns.is_none() {
            rows.push(VisualRow {
                line,
                start: 0,
                end: line_len,
                line_len,
                text,
            });
            continue;
        }
        let limit = wrap_columns.unwrap_or(usize::MAX).max(1);
        let mut start = 0;
        while start < line_len {
            let mut end = start;
            let mut cells = 0usize;
            let mut last_space = None;
            while end < line_len {
                let Some(ch) = chars.get(end).copied() else {
                    break;
                };
                let width = char_display_width(ch);
                if end > start && cells.saturating_add(width) > limit {
                    break;
                }
                cells = cells.saturating_add(width);
                end = end.saturating_add(1);
                if ch.is_whitespace() {
                    last_space = Some(end);
                }
            }
            if end < line_len
                && let Some(space) = last_space.filter(|space| *space > start)
            {
                end = space;
            }
            rows.push(VisualRow {
                line,
                start,
                end,
                line_len,
                text: chars.get(start..end).unwrap_or(&[]).iter().collect(),
            });
            start = end;
        }
    }
    rows
}

/// Display cells of one char: East Asian wide chars take two columns,
/// everything else (including tabs, kept single-cell like the engine)
/// takes one. Control chars fall back to one cell.
#[must_use]
pub fn char_display_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c)
        .unwrap_or(1)
        .max(1)
}

/// Display-column x (in cells) of char column `col`.
#[must_use]
pub fn display_x(text: &str, col: usize) -> usize {
    text.chars()
        .take(col)
        .map(char_display_width)
        .fold(0usize, |acc, w| acc.saturating_add(w))
}

/// Char column at display cell `x` (floor: a click on the second cell of
/// a wide char lands on that char).
#[must_use]
pub fn col_at_display_x(text: &str, x: usize) -> usize {
    let mut acc = 0usize;
    let mut col = 0usize;
    for c in text.chars() {
        let next = acc.saturating_add(char_display_width(c));
        if next > x {
            break;
        }
        acc = next;
        col = col.saturating_add(1);
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;

    #[test]
    fn row_cache_reuses_rows_until_the_buffer_or_width_changes() {
        let mut doc = Doc::new("alpha\nbeta\n");
        let mut cache = RowCache::default();
        cache.ensure(&doc, None);
        let first = cache.rows().to_vec();
        assert_eq!(first.len(), doc.line_count());
        assert_eq!(cache.max_cells(), "alpha".chars().count());

        // Same revision and width: the rows are reused, not rebuilt.
        cache.ensure(&doc, None);
        assert_eq!(cache.rows(), first.as_slice());

        doc.insert(doc.char_at_line_col(1, 4), "!");
        cache.ensure(&doc, None);
        assert_ne!(cache.rows(), first.as_slice());
        assert_eq!(cache.max_cells(), "beta!".chars().count());

        // A narrower wrap width re-lays the rows out.
        cache.ensure(&doc, Some(2));
        assert!(cache.rows().len() > doc.line_count());
    }

    #[test]
    fn wrapped_rows_preserve_source_and_cursor_columns() {
        let doc = Doc::new("# Heading\n- Long words remain in one logical line\n");
        let rows = visual_rows(&doc, Some(10));
        for line in 0..doc.line_count() {
            let joined: String = rows
                .iter()
                .filter(|row| row.line == line)
                .map(|row| row.text.as_str())
                .collect();
            assert_eq!(joined, doc.line_text(line));
        }
        assert!(rows.iter().filter(|row| row.line == 1).count() > 1);
        for col in 0..=doc.line_text(1).chars().count() {
            assert_eq!(
                rows.iter().filter(|row| row.owns_cursor(1, col)).count(),
                1,
                "cursor column {col} must have one visual row"
            );
        }
        assert!(
            rows.iter()
                .all(|row| display_x(&row.text, row.text.chars().count()) <= 10)
        );
    }
}
