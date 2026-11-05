//! The Find in Files list model: one header row per file, then its hits.
//! The keyboard cursor walks the hits in list order (headers are skipped)
//! and wraps at both ends; results that stream in later keep it in place.
use super::*;

/// One list row: a file header, or the `index`-th hit in list order (in
/// `file`).
pub(crate) enum Row<'a> {
    File(&'a FileHits),
    Hit {
        file: &'a FileHits,
        hit: &'a LineHit,
        index: usize,
    },
}

impl FindInFiles {
    /// Move the cursor one hit down (or up), wrapping at the ends.
    pub fn step(&mut self, down: bool) {
        let Some(last) = self.total.checked_sub(1) else {
            return;
        };
        let at = self.selected.min(last);
        self.selected = match (down, at) {
            (true, at) if at >= last => 0,
            (true, at) => at.saturating_add(1),
            (false, 0) => last,
            (false, at) => at.saturating_sub(1),
        };
        self.reveal = true;
    }

    /// Put the cursor on the `index`-th hit.
    pub fn select(&mut self, index: usize) {
        self.selected = index.min(self.total.saturating_sub(1));
    }

    /// File and hit position of the `index`-th hit.
    pub(super) fn locate(&self, index: usize) -> Option<(usize, usize)> {
        let mut rest = index;
        for (at, file) in self.files.iter().enumerate() {
            if rest < file.hits.len() {
                return Some((at, rest));
            }
            rest = rest.saturating_sub(file.hits.len());
        }
        None
    }

    /// The cursor's file and hit.
    pub fn selected_hit(&self) -> Option<(&FileHits, &LineHit)> {
        let (at, hit) = self.locate(self.selected)?;
        let file = self.files.get(at)?;
        Some((file, file.hits.get(hit)?))
    }

    /// Where Enter opens the cursor's hit: its file at the first match.
    pub fn selected_location(&self) -> Option<NavLocation> {
        let (file, hit) = self.selected_hit()?;
        Some(NavLocation {
            path: file.path.clone(),
            line: hit.line,
            character: hit.character,
        })
    }

    /// List row of the `index`-th hit: each file adds its header row.
    pub fn row_of(&self, index: usize) -> Option<usize> {
        let (file, _) = self.locate(index)?;
        Some(index.saturating_add(file).saturating_add(1))
    }

    pub fn row_count(&self) -> usize {
        self.files.len().saturating_add(self.total)
    }

    /// The list rows in `range`, in order.
    pub fn rows(&self, range: std::ops::Range<usize>) -> Vec<Row<'_>> {
        let mut rows = Vec::new();
        let (mut row, mut index) = (0usize, 0usize);
        for file in &self.files {
            if row >= range.end {
                break;
            }
            let next = row.saturating_add(1).saturating_add(file.hits.len());
            if next <= range.start {
                // The whole file is above the visible rows.
                (row, index) = (next, index.saturating_add(file.hits.len()));
                continue;
            }
            if range.contains(&row) {
                rows.push(Row::File(file));
            }
            for hit in &file.hits {
                row = row.saturating_add(1);
                if range.contains(&row) {
                    rows.push(Row::Hit { file, hit, index });
                }
                index = index.saturating_add(1);
            }
            row = next;
        }
        rows
    }
}

/// Scroll offset that shows a row at `top` of `height` in a `viewport`
/// now scrolled to `offset`; unchanged when the row already shows.
pub(crate) fn keep_visible(top: f32, height: f32, viewport: f32, offset: f32) -> f32 {
    if top < offset {
        top
    } else if top + height > offset + viewport {
        (top + height - viewport).max(0.0)
    } else {
        offset
    }
}
