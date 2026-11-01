//! Staging one editor gutter block: the working-tree hunks it overlaps,
//! written to the index as one patch.
use super::*;
use std::ops::Range;

/// One working-tree hunk to stage, or only some of its lines.
pub(crate) type BlockPick = (usize, Option<LineSelection>);

/// The working-tree hunks a gutter block over buffer lines `block`
/// covers. A deleted block (`start == end`) takes the removal at its
/// line. A hunk that overlaps the block only in part keeps just the
/// overlapping added lines and the removed lines at the same offsets.
pub(crate) fn block_picks(hunks: &[Hunk], block: &Range<usize>) -> Vec<BlockPick> {
    hunks
        .iter()
        .enumerate()
        .filter_map(|(index, hunk)| pick(index, hunk, block))
        .collect()
}

fn pick(index: usize, hunk: &Hunk, block: &Range<usize>) -> Option<BlockPick> {
    if hunk.new.is_empty() {
        // A pure removal sits before line `at`.
        let at = hunk.new.start;
        let inside = if block.is_empty() {
            at == block.start
        } else {
            block.contains(&at)
        };
        return inside.then_some((index, None));
    }
    if block.is_empty() {
        return None;
    }
    let start = block.start.max(hunk.new.start);
    let end = block.end.min(hunk.new.end);
    if start >= end {
        return None;
    }
    if start == hunk.new.start && end == hunk.new.end {
        return Some((index, None));
    }
    let first = start.saturating_sub(hunk.new.start);
    let last = if end == hunk.new.end {
        hunk.old.len()
    } else {
        end.saturating_sub(hunk.new.start)
    };
    let old = (first..last.min(hunk.old.len()))
        .map(|offset| hunk.old.start.saturating_add(offset))
        .collect();
    let lines = LineSelection {
        old,
        new: (start..end).collect(),
    };
    Some((index, Some(lines)))
}

/// One index change for all picks. Picks apply last to first, so the
/// left-side lines of earlier hunks keep their positions.
fn combined(source: &Source, doc: &DiffDocument, picks: &[BlockPick]) -> Option<Undo> {
    let mut work = doc.clone();
    let mut change: Option<Undo> = None;
    for (index, lines) in picks.iter().rev() {
        let hunk = doc.hunks.get(*index)?;
        let step = edit(EditInput {
            source,
            doc: &work,
            hunk,
            kind: Kind::Stage,
            lines: lines.as_ref(),
        });
        work.left_text.clone_from(&step.after.text);
        change = Some(match change {
            None => step,
            Some(first) => Undo {
                after: step.after,
                ..first
            },
        });
    }
    change
}

pub(crate) struct StageBlockInput<'a> {
    pub(crate) service: &'a Services,
    pub(crate) cwd: PathBuf,
    pub(crate) path: PathBuf,
    /// Buffer lines of the gutter block; empty for a deleted block.
    pub(crate) lines: Range<usize>,
    /// The saved buffer text the block was computed from.
    pub(crate) text: String,
    pub(crate) cancel: &'a CancellationToken,
}

/// Stage the working-tree changes under one gutter block.
pub(crate) async fn stage_block(input: StageBlockInput<'_>) -> Result<String> {
    let StageBlockInput {
        service,
        cwd,
        path,
        lines,
        text,
        cancel,
    } = input;
    let _guard = execute::mutations().lock().await;
    let doc = crate::diff::document_async(crate::diff::DocumentAsyncInput {
        service,
        cwd,
        path,
        staged: false,
        cancel,
    })
    .await?;
    ensure!(!doc.read_only, "This file cannot be staged from the editor");
    let source = doc
        .source
        .as_ref()
        .context("Staging needs a Git repository")?;
    ensure!(
        doc.right_text.replace("\r\n", "\n") == text.replace("\r\n", "\n"),
        "The file changed on disk; reload it before staging"
    );
    let picks = block_picks(&doc.hunks, &lines);
    ensure!(!picks.is_empty(), "These changes are already staged");
    let change = combined(source, &doc, &picks).context("Nothing to stage")?;
    execute::apply(service, &change, cancel).await?;
    Ok(format!(
        "Staged {} hunk{}",
        picks.len(),
        if picks.len() == 1 { "" } else { "s" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(old: Range<usize>, new: Range<usize>) -> Hunk {
        Hunk {
            old,
            new,
            unified_row: 0,
            split_row: 0,
        }
    }

    #[test]
    fn gutter_block_maps_to_overlapping_diff_hunk() {
        let hunks = [hunk(1..2, 1..2), hunk(5..5, 5..7), hunk(9..10, 11..12)];
        assert_eq!(block_picks(&hunks, &(4..8)), [(1, None)]);
        assert_eq!(block_picks(&hunks, &(0..20)).len(), 3);
        assert!(block_picks(&hunks, &(2..5)).is_empty());
    }

    #[test]
    fn deleted_gutter_block_maps_to_hunk_at_its_line() {
        let hunks = [hunk(1..2, 1..2), hunk(3..5, 3..3), hunk(6..7, 4..5)];
        assert_eq!(block_picks(&hunks, &(3..3)), [(1, None)]);
        assert!(block_picks(&hunks, &(4..4)).is_empty());
    }

    #[test]
    fn partial_overlap_stages_only_overlapping_lines() {
        let hunks = [hunk(2..5, 2..5)];
        let picks = block_picks(&hunks, &(3..4));
        let expected = LineSelection {
            old: [3].into(),
            new: [3].into(),
        };
        assert_eq!(picks, [(0, Some(expected))]);
        // Reaching the hunk's end takes the removed lines past it too.
        let wide = [hunk(2..6, 2..4)];
        let picks = block_picks(&wide, &(3..9));
        let expected = LineSelection {
            old: [3, 4, 5].into(),
            new: [3].into(),
        };
        assert_eq!(picks, [(0, Some(expected))]);
    }

    #[test]
    fn combined_picks_stage_every_block_hunk_in_one_change() {
        let mut doc = DiffDocument {
            left_text: "a\nb\nc\nd\n".into(),
            right_text: "a\nB\nc\nD\n".into(),
            ..DiffDocument::default()
        };
        doc.hunks = vec![hunk(1..2, 1..2), hunk(3..4, 3..4)];
        let source = Source {
            root: PathBuf::new(),
            path: "f".into(),
            index: b"100644 x 0\tf".to_vec(),
            head: Vec::new(),
            exists: true,
            mode: 0o100644,
        };
        let picks = block_picks(&doc.hunks, &(0..4));
        let change = combined(&source, &doc, &picks).unwrap();
        assert_eq!(change.before.text, doc.left_text);
        assert_eq!(change.after.text, doc.right_text);
    }
}
