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
    /// Last row of a folded region's first line: paint the fold marker.
    pub folded: bool,
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
    folds: u64,
    ready: bool,
    rows: Vec<VisualRow>,
    max_cells: usize,
}

impl RowCache {
    /// Recompute when the buffer revision or the wrap width changed; every
    /// other frame reuses the rows as-is.
    pub fn ensure<B: Buffer>(
        &mut self,
        doc: &B,
        wrap_columns: Option<usize>,
        folds: Option<&crate::fold::Folds>,
    ) {
        let fold_generation = folds.map_or(0, crate::fold::Folds::generation);
        if self.ready
            && self.revision == doc.revision()
            && self.columns == wrap_columns
            && self.folds == fold_generation
        {
            return;
        }
        self.folds = fold_generation;
        self.rows = visual_rows_folded(doc, wrap_columns, folds);
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
    visual_rows_folded(doc, wrap_columns, None)
}

/// [`visual_rows`] without the lines folded regions hide; a folded
/// region's first line is marked for its placeholder.
pub fn visual_rows_folded<B: Buffer>(
    doc: &B,
    wrap_columns: Option<usize>,
    folds: Option<&crate::fold::Folds>,
) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    for line in 0..doc.line_count() {
        if folds.is_some_and(|folds| folds.hidden(line)) {
            continue;
        }
        let first = rows.len();
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
                folded: false,
            });
            mark_folded(MarkFoldedInput {
                rows: &mut rows,
                first,
                line,
                folds,
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
                let next = advance_cell(cells, ch);
                if end > start && next > limit {
                    break;
                }
                cells = next;
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
                folded: false,
            });
            start = end;
        }
        mark_folded(MarkFoldedInput {
            rows: &mut rows,
            first,
            line,
            folds,
        });
    }
    rows
}

struct MarkFoldedInput<'a> {
    rows: &'a mut [VisualRow],
    first: usize,
    line: usize,
    folds: Option<&'a crate::fold::Folds>,
}

/// Flag the last row of `line` when a folded region starts there.
fn mark_folded(input: MarkFoldedInput<'_>) {
    let MarkFoldedInput {
        rows,
        first,
        line,
        folds,
    } = input;
    if folds.is_some_and(|folds| folds.is_folded(line))
        && rows.len() > first
        && let Some(last) = rows.last_mut()
    {
        last.folded = true;
    }
}

/// Tab stop width in display cells.
pub const TAB_WIDTH: usize = 4;

/// Display cells of one char outside a tab: East Asian wide chars take two
/// columns, everything else takes one. Control chars fall back to one
/// cell. Tabs are position-dependent; see [`advance_cell`].
#[must_use]
pub fn char_display_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c)
        .unwrap_or(1)
        .max(1)
}

/// Display cell after placing `c` at cell `at`: a tab advances to the
/// next [`TAB_WIDTH`] stop, every other char by its own width.
#[must_use]
pub fn advance_cell(at: usize, c: char) -> usize {
    if c == '\t' {
        at.saturating_add(TAB_WIDTH.saturating_sub(at % TAB_WIDTH))
    } else {
        at.saturating_add(char_display_width(c))
    }
}

/// Display-column x (in cells) of char column `col`.
#[must_use]
pub fn display_x(text: &str, col: usize) -> usize {
    text.chars().take(col).fold(0usize, advance_cell)
}

/// `text` as painted when it starts at display cell `at`: tabs become the
/// spaces that reach their stop, so glyph advances match [`display_x`].
#[must_use]
pub fn expand_tabs(text: &str, at: usize) -> std::borrow::Cow<'_, str> {
    if !text.contains('\t') {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len().saturating_add(TAB_WIDTH));
    let mut cell = at;
    for c in text.chars() {
        let next = advance_cell(cell, c);
        if c == '\t' {
            out.extend(std::iter::repeat_n(' ', next.saturating_sub(cell)));
        } else {
            out.push(c);
        }
        cell = next;
    }
    std::borrow::Cow::Owned(out)
}

/// Char column at display cell `x` (floor: a click on the second cell of
/// a wide char or inside a tab lands on that char).
#[must_use]
pub fn col_at_display_x(text: &str, x: usize) -> usize {
    let mut acc = 0usize;
    let mut col = 0usize;
    for c in text.chars() {
        let next = advance_cell(acc, c);
        if next > x {
            break;
        }
        acc = next;
        col = col.saturating_add(1);
    }
    col
}

/// An inline hint on one row: `label` is painted before the char at
/// row-local column `col` and takes `cells` display cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowHint<'a> {
    pub col: usize,
    pub cells: usize,
    pub label: &'a str,
}

