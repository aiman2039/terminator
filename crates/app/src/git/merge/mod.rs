//! Three-way merge tool (`IntelliJ` Merge Revisions): Ours | Result |
//! Theirs for one conflicted file. Sides come from the index stages
//! (`:1:` base, `:2:` ours, `:3:` theirs); the result starts as the base
//! and each change chunk is accepted from a side or edited by hand. Saving
//! writes the file; staging it ("Mark resolved") is a separate explicit
//! click. Git reads and file writes run on the file pool.
use std::{ops::Range, path::PathBuf};

mod io;
#[cfg(test)]
mod tests;
mod three_way;

pub(crate) use io::{MergeTab, MergeTabs};
pub(crate) use three_way::ChunkKind;

/// One version of the file, split into lines (endings kept).
#[derive(Clone, Debug, Default)]
pub(crate) struct Side {
    pub text: String,
    pub lines: Vec<Range<usize>>,
    /// Syntax spans per line; empty when plain.
    pub spans: Vec<Vec<crate::diff::DiffSpan>>,
    /// False when this side does not have the file (add/add, modify/delete).
    pub present: bool,
}

impl Side {
    pub(crate) fn new(text: String, present: bool) -> Self {
        let mut lines = Vec::new();
        let mut start = 0usize;
        for line in text.split_inclusive('\n') {
            let end = start.saturating_add(line.len());
            lines.push(start..end);
            start = end;
        }
        Self {
            text,
            lines,
            spans: Vec::new(),
            present,
        }
    }

    /// Text of a line range, endings included.
    pub(crate) fn slice(&self, lines: &Range<usize>) -> &str {
        let start = self
            .lines
            .get(lines.start)
            .map_or(self.text.len(), |r| r.start);
        let end = lines
            .end
            .checked_sub(1)
            .and_then(|last| self.lines.get(last))
            .map_or(start, |r| r.end);
        self.text.get(start..end.max(start)).unwrap_or_default()
    }

    fn line_list(&self) -> Vec<&str> {
        self.lines
            .iter()
            .map(|range| self.text.get(range.clone()).unwrap_or_default())
            .collect()
    }
}

/// One aligned region and its current result text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chunk {
    pub kind: ChunkKind,
    pub base: Range<usize>,
    pub ours: Range<usize>,
    pub theirs: Range<usize>,
    pub result: String,
    /// The user accepted a side or edited this chunk.
    pub resolved: bool,
}

/// What a chunk action puts in the result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    Ours,
    Theirs,
    /// Ours, then theirs.
    Both,
    /// Keep the base text (ignore both changes).
    Base,
}

/// The merge being resolved.
#[derive(Clone, Debug, Default)]
pub(crate) struct MergeDoc {
    pub base: Side,
    pub ours: Side,
    pub theirs: Side,
    pub chunks: Vec<Chunk>,
}

impl MergeDoc {
    pub(crate) fn new(base: Side, ours: Side, theirs: Side) -> Self {
        let regions = {
            let (b, o, t) = (base.line_list(), ours.line_list(), theirs.line_list());
            three_way::regions(three_way::Lines {
                base: &b,
                ours: &o,
                theirs: &t,
            })
        };
        let chunks = regions
            .into_iter()
            .flat_map(split_stable)
            .map(|region| Chunk {
                kind: region.kind,
                result: base.slice(&region.base).to_owned(),
                resolved: region.kind == ChunkKind::Stable,
                base: region.base,
                ours: region.ours,
                theirs: region.theirs,
            })
            .collect();
        Self {
            base,
            ours,
            theirs,
            chunks,
        }
    }

    /// Put `pick` in chunk `index` and mark it resolved.
    pub(crate) fn accept(&mut self, index: usize, pick: Pick) {
        let Some(chunk) = self.chunks.get(index) else {
            return;
        };
        let (ours, theirs) = (
            self.ours.slice(&chunk.ours),
            self.theirs.slice(&chunk.theirs),
        );
        let text = match pick {
            Pick::Ours => ours.to_owned(),
            Pick::Theirs => theirs.to_owned(),
            Pick::Both => joined(ours, theirs),
            Pick::Base => self.base.slice(&chunk.base).to_owned(),
        };
        if let Some(chunk) = self.chunks.get_mut(index) {
            chunk.result = text;
            chunk.resolved = true;
        }
    }

