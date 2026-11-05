//! Commit graph lanes, computed once per log load from parent hashes.
//!
//! Commits arrive children first (`git log --date-order`). Each lane runs
//! down to the next commit it expects. A commit sits on the leftmost lane
//! that expects it, and other lanes that expect it end at its dot. Its
//! first parent continues its lane. Each other parent joins the lane that
//! already expects it, or opens a new lane. A branch tip opens a lane and a
//! root commit ends its lane. Lanes keep their column, so lines stay
//! straight, and a freed column is reused by the next new lane.

/// One lane line in a commit row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LaneLine {
    /// The lane's column: at the row top for `up` and `through`, at the
    /// row bottom for `down`.
    pub col: u16,
    /// Index into the lane palette; it wraps.
    pub color: u8,
}

/// The lane lines one commit row paints.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphRow {
    /// The commit dot's column.
    pub col: u16,
    /// The commit dot's palette index.
    pub color: u8,
    /// Lines from the row top into the dot: lanes of the commit's children.
    pub up: Vec<LaneLine>,
    /// Lines from the dot to the row bottom: lanes of the commit's parents.
    pub down: Vec<LaneLine>,
    /// Lanes that pass the row without touching the dot.
    pub through: Vec<LaneLine>,
}

impl GraphRow {
    /// Columns this row spans.
    #[must_use]
    pub fn width(&self) -> usize {
        let widest = self
            .up
            .iter()
            .chain(&self.down)
            .chain(&self.through)
            .map(|line| line.col)
            .fold(self.col, u16::max);
        usize::from(widest).saturating_add(1)
    }
}

/// Graph rows for `(hash, parents)` pairs in topological order, children
/// first. A parent that is not in the list keeps its lane open to the
/// bottom, because history continues below the list.
pub fn graph_rows<'a, S>(commits: impl IntoIterator<Item = (&'a str, &'a [S])>) -> Vec<GraphRow>
where
    S: AsRef<str> + 'a,
{
    let mut lanes = Lanes::default();
    let mut parents: Vec<&'a str> = Vec::new();
    commits
        .into_iter()
        .map(|(hash, list)| {
            parents.clear();
            for parent in list.iter().map(AsRef::as_ref) {
                if !parents.contains(&parent) {
                    parents.push(parent);
                }
            }
            lanes.row(hash, &parents)
        })
        .collect()
}

/// A lane in use: it runs down to `next`.
#[derive(Clone, Copy)]
struct Lane<'a> {
    next: &'a str,
    color: u8,
}

/// Open lanes by column; `None` is a free column.
#[derive(Default)]
struct Lanes<'a> {
    slots: Vec<Option<Lane<'a>>>,
    next_color: u8,
}

fn column(index: usize) -> u16 {
    u16::try_from(index).unwrap_or(u16::MAX)
}

impl<'a> Lanes<'a> {
    /// Places one commit and moves its lanes on to its parents.
    fn row(&mut self, hash: &'a str, parents: &[&'a str]) -> GraphRow {
        let mut row = GraphRow::default();
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(lane) = slot else {
                continue;
            };
            let line = LaneLine {
                col: column(index),
                color: lane.color,
            };
            if lane.next == hash {
                row.up.push(line);
            } else {
                row.through.push(line);
            }
        }
        // The leftmost lane that expects the commit holds its dot; the
        // other ones end there. A branch tip opens a lane.
        let dot = match row.up.first() {
            Some(line) => *line,
            None => self.open(hash),
        };
        for line in row.up.iter().skip(1) {
            self.set(line.col, None);
        }
        row.col = dot.col;
        row.color = dot.color;
        if let Some((&first, rest)) = parents.split_first() {
            self.set(
                dot.col,
                Some(Lane {
                    next: first,
                    color: dot.color,
                }),
            );
            row.down.push(dot);
            for &parent in rest {
                let line = self.find(parent).unwrap_or_else(|| self.open(parent));
                row.down.push(line);
            }
        } else {
            self.set(dot.col, None);
        }
        while matches!(self.slots.last(), Some(None)) {
            self.slots.pop();
        }
        row
    }

    /// Opens a lane to `next` in the leftmost free column, in a new color.
    fn open(&mut self, next: &'a str) -> LaneLine {
        let color = self.next_color;
        self.next_color = self.next_color.wrapping_add(1);
        let lane = Some(Lane { next, color });
        let free = self
            .slots
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| slot.is_none());
        let index = if let Some((index, slot)) = free {
            *slot = lane;
            index
        } else {
            self.slots.push(lane);
            self.slots.len().saturating_sub(1)
        };
        LaneLine {
            col: column(index),
            color,
        }
    }

    /// The lane that already runs down to `next`.
    fn find(&self, next: &str) -> Option<LaneLine> {
        self.slots.iter().enumerate().find_map(|(index, slot)| {
            slot.filter(|lane| lane.next == next).map(|lane| LaneLine {
                col: column(index),
                color: lane.color,
            })
        })
    }

    fn set(&mut self, col: u16, lane: Option<Lane<'a>>) {
        if let Some(slot) = self.slots.get_mut(usize::from(col)) {
            *slot = lane;
        }
    }
}