/// The hints of `hints` (sorted by line and column) that fall on `row`,
/// in row-local columns. A hint at a wrap break belongs to the next row;
/// one at the end of the line to its last row.
#[must_use]
pub fn row_hints<'a>(hints: &'a [crate::inlay::InlayHint], row: &VisualRow) -> Vec<RowHint<'a>> {
    let first = hints.partition_point(|hint| (hint.line, hint.col) < (row.line, row.start));
    hints
        .get(first..)
        .unwrap_or_default()
        .iter()
        .take_while(|hint| hint.line == row.line)
        .filter(|hint| hint.col < row.end || (hint.col == row.end && row.end == row.line_len))
        .map(|hint| RowHint {
            col: hint.col.saturating_sub(row.start),
            cells: hint.label.chars().map(char_display_width).sum(),
            label: &hint.label,
        })
        .collect()
}

/// Display geometry of one row with its inline hints: a hint pushes the
/// char after it, and everything further right, by its width. Tab stops
/// still count text cells only, so a hint never changes a tab's width.
pub struct RowCells<'a> {
    pub text: &'a str,
    pub hints: &'a [RowHint<'a>],
}

impl RowCells<'_> {
    fn shift(&self, col: usize, inclusive: bool) -> usize {
        self.hints
            .iter()
            .filter(|hint| hint.col < col || (inclusive && hint.col == col))
            .map(|hint| hint.cells)
            .sum()
    }

    /// Cell where the char at `col` is painted (after any hint before it).
    #[must_use]
    pub fn char_x(&self, col: usize) -> usize {
        display_x(self.text, col).saturating_add(self.shift(col, true))
    }

    /// Cell of a caret before `col`, which is also where a span ending at
    /// `col` ends: a hint before the char stays right of the caret.
    #[must_use]
    pub fn caret_x(&self, col: usize) -> usize {
        display_x(self.text, col).saturating_add(self.shift(col, false))
    }

    /// Char column at display cell `x`; a click on a hint lands before the
    /// char it annotates.
    #[must_use]
    pub fn col_at(&self, x: usize) -> usize {
        let (mut base, mut shift) = (0usize, 0usize);
        let mut col = 0usize;
        for c in self.text.chars() {
            for hint in self.hints.iter().filter(|hint| hint.col == col) {
                shift = shift.saturating_add(hint.cells);
                if base.saturating_add(shift) > x {
                    return col;
                }
            }
            let next = advance_cell(base, c);
            if next.saturating_add(shift) > x {
                return col;
            }
            base = next;
            col = col.saturating_add(1);
        }
        col
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;

    #[test]
    fn hints_shift_the_chars_after_them_but_not_the_caret_before() {
        let hints = [RowHint {
            col: 1,
            cells: 5,
            label: ": i32",
        }];
        let row = RowCells {
            text: "x = 1",
            hints: &hints,
        };
        assert_eq!(row.caret_x(1), 1);
        assert_eq!(row.char_x(1), 6);
        assert_eq!(row.char_x(0), 0);
        assert_eq!(row.caret_x(2), 7);
        // Clicks on the hint land before the char it annotates.
        assert_eq!(row.col_at(3), 1);
        assert_eq!(row.col_at(6), 1);
        assert_eq!(row.col_at(7), 2);
        assert_eq!(row.col_at(0), 0);
        assert_eq!(row.col_at(99), 5);
        // Tab stops ignore hint cells.
        let tabbed = RowCells {
            text: "a\tb",
            hints: &[RowHint {
                col: 1,
                cells: 2,
                label: "  ",
            }],
        };
        assert_eq!(tabbed.char_x(2), TAB_WIDTH + 2);
    }

    #[test]
    fn tabs_advance_to_the_next_tab_stop() {
        assert_eq!(display_x("\tfoo", 1), TAB_WIDTH);
        assert_eq!(display_x("ab\tc", 3), TAB_WIDTH);
        assert_eq!(display_x("abcd\tx", 5), TAB_WIDTH * 2);
        assert_eq!(col_at_display_x("\tfoo", 2), 0);
        assert_eq!(col_at_display_x("\tfoo", TAB_WIDTH), 1);
        assert_eq!(expand_tabs("a\tb", 0), "a   b");
        assert_eq!(expand_tabs("\tb", 2), "  b");
    }

    #[test]
    fn row_cache_reuses_rows_until_the_buffer_or_width_changes() {
        let mut doc = Doc::new("alpha\nbeta\n");
        let mut cache = RowCache::default();
        cache.ensure(&doc, None, None);
        let first = cache.rows().to_vec();
        assert_eq!(first.len(), doc.line_count());
        assert_eq!(cache.max_cells(), "alpha".chars().count());

        // Same revision and width: the rows are reused, not rebuilt.
        cache.ensure(&doc, None, None);
        assert_eq!(cache.rows(), first.as_slice());

        doc.insert(doc.char_at_line_col(1, 4), "!");
        cache.ensure(&doc, None, None);
        assert_ne!(cache.rows(), first.as_slice());
        assert_eq!(cache.max_cells(), "beta!".chars().count());

        // A narrower wrap width re-lays the rows out.
        cache.ensure(&doc, Some(2), None);
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