    /// Typing in a change chunk resolves it by hand.
    pub(crate) fn edited(&mut self, index: usize) {
        if let Some(chunk) = self.chunks.get_mut(index) {
            chunk.resolved = true;
        }
    }

    /// Accept every unresolved change that needs no choice. Returns how
    /// many chunks changed.
    pub(crate) fn apply_non_conflicting(&mut self) -> usize {
        let picks: Vec<(usize, Pick)> = self
            .chunks
            .iter()
            .enumerate()
            .filter(|(_, chunk)| !chunk.resolved && chunk.kind.non_conflicting())
            .map(|(index, chunk)| {
                let pick = if chunk.kind == ChunkKind::Theirs {
                    Pick::Theirs
                } else {
                    Pick::Ours
                };
                (index, pick)
            })
            .collect();
        for (index, pick) in &picks {
            self.accept(*index, *pick);
        }
        picks.len()
    }

    pub(crate) fn conflicts_left(&self) -> usize {
        self.unresolved(|kind| kind == ChunkKind::Conflict)
    }

    pub(crate) fn changes_left(&self) -> usize {
        self.unresolved(ChunkKind::non_conflicting)
    }

    fn unresolved(&self, keep: impl Fn(ChunkKind) -> bool) -> usize {
        self.chunks
            .iter()
            .filter(|chunk| !chunk.resolved && keep(chunk.kind))
            .count()
    }

    /// File text to save. Unresolved conflicts keep diff3 markers so no
    /// side is lost; unresolved one-sided changes keep the base text.
    pub(crate) fn result_text(&self) -> String {
        let mut out = String::with_capacity(self.base.text.len());
        for chunk in &self.chunks {
            if chunk.resolved || chunk.kind != ChunkKind::Conflict {
                out.push_str(&chunk.result);
                continue;
            }
            for (marker, text) in [
                ("<<<<<<< ours", self.ours.slice(&chunk.ours)),
                ("||||||| base", self.base.slice(&chunk.base)),
                ("=======", self.theirs.slice(&chunk.theirs)),
            ] {
                ensure_line_end(&mut out);
                out.push_str(marker);
                out.push('\n');
                out.push_str(text);
            }
            ensure_line_end(&mut out);
            out.push_str(">>>>>>> theirs\n");
        }
        out
    }

    /// The next (or previous) unresolved change chunk from `from`.
    pub(crate) fn neighbor(&self, from: Option<usize>, forward: bool) -> Option<usize> {
        let open = |index: &usize| {
            self.chunks
                .get(*index)
                .is_some_and(|chunk| !chunk.resolved && chunk.kind != ChunkKind::Stable)
        };
        if forward {
            let start = from.map_or(0, |at| at.saturating_add(1));
            (start..self.chunks.len()).find(open)
        } else {
            let end = from.unwrap_or(self.chunks.len());
            (0..end).rev().find(open)
        }
    }
}

/// Long stable runs become blocks of at most [`STABLE_BLOCK`] lines, so
/// the view lays out and copies only the blocks it shows.
fn split_stable(region: three_way::Region) -> Vec<three_way::Region> {
    if region.kind != ChunkKind::Stable || region.base.len() <= STABLE_BLOCK {
        return vec![region];
    }
    let shift = |range: &Range<usize>, offset: usize, len: usize| {
        let start = range.start.saturating_add(offset);
        start..start.saturating_add(len)
    };
    (0..region.base.len())
        .step_by(STABLE_BLOCK)
        .map(|offset| {
            let len = STABLE_BLOCK.min(region.base.len().saturating_sub(offset));
            three_way::Region {
                kind: ChunkKind::Stable,
                base: shift(&region.base, offset, len),
                ours: shift(&region.ours, offset, len),
                theirs: shift(&region.theirs, offset, len),
            }
        })
        .collect()
}

/// Lines per stable display block.
const STABLE_BLOCK: usize = 64;

fn ensure_line_end(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
}

/// `first` then `second`, with a line break between when `first` lacks one.
fn joined(first: &str, second: &str) -> String {
    let mut out = first.to_owned();
    if !second.is_empty() {
        ensure_line_end(&mut out);
    }
    out.push_str(second);
    out
}

/// Open-tab identity for a conflicted file: the repository root and path.
pub(crate) fn tab(root: PathBuf, path: PathBuf) -> crate::Tab {
    crate::Tab::Merge { cwd: root, path }
}
