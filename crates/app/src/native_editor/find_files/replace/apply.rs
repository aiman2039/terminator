//! Running a replace, as the rename does: the plan is read and checked on
//! the file pool, open buffers are checked on the GUI thread, then the
//! staging writes every file or none on the pool. Clean open buffers then
//! reload as one undo step; an unsaved, loading, or saving buffer stops
//! the replace before anything is written.
use super::edits::Stale;
use super::*;
use crate::native_editor::disk::run_on_files;

/// Completes "Save x before …" for a busy or unsaved buffer.
const REPLACING: &str = "replacing in it";

/// A stage's worker answered.
enum Answer {
    Prepared(anyhow::Result<PreparedRename>, bool),
    Applied(Result<Vec<PathBuf>, String>, bool, usize),
}

impl ReplaceState {
    /// The answer of the stage in flight, once its worker sent one. A
    /// worker that stopped without one fails the stage.
    fn take_answer(&mut self) -> Option<Answer> {
        use tokio::sync::mpsc::error::TryRecvError;
        let answer = match &mut self.stage {
            Stage::Idle => return None,
            Stage::Preparing { rx, selected_only } => {
                let result = match rx.try_recv() {
                    Ok(result) => result,
                    Err(TryRecvError::Empty) => return None,
                    Err(TryRecvError::Disconnected) => Err(anyhow::anyhow!(
                        "The replace stopped before it read the files."
                    )),
                };
                Answer::Prepared(result, *selected_only)
            }
            Stage::Applying {
                rx,
                selected_only,
                edits,
            } => {
                let result = match rx.try_recv() {
                    Ok(result) => result,
                    Err(TryRecvError::Empty) => return None,
                    Err(TryRecvError::Disconnected) => {
                        Err("The replace stopped while it wrote files; check them.".into())
                    }
                };
                Answer::Applied(result, *selected_only, *edits)
            }
        };
        self.stage = Stage::Idle;
        Some(answer)
    }
}

impl App {
    /// Replace All (`selected_only` false) or Replace in Selected File:
    /// check the plan and open buffers, then read the files on the pool.
    pub(crate) fn start_replace(&mut self, selected_only: bool) {
        let Some(find) = self.find_in_files.as_ref() else {
            return;
        };
        let started = find.replace_plan(selected_only).and_then(|plan| {
            for file in &plan.files {
                self.open_doc_is_clean(&file.path, None, REPLACING)?;
            }
            let root = plan.root.clone();
            let job = (
                "find-in-files-replace",
                root.as_path(),
                Policy::ReplaceableRead,
            );
            run_on_files(&self.services, job, move || prepare_replace(&plan))
                .ok_or_else(|| "Could not start the replace.".to_owned())
        });
        let Some(find) = self.find_in_files.as_mut() else {
            return;
        };
        match started {
            Ok(rx) => {
                find.replace.stage = Stage::Preparing { rx, selected_only };
                find.replace.note = None;
            }
            Err(error) => find.replace_failed(error),
        }
    }

    /// Settle the replace in flight, whether or not the window shows.
    pub(crate) fn drive_replace(&mut self, ctx: &egui::Context) {
        let Some(find) = self.find_in_files.as_mut() else {
            return;
        };
        if find.replace.busy() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        match find.replace.take_answer() {
            Some(Answer::Prepared(result, selected_only)) => {
                self.replace_prepared(result, selected_only);
            }
            Some(Answer::Applied(result, selected_only, edits)) => {
                // A closed window gives the keyboard back, as Esc does.
                if let Some(id) = self.replace_applied(result, selected_only, edits) {
                    ctx.memory_mut(|memory| memory.request_focus(id));
                }
            }
            None => {}
        }
    }

    /// The files are read and checked: check open buffers against the
    /// bytes read, then write. Out-of-date results search again.
    fn replace_prepared(&mut self, result: anyhow::Result<PreparedRename>, selected_only: bool) {
        let next = match result {
            Ok(prepared) => self.apply_stage(prepared, selected_only),
            Err(error) if error.downcast_ref::<Stale>().is_some() => {
                if let Some(find) = self.find_in_files.as_mut() {
                    find.searched = None;
                }
                Err(format!(
                    "{error} The search ran again: check the results and replace again."
                ))
            }
            Err(error) => Err(format!("{error:#}")),
        };
        match next {
            Ok(stage) => {
                if let Some(find) = self.find_in_files.as_mut() {
                    find.replace.stage = stage;
                }
            }
            Err(error) => self.replace_error(error),
        }
    }

    /// A replace that stopped: the window's note, and the status bar when
    /// the window was closed while it ran.
    fn replace_error(&mut self, error: String) {
        match self.find_in_files.as_mut() {
            Some(find) if find.shown => find.replace_failed(error),
            Some(find) => {
                find.replace_failed(error.clone());
                self.info = Some(error);
            }
            None => self.info = Some(error),
        }
    }

    fn apply_stage(&self, prepared: PreparedRename, selected_only: bool) -> Result<Stage, String> {
        if prepared.file_count() == 0 {
            return Err("The replacement leaves the text as it is.".into());
        }
        self.open_docs_are_clean(&prepared, REPLACING)?;
        let edits = prepared.edit_count();
        let rx = self.start_rename_apply(prepared)?;
        Ok(Stage::Applying {
            rx,
            selected_only,
            edits,
        })
    }

    /// The files are written: reload their clean buffers (one undo step
    /// each), then close the window (Replace All) or drop the file's
    /// results (Replace in Selected File). Returns where the keyboard goes
    /// back to when the window closed.
    fn replace_applied(
        &mut self,
        result: Result<Vec<PathBuf>, String>,
        selected_only: bool,
        edits: usize,
    ) -> Option<egui::Id> {
        let paths = match result {
            Ok(paths) => paths,
            Err(error) => {
                self.replace_error(error);
                return None;
            }
        };
        let mut kept = false;
        for path in &paths {
            kept |= self.reload_renamed_doc(path, "Replace in Files");
        }
        self.lsp.invalidate_paths(&paths);
        let summary = replaced_summary(edits, paths.len(), kept);
        let Some(find) = self.find_in_files.as_mut() else {
            self.info = Some(summary);
            return None;
        };
        if !selected_only || !find.shown {
            let back = find.return_focus.take();
            find.hide();
            self.info = Some(summary);
            return back;
        }
        find.drop_files(&paths);
        find.replace.note = Some(Note {
            text: summary,
            error: false,
        });
        find.refocus();
        None
    }
}

/// "Replaced 3 matches in 2 files.", and a warning when an open buffer
/// with unsaved edits was kept.
fn replaced_summary(edits: usize, files: usize, kept: bool) -> String {
    let plural = |count: usize, one: &str, many: &str| {
        format!("{count} {}", if count == 1 { one } else { many })
    };
    let mut text = format!(
        "Replaced {} in {}.",
        plural(edits, "match", "matches"),
        plural(files, "file", "files")
    );
    if kept {
        text.push_str(" A buffer with unsaved edits was kept: review it before saving.");
    }
    text
}
