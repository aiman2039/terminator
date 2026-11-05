//! The last edits of a buffer, so positions taken at an older revision
//! can follow them ([`crate::doc::Buffer::changes_since`]).
use std::collections::VecDeque;

/// Edits a [`Journal`] keeps.
const JOURNAL_LEN: usize = 1024;

/// One edit: `removed` chars at char `at` were replaced by `inserted`
/// chars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub at: usize,
    pub removed: usize,
    pub inserted: usize,
}

impl Change {
    /// `len` chars inserted at `at`.
    #[must_use]
    pub fn insert(at: usize, len: usize) -> Self {
        Self {
            at,
            removed: 0,
            inserted: len,
        }
    }

    /// `len` chars deleted at `at`.
    #[must_use]
    pub fn delete(at: usize, len: usize) -> Self {
        Self {
            at,
            removed: len,
            inserted: 0,
        }
    }
}

/// The last [`JOURNAL_LEN`] edits, each with the revision before it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Journal(VecDeque<(u64, Change)>);

impl Journal {
    /// Remember `change`, made at revision `before`.
    pub(crate) fn record(&mut self, before: u64, change: Change) {
        if self.0.len() >= JOURNAL_LEN {
            self.0.pop_front();
        }
        self.0.push_back((before, change));
    }

    /// The edits from `revision` to `current`, oldest first; `None` when
    /// `revision` is older than the journal.
    pub(crate) fn since(&self, revision: u64, current: u64) -> Option<Vec<Change>> {
        if revision == current {
            return Some(Vec::new());
        }
        let from = self.0.iter().position(|(before, _)| *before == revision)?;
        Some(
            self.0
                .iter()
                .skip(from)
                .map(|(_, change)| *change)
                .collect(),
        )
    }
}
