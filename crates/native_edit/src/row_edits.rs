//! One edit re-lays only the lines it touched: typing in a large file must
//! not slice every line again (see [`RowCache`]).
use crate::doc::Buffer;
use crate::rows::{RowCache, VisualRow, layout_line, row_cells};

impl RowCache {
    /// Re-lay only the lines one edit changed, for a layout with no folded
    /// region and no spacer. Returns false when this does not apply (more
    /// than one edit, a forgotten revision, a changed wrap or tab width);
    /// the caller then lays out every line.
    pub(crate) fn relayout_edit<B: Buffer>(
        &mut self,
        doc: &B,
        wrap_columns: Option<usize>,
        folds: Option<&crate::fold::Folds>,
    ) -> bool {
        let tab = doc.indent_style().tab_cells();
        if !(self.ready && self.plain && self.columns == wrap_columns && self.tab == tab) {
            return false;
        }
        let Some(changes) = doc.changes_since(self.revision) else {
            return false;
        };
        let [change] = changes.as_slice() else {
            return false;
        };
        // Lines of the edit in the new text, and the old lines they replace.
        let end_char = change
            .at
            .saturating_add(change.inserted)
            .min(doc.len_chars());
        let first = doc.line_col_at(change.at.min(doc.len_chars())).0;
        let last_new = doc.line_col_at(end_char).0;
        let lines = doc.line_count();
        let Some(last_old) = (last_new.saturating_add(self.lines)).checked_sub(lines) else {
            return false;
        };
        if last_old < first || last_old >= self.lines {
            return false;
        }
        let from = self.rows.partition_point(|row| row.line < first);
        let to = self.rows.partition_point(|row| row.line <= last_old);
        let mut fresh = Vec::new();
        for line in first..=last_new {
            layout_line(
                &mut fresh,
                doc.line_text(line),
                (line, wrap_columns, tab),
                folds,
            );
        }
        let removed_max = self
            .rows
            .get(from..to)
            .unwrap_or_default()
            .iter()
            .map(|row| row_cells(row, tab))
            .max()
            .unwrap_or(0);
        let added_max = fresh
            .iter()
            .map(|row| row_cells(row, tab))
            .max()
            .unwrap_or(0);
        for row in self.rows.get_mut(to..).unwrap_or_default() {
            row.line = (row.line.saturating_add(lines)).saturating_sub(self.lines);
        }
        let _ = self.rows.splice(from..to, fresh);
        self.max_cells = if removed_max < self.max_cells {
            self.max_cells.max(added_max)
        } else {
            self.rows
                .iter()
                .map(|row: &VisualRow| row_cells(row, tab))
                .max()
                .unwrap_or(0)
        };
        self.lines = lines;
        self.revision = doc.revision();
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::doc::{Buffer, Doc};
    use crate::rows::{RowCache, row_cells, visual_rows};

    /// After every edit the cached rows match a full layout.
    #[test]
    fn one_edit_relayout_matches_a_full_layout() {
        let start = "fn main() {\n    let long_line = 1 + 2 + 3 + 4 + 5;\n}\n\ntail line\n";
        let edits: &[&dyn Fn(&mut Doc)] = &[
            &|doc| doc.insert(3, "x"),
            &|doc| doc.insert(12, "\n\n"),
            &|doc| doc.insert(0, "first\nsecond "),
            &|doc| doc.delete(2..20),
            &|doc| {
                let end = doc.len_chars();
                doc.insert(end, "\nnew last line that is rather long indeed");
            },
            &|doc| {
                let end = doc.len_chars();
                doc.delete(end.saturating_sub(30)..end);
            },
            &|doc| doc.delete(0..1),
            &|doc| doc.insert(5, "\t\tindented"),
        ];
        for wrap in [None, Some(10)] {
            let mut doc = Doc::new(start);
            let mut cache = RowCache::default();
            cache.ensure(&doc, wrap, None);
            for (index, edit) in edits.iter().enumerate() {
                edit(&mut doc);
                cache.ensure(&doc, wrap, None);
                let full = visual_rows(&doc, wrap);
                assert_eq!(cache.rows(), full.as_slice(), "edit {index}, wrap {wrap:?}");
                let tab = doc.indent_style().tab_cells();
                let widest = full
                    .iter()
                    .map(|row| row_cells(row, tab))
                    .max()
                    .unwrap_or(0);
                assert_eq!(cache.max_cells(), widest, "edit {index}, wrap {wrap:?}");
            }
        }
    }
}
