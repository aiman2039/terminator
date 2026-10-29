//! diff3 on top of `similar`: line diffs base→ours and base→theirs, merged
//! into stable runs and change chunks. Overlapping or touching changes from
//! both sides form one chunk; it conflicts unless both sides agree.
use similar::{Algorithm, DiffTag, TextDiff};
use std::ops::Range;

/// Wall-clock budget per line diff; `similar` falls back to a coarser
/// (still correct) diff past it.
const DIFF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChunkKind {
    /// Unchanged on both sides.
    Stable,
    /// Only ours changed.
    Ours,
    /// Only theirs changed.
    Theirs,
    /// Both changed the same way.
    Same,
    /// Both changed differently.
    Conflict,
}

impl ChunkKind {
    /// Changes that need no choice between sides.
    pub(crate) fn non_conflicting(self) -> bool {
        matches!(self, Self::Ours | Self::Theirs | Self::Same)
    }
}

/// One aligned region, as line ranges into each side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Region {
    pub kind: ChunkKind,
    pub base: Range<usize>,
    pub ours: Range<usize>,
    pub theirs: Range<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Ours,
    Theirs,
}

struct Hunk {
    side: Side,
    base: Range<usize>,
    other: Range<usize>,
}

fn hunks(base: &[&str], other: &[&str], side: Side) -> Vec<Hunk> {
    TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .timeout(DIFF_TIMEOUT)
        .diff_slices(base, other)
        .ops()
        .iter()
        .filter(|op| op.tag() != DiffTag::Equal)
        .map(|op| Hunk {
            side,
            base: op.old_range(),
            other: op.new_range(),
        })
        .collect()
}

/// Running offset of one side relative to the base.
#[derive(Clone, Copy, Default)]
struct Offset(isize);

impl Offset {
    fn map(self, line: usize) -> usize {
        line.checked_add_signed(self.0).unwrap_or(0)
    }

    fn after(hunk: &Hunk) -> Self {
        let other = isize::try_from(hunk.other.end).unwrap_or(isize::MAX);
        let base = isize::try_from(hunk.base.end).unwrap_or(isize::MAX);
        Self(other.saturating_sub(base))
    }
}

/// The side range a group of hunks covers over base `lo..hi`.
fn side_range(group: &[&Hunk], side: Side, offset: Offset, span: Range<usize>) -> Range<usize> {
    let mut own = group.iter().filter(|hunk| hunk.side == side);
    let Some(first) = own.next() else {
        return offset.map(span.start)..offset.map(span.end);
    };
    let last = own.next_back().unwrap_or(first);
    let start = first
        .other
        .start
        .saturating_sub(first.base.start.saturating_sub(span.start));
    let end = last
        .other
        .end
        .saturating_add(span.end.saturating_sub(last.base.end));
    start..end
}

/// The three line lists being merged.
#[derive(Clone, Copy)]
pub(crate) struct Lines<'a> {
    pub base: &'a [&'a str],
    pub ours: &'a [&'a str],
    pub theirs: &'a [&'a str],
}

/// Walk state: the base line reached and each side's offset there.
#[derive(Default)]
struct Cursor {
    at: usize,
    ours: Offset,
    theirs: Offset,
}

impl Cursor {
    /// Stable run from the cursor up to base line `to`.
    fn stable(&self, to: usize) -> Region {
        Region {
            kind: ChunkKind::Stable,
            base: self.at..to,
            ours: self.ours.map(self.at)..self.ours.map(to),
            theirs: self.theirs.map(self.at)..self.theirs.map(to),
        }
    }

    fn change(&mut self, lines: Lines<'_>, group: &[&Hunk], span: Range<usize>) -> Region {
        let ours = side_range(group, Side::Ours, self.ours, span.clone());
        let theirs = side_range(group, Side::Theirs, self.theirs, span.clone());
        let changed = |side: Side| group.iter().any(|hunk| hunk.side == side);
        let kind = match (changed(Side::Ours), changed(Side::Theirs)) {
            (true, false) => ChunkKind::Ours,
            (false, _) => ChunkKind::Theirs,
            (true, true) if lines.ours.get(ours.clone()) == lines.theirs.get(theirs.clone()) => {
                ChunkKind::Same
            }
            (true, true) => ChunkKind::Conflict,
        };
        for hunk in group {
            match hunk.side {
                Side::Ours => self.ours = Offset::after(hunk),
                Side::Theirs => self.theirs = Offset::after(hunk),
            }
        }
        self.at = span.end;
        Region {
            kind,
            base: span,
            ours,
            theirs,
        }
    }
}

/// Aligned regions covering all three line lists, in order.
pub(crate) fn regions(lines: Lines<'_>) -> Vec<Region> {
    let mut all = hunks(lines.base, lines.ours, Side::Ours);
    all.extend(hunks(lines.base, lines.theirs, Side::Theirs));
    all.sort_by_key(|hunk| (hunk.base.start, hunk.base.end));
    let mut out = Vec::new();
    let mut cursor = Cursor::default();
    let mut index = 0usize;
    while let Some(first) = all.get(index) {
        let (lo, mut hi) = (first.base.start, first.base.end);
        let mut group = vec![first];
        index = index.saturating_add(1);
        // Touching changes group too, like Git's own merge.
        while let Some(next) = all.get(index).filter(|next| next.base.start <= hi) {
            hi = hi.max(next.base.end);
            group.push(next);
            index = index.saturating_add(1);
        }
        if lo > cursor.at {
            out.push(cursor.stable(lo));
        }
        out.push(cursor.change(lines, &group, lo..hi));
    }
    if cursor.at < lines.base.len() || out.is_empty() {
        out.push(cursor.stable(lines.base.len()));
    }
    out
}
