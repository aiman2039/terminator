//! Per-buffer derivations and requests that outlive one frame: cached
//! diagnostic underlines, the `gf` lookup, autosave timing, the
//! buffer-versus-disk comparison, and the large-file override.
use super::file_io::{LineEnding, TextEncoding};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use terminator_native_edit::doc::{Buffer, Doc};
use terminator_native_edit::lsp as native_lsp;
use terminator_native_edit::view::{Squiggle, squiggle_tails, squiggles};
use terminator_native_edit::vim::SplitPlace;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::error::TryRecvError;

/// A `gf` target: file, zero-based line and column.
/// Path, zero-based line and column, and whether `:e` makes a new file.
pub(super) type GotoTarget = Option<(PathBuf, u32, u32, bool)>;
pub(super) type GotoReceiver = UnboundedReceiver<anyhow::Result<GotoTarget>>;

#[derive(Default)]
pub(crate) struct DocExtras {
    /// The user enabled highlighting and folds for this large file.
    pub(super) large_override: bool,
    squiggles: Option<SquiggleCache>,
    /// `gf` resolving on the file pool: the name and the answer.
    pub(super) goto: Option<(String, Option<SplitPlace>, GotoReceiver)>,
    /// Revision and time of the last edit, for delayed autosave.
    pub(super) last_edit: Option<(u64, Instant)>,
    /// The view that had keyboard focus last frame (focus-loss autosave).
    pub(super) focus: Option<String>,
    /// The buffer-versus-disk diff under the conflict bar.
    pub(super) compare: Option<CompareView>,
    /// Status-bar cells and the format they describe.
    pub(super) status: Option<(StatusKey, Arc<Vec<String>>)>,
    /// The Quick Documentation pointer timer and popup of this file's
    /// views.
    pub(super) quick_doc: super::quick_doc::QuickDoc,
}

/// What the status-bar cells show: line breaks, encoding, indentation.
pub(super) type StatusKey = (LineEnding, TextEncoding, native_lsp::FormatOptions);

/// Underlines for one buffer revision and diagnostics generation.
struct SquiggleCache {
    revision: u64,
    generation: u64,
    heads: Arc<Vec<Squiggle>>,
    tails: Arc<Vec<Squiggle>>,
}

/// Underlines: first lines (navigation targets) and continuation lines.
pub(super) type SquiggleSets = (Arc<Vec<Squiggle>>, Arc<Vec<Squiggle>>);

impl DocExtras {
    /// Underlines for `problems` (at diagnostics `generation`) on `doc`,
    /// rebuilt only when the buffer revision or the diagnostics change
    /// (not on every frame).
    pub(super) fn squiggles(
        &mut self,
        doc: &Doc,
        (problems, generation): (&[native_lsp::Diagnostic], u64),
    ) -> SquiggleSets {
        let current = self
            .squiggles
            .as_ref()
            .filter(|cache| cache.revision == doc.revision() && cache.generation == generation);
        if let Some(cache) = current {
            return (Arc::clone(&cache.heads), Arc::clone(&cache.tails));
        }
        let cache = SquiggleCache {
            revision: doc.revision(),
            generation,
            heads: Arc::new(squiggles(doc, problems)),
            tails: Arc::new(squiggle_tails(doc, problems)),
        };
        let sets = (Arc::clone(&cache.heads), Arc::clone(&cache.tails));
        self.squiggles = Some(cache);
        sets
    }

    /// The `gf` answer, once it landed.
    pub(super) fn take_goto(&mut self) -> Option<(String, Option<SplitPlace>, GotoTarget)> {
        let (_, _, rx) = self.goto.as_mut()?;
        let answer = match rx.try_recv() {
            Ok(answer) => answer.ok().flatten(),
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => None,
        };
        let (name, place, _) = self.goto.take()?;
        Some((name, place, answer))
    }
}

/// One line of the buffer-versus-disk comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CompareLine {
    Same(String),
    /// Only in the buffer (would be written by a save).
    Mine(String),
    /// Only on disk (would come back with a reload).
    Disk(String),
    /// Unchanged lines left out between two changes.
    Skipped,
}

/// The comparison under the conflict bar: reading the disk text, then
/// the diff of the buffer against it.
pub(super) struct CompareView {
    pub(super) rx: Option<UnboundedReceiver<anyhow::Result<String>>>,
    pub(super) lines: Vec<CompareLine>,
    pub(super) error: Option<String>,
}

/// Changed lines of `disk` → `mine` with three lines of context.
pub(super) fn compare_lines(disk: &str, mine: &str) -> Vec<CompareLine> {
    let diff = similar::TextDiff::configure()
        .timeout(crate::diff::DIFF_TIMEOUT)
        .diff_lines(disk, mine);
    let mut out = Vec::new();
    for (index, group) in diff.grouped_ops(3).iter().enumerate() {
        if index > 0 {
            out.push(CompareLine::Skipped);
        }
        for op in group {
            for change in diff.iter_changes(op) {
                let text = change.value().trim_end_matches(['\n', '\r']).to_owned();
                out.push(match change.tag() {
                    similar::ChangeTag::Equal => CompareLine::Same(text),
                    similar::ChangeTag::Insert => CompareLine::Mine(text),
                    similar::ChangeTag::Delete => CompareLine::Disk(text),
                });
            }
        }
    }
    out
}
