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

/// Diff identity: buffer revision plus the `HEAD` text's address.
type GutterKey = (u64, usize);

/// One finished diff: its key, the hunks, and their marks.
type GutterDone = (GutterKey, Arc<Vec<GutterHunk>>, Arc<Vec<ChangeMark>>);

/// Change marks for one buffer. The line diff (up to 500 ms on a large
/// file) runs on a worker thread keyed by [`GutterKey`]; the GUI thread
/// only snapshots the text once per revision and never waits. At most
/// one diff runs per buffer; edits made meanwhile are diffed when it
/// lands, newest revision only.
#[derive(Default)]
pub(crate) struct GutterState {
    done: Option<GutterDone>,
    running: Option<(GutterKey, std::sync::mpsc::Receiver<Vec<GutterHunk>>)>,
}

pub(crate) struct GutterRequest<'a> {
    pub ctx: &'a egui::Context,
    pub revision: u64,
    pub base: &'a Arc<str>,
    pub crlf: bool,
    pub text: &'a dyn Fn() -> String,
}

impl GutterState {
    /// Hunks and marks for this revision. While a diff for it is still
    /// running, the last marks stay painted but no hunks are returned:
    /// a stale hunk must never drive Rollback onto shifted lines.
    pub(crate) fn current(
        &mut self,
        request: &GutterRequest<'_>,
    ) -> (Arc<Vec<GutterHunk>>, Arc<Vec<ChangeMark>>) {
        let key = (
            request.revision,
            Arc::as_ptr(request.base).cast::<u8>() as usize,
        );
        self.adopt_finished();
        if let Some((done_key, hunks, marks)) = self.done.as_ref()
            && *done_key == key
        {
            return (Arc::clone(hunks), Arc::clone(marks));
        }
        if self.running.is_none() {
            self.start(key, request);
        }
        let stale = self
            .done
            .as_ref()
            .map(|(_, _, marks)| Arc::clone(marks))
            .unwrap_or_default();
        (Arc::default(), stale)
    }

    fn adopt_finished(&mut self) {
        let Some((key, rx)) = self.running.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(hunks) => {
                let marks = Arc::new(hunks.iter().map(|hunk| hunk.mark).collect::<Vec<_>>());
                self.done = Some((*key, Arc::new(hunks), marks));
                self.running = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.running = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    fn start(&mut self, key: GutterKey, request: &GutterRequest<'_>) {
        let text = (request.text)();
        let base = Arc::clone(request.base);
        let crlf = request.crlf;
        let ctx = request.ctx.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("gutter-diff".into())
            .spawn(move || {
                let base_text = if crlf {
                    base.replace("\r\n", "\n")
                } else {
                    base.to_string()
                };
                let _ = tx.send(hunks(&base_text, &text));
                ctx.request_repaint();
            });
        if spawned.is_ok() {
            self.running = Some((key, rx));
        }
    }
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

pub(crate) struct PopupUiInput<'a> {
    pub(crate) ui: &'a egui::Ui,
    pub(crate) popup: &'a mut Option<GutterPopup>,
    pub(crate) hunks: &'a [GutterHunk],
    pub(crate) deleted: egui::Color32,
}

/// Paint the popup; returns true when Rollback was pressed, and clears
/// `popup` when the user closes it.
pub(crate) fn popup_ui(input: PopupUiInput<'_>) -> bool {
    let PopupUiInput {
        ui,
        popup,
        hunks,
        deleted,
    } = input;
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
    fn gutter_diff_runs_off_thread_and_never_returns_stale_hunks() {
        let ctx = egui::Context::default();
        let base: Arc<str> = Arc::from("a\nb\n");
        let mut state = GutterState::default();
        let first_text = || "a\nB\n".to_owned();
        let first = GutterRequest {
            ctx: &ctx,
            revision: 1,
            base: &base,
            crlf: false,
            text: &first_text,
        };
        let (hunks, _) = state.current(&first);
        assert!(hunks.is_empty(), "the diff is not ready on the first frame");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut ready = Arc::default();
        while std::time::Instant::now() < deadline {
            ready = state.current(&first).0;
            if !ready.is_empty() {
                break;
            }
            std::thread::yield_now();
        }
        assert_eq!(ready.len(), 1);
        // A new revision keeps the old marks painted but hides hunks.
        let second_text = || "a\nb\n".to_owned();
        let (hunks, marks) = state.current(&GutterRequest {
            revision: 2,
            text: &second_text,
            ..first
        });
        assert!(hunks.is_empty());
        assert_eq!(marks.len(), 1);
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
