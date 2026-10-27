//! VCS change markers in the editor gutter (`IntelliJ`): the buffer is
//! diffed against the file's `HEAD` version on each edit; added, changed,
//! and deleted blocks get a gutter mark, and clicking one shows the
//! original lines with a Rollback action.

use super::*;
use std::sync::Arc;
use terminator_native_edit::view::{ChangeKind, ChangeMark};

/// One changed block and the base lines it replaced (with line breaks).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GutterHunk {
    pub mark: ChangeMark,
    pub old: Vec<String>,
}

/// Changed blocks of `text` relative to `base`, in buffer order.
pub(crate) fn hunks(base: &str, text: &str) -> Vec<GutterHunk> {
    let diff = similar::TextDiff::configure()
        .timeout(std::time::Duration::from_millis(500))
        .diff_lines(base, text);
    let old_lines: Vec<&str> = base.split_inclusive('\n').collect();
    let old = |range: std::ops::Range<usize>| -> Vec<String> {
        range
            .filter_map(|index| old_lines.get(index))
            .map(|line| (*line).to_owned())
            .collect()
    };
    diff.ops()
        .iter()
        .filter_map(|op| {
            let (old_range, new) = (op.old_range(), op.new_range());
            let kind = match op.tag() {
                similar::DiffTag::Equal => return None,
                similar::DiffTag::Insert => ChangeKind::Added,
                similar::DiffTag::Delete => ChangeKind::Deleted,
                similar::DiffTag::Replace => ChangeKind::Modified,
            };
            Some(GutterHunk {
                mark: ChangeMark {
                    start: new.start,
                    end: if kind == ChangeKind::Deleted {
                        new.start
                    } else {
                        new.end
                    },
                    kind,
                },
                old: old(old_range),
            })
        })
        .collect()
}

/// Put the base lines back in place of a changed block (one undo step).
pub(crate) fn rollback(buffer: &mut Doc, engine: &mut VimEngine, hunk: &GutterHunk) {
    let lines = buffer.line_count();
    let at = |line: usize| {
        if line < lines {
            buffer.char_at_line_col(line, 0)
        } else {
            buffer.len_chars()
        }
    };
    let (from, to) = (at(hunk.mark.start), at(hunk.mark.end));
    let mut old = hunk.old.concat();
    // A block at the very end of a file without a final newline must not
    // gain one.
    if to == buffer.len_chars()
        && !buffer.text().ends_with('\n')
        && old.ends_with('\n')
        && from != to
    {
        old.pop();
    }
    buffer.begin_undo_group();
    buffer.delete(from..to);
    buffer.insert(from, &old);
    buffer.end_undo_group();
    let (line, col) = buffer.line_col_at(from);
    let _ = engine.place_cursor(buffer, line, col);
}

/// The clicked change's popup: what the lines were, and Rollback.
pub(crate) struct GutterPopup {
    pub path: PathBuf,
    pub index: usize,
    pub at: egui::Pos2,
}

/// Paint the popup; returns true when Rollback was pressed, and clears
/// `popup` when the user closes it.
pub(crate) fn popup_ui(
    ui: &egui::Ui,
    popup: &mut Option<GutterPopup>,
    hunks: &[GutterHunk],
    deleted: egui::Color32,
) -> bool {
    let Some(open) = popup.as_ref() else {
        return false;
    };
    let Some(hunk) = hunks.get(open.index) else {
        *popup = None;
        return false;
    };
    let mut rollback = false;
    let mut close = false;
    egui::Area::new(egui::Id::unique(("native-gutter-change", &open.path)))
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(open.at.x + 8.0, open.at.y))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(560.0);
                ui.horizontal(|ui| {
                    ui.strong(match hunk.mark.kind {
                        ChangeKind::Added => "Added lines",
                        ChangeKind::Modified => "Changed lines",
                        ChangeKind::Deleted => "Deleted lines",
                    });
                    if ui.button("Rollback").clicked() {
                        rollback = true;
                    }
                    if appearance::sidebar_action(ui, "X", "Close").clicked() {
                        close = true;
                    }
                });
                if !hunk.old.is_empty() {
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            for line in hunk.old.iter().take(200) {
                                ui.label(
                                    egui::RichText::new(line.trim_end_matches(['\n', '\r']))
                                        .monospace()
                                        .color(deleted),
                                );
                            }
                        });
                }
            });
        });
    if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        close = true;
    }
    if rollback || close {
        *popup = None;
    }
    rollback
}

impl App {
    /// The `HEAD` text for `path`, requesting it in the background when it
    /// is missing or predates the last Git refresh. A stale base keeps
    /// showing until the fresh one lands, so markers never flicker.
    pub(crate) fn git_base(&mut self, path: &Path) -> Option<Arc<str>> {
        let entry = self.git_bases.get(path);
        let fresh = entry.is_some_and(|(generation, _)| *generation == self.git_base_generation);
        if !fresh && self.git_base_pending.insert(path.to_owned()) {
            let _ = self
                .jobs
                .send(Job::GitBase(path.to_owned(), self.git_base_generation));
        }
        entry.and_then(|(_, base)| base.clone())
    }

    pub(crate) fn apply_git_base(&mut self, path: PathBuf, generation: u64, base: Option<String>) {
        self.git_base_pending.remove(&path);
        self.git_bases
            .insert(path, (generation, base.map(Arc::from)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunks_mark_added_changed_and_deleted_blocks() {
        let base = "a\nb\nc\nd\n";
        let text = "a\nB\nc\nnew\n";
        let found = hunks(base, text);
        assert_eq!(
            found.iter().map(|hunk| hunk.mark).collect::<Vec<_>>(),
            [
                ChangeMark {
                    start: 1,
                    end: 2,
                    kind: ChangeKind::Modified
                },
                ChangeMark {
                    start: 3,
                    end: 4,
                    kind: ChangeKind::Modified
                },
            ]
        );
        let removed = hunks("a\nb\nc\n", "a\nc\n");
        assert_eq!(
            removed[0].mark,
            ChangeMark {
                start: 1,
                end: 1,
                kind: ChangeKind::Deleted
            }
        );
        assert_eq!(removed[0].old, ["b\n"]);
        let added = hunks("a\n", "a\nb\n");
        assert_eq!(added[0].mark.kind, ChangeKind::Added);
    }

    #[test]
    fn rollback_restores_each_kind_of_block() {
        let base = "a\nb\nc\n";
        for text in ["a\nB\nc\n", "a\nc\n", "a\nb\nx\nc\n"] {
            let mut buffer = Doc::new(text);
            let mut engine = VimEngine::new();
            let found = hunks(base, text);
            rollback(&mut buffer, &mut engine, &found[0]);
            assert_eq!(buffer.text(), base, "rolling back {text:?}");
            engine.undo(&mut buffer);
            assert_eq!(buffer.text(), text);
        }
    }
}
