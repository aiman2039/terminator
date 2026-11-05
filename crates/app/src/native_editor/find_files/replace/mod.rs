//! Replace mode of the Find in Files window (`JetBrains` Replace in Path,
//! Cmd/Ctrl+Shift+R): a replacement (`$1` groups expand when regex is
//! on), include checkboxes per file and per result line, the old and new
//! text of each changed line, and Replace All / Replace in Selected File.
//! Files are written through the rename staging, off the GUI thread:
//! every file or none, with its size caps and open-buffer checks.
use super::*;
use crate::native_editor::rename::{MAX_RENAME_FILES, PreparedRename};
use std::collections::{BTreeSet, HashMap, HashSet};
use tokio::sync::mpsc::UnboundedReceiver;

mod apply;
mod edits;
mod view;

pub(crate) use edits::{LineReplace, ReplacePlan, change_line, prepare_replace};
pub(crate) use view::{DiffColors, ReplaceAsk, changes_pane, replace_buttons};

/// A replace's outcome, shown beside the Replace buttons.
pub(crate) struct Note {
    pub text: String,
    pub error: bool,
}

/// The replace in flight: reading the files, then writing them. Both
/// remember whether only the cursor's file was asked for.
#[derive(Default)]
enum Stage {
    #[default]
    Idle,
    Preparing {
        rx: UnboundedReceiver<anyhow::Result<PreparedRename>>,
        selected_only: bool,
    },
    Applying {
        rx: UnboundedReceiver<Result<Vec<PathBuf>, String>>,
        selected_only: bool,
        edits: usize,
    },
}

/// Replace mode state. The replacement survives closing the window; the
/// checkboxes reset with each new search.
#[derive(Default)]
pub(crate) struct ReplaceState {
    /// The replacement field, checkboxes, and Replace buttons show.
    pub on: bool,
    pub text: String,
    /// Focus the replacement field on the next paint; `select` also
    /// selects its text, so typing replaces it.
    pub focus: bool,
    pub select: bool,
    /// Results left out: whole files, and single lines by file.
    skipped_files: HashSet<PathBuf>,
    skipped_lines: HashMap<PathBuf, BTreeSet<u32>>,
    stage: Stage,
    pub note: Option<Note>,
    /// The preview's regex and its `$1` mode, for the query it came from.
    regex: Option<(crate::search::Query, regex::Regex)>,
    /// The changes pane's scroll offset, and the cursor it last showed.
    scroll: f32,
    shown_for: Option<(u64, usize)>,
}

impl ReplaceState {
    /// A replace is reading or writing files.
    pub fn busy(&self) -> bool {
        !matches!(self.stage, Stage::Idle)
    }

    /// New results: every line counts again.
    pub fn forget_results(&mut self) {
        self.skipped_files.clear();
        self.skipped_lines.clear();
        self.shown_for = None;
    }

    pub fn line_included(&self, path: &Path, line: u32) -> bool {
        !self.skipped_files.contains(path)
            && self
                .skipped_lines
                .get(path)
                .is_none_or(|lines| !lines.contains(&line))
    }

    /// The file's checkbox: checked while any of its lines counts, and
    /// partial while some do not.
    pub fn file_check(&self, file: &FileHits) -> (bool, bool) {
        let included = file
            .hits
            .iter()
            .filter(|hit| self.line_included(&file.path, hit.line))
            .count();
        (included > 0, included > 0 && included < file.hits.len())
    }

    /// Leave a checked file out; take an unchecked one back whole.
    fn toggle_file(&mut self, file: &FileHits) {
        if self.file_check(file).0 {
            self.skipped_files.insert(file.path.clone());
        } else {
            self.skipped_files.remove(&file.path);
            self.skipped_lines.remove(&file.path);
        }
    }

    /// Flip one line. Checking a line of a left-out file takes back only
    /// that line.
    fn toggle_line(&mut self, file: &FileHits, line: u32) {
        let path = &file.path;
        if self.line_included(path, line) {
            self.skipped_lines
                .entry(path.clone())
                .or_default()
                .insert(line);
        } else if self.skipped_files.remove(path) {
            let others = file
                .hits
                .iter()
                .map(|hit| hit.line)
                .filter(|&at| at != line);
            self.skipped_lines.insert(path.clone(), others.collect());
        } else if let Some(lines) = self.skipped_lines.get_mut(path) {
            lines.remove(&line);
        }
    }

    /// The counted lines of `file` with their text as the search saw it.
    fn planned(&self, file: &FileHits) -> Option<edits::PlannedFile> {
        let lines: Vec<(u32, String)> = file
            .hits
            .iter()
            .filter(|hit| self.line_included(&file.path, hit.line))
            .map(|hit| {
                let seen = file.lines.get(&hit.line).cloned().unwrap_or_default();
                (hit.line, seen)
            })
            .collect();
        (!lines.is_empty()).then(|| edits::PlannedFile {
            path: file.path.clone(),
            lines,
        })
    }

    fn fail(&mut self, error: String) {
        self.note = Some(Note {
            text: error,
            error: true,
        });
    }
}

impl FindInFiles {
    /// Switch replace mode. Turning it on with a query moves the keyboard
    /// to the replacement, selected.
    pub fn set_replace_mode(&mut self, on: bool) {
        self.replace.on = on;
        if on && !self.query.trim().is_empty() {
            self.focus = false;
            self.replace.focus = true;
            self.replace.select = true;
        }
    }

    /// Give the keyboard back to the field the mode types in.
    pub fn refocus(&mut self) {
        if self.replace.on {
            self.replace.focus = true;
        } else {
            self.focus = true;
        }
    }

    /// Show why a replace stopped. The fields were off while it ran, so
    /// the keyboard comes back too.
    fn replace_failed(&mut self, error: String) {
        self.replace.fail(error);
        self.refocus();
    }

    /// Flip a file's checkbox (`line` `None`) or one line's.
    pub fn toggle_include(&mut self, path: &Path, line: Option<u32>) {
        let Some(file) = self.files.iter().find(|file| file.path == path) else {
            return;
        };
        match line {
            Some(line) => self.replace.toggle_line(file, line),
            None => self.replace.toggle_file(file),
        }
    }

    /// Counted lines and the files they are in.
    pub fn included_counts(&self) -> (usize, usize) {
        self.files
            .iter()
            .fold((0usize, 0usize), |(lines, files), file| {
                let counted = file
                    .hits
                    .iter()
                    .filter(|hit| self.replace.line_included(&file.path, hit.line))
                    .count();
                (
                    lines.saturating_add(counted),
                    files.saturating_add(usize::from(counted > 0)),
                )
            })
    }

    /// Why the Replace buttons cannot run now, if they cannot.
    pub fn replace_blocked(&self) -> Option<&'static str> {
        if self.replace.busy() {
            Some("A replace is running")
        } else if self.running {
            Some("Wait for the search to finish")
        } else if self.truncated {
            Some("Too many results: refine the search")
        } else if self.included_counts().0 == 0 {
            Some("Check at least one result")
        } else {
            None
        }
    }

    /// What a replace changes: the counted lines of every file, or of the
    /// cursor's file only, under the query the results came from.
    pub fn replace_plan(&self, selected_only: bool) -> Result<ReplacePlan, String> {
        if let Some(why) = self.replace_blocked() {
            return Err(format!("{why}."));
        }
        let Some((query, _)) = self
            .searched
            .as_ref()
            .filter(|(query, _)| !query.is_empty())
        else {
            return Err("Type the text to find first.".into());
        };
        let chosen: Vec<&FileHits> = if selected_only {
            let (file, _) = self
                .selected_hit()
                .ok_or_else(|| "Pick a result first.".to_owned())?;
            vec![file]
        } else {
            self.files.iter().collect()
        };
        let files: Vec<edits::PlannedFile> = chosen
            .into_iter()
            .filter_map(|file| self.replace.planned(file))
            .collect();
        if files.is_empty() {
            return Err("Nothing to replace: check at least one line of this file.".into());
        }
        if files.len() > MAX_RENAME_FILES {
            return Err(format!(
                "Replace in Files changes at most {MAX_RENAME_FILES} files at once. \
                 Uncheck some files or narrow the search."
            ));
        }
        Ok(ReplacePlan {
            root: self.root.clone(),
            query: query.clone(),
            replacement: self.replace.text.clone(),
            files,
        })
    }

    /// The regex the results came from, and whether `$1` groups expand.
    pub fn preview_regex(&mut self) -> Option<(regex::Regex, bool)> {
        let (query, _) = self.searched.as_ref()?;
        if self
            .replace
            .regex
            .as_ref()
            .is_none_or(|(key, _)| key != query)
        {
            let regex = crate::search::build_typed_regex(query).ok()?;
            self.replace.regex = Some((query.clone(), regex));
        }
        let (query, regex) = self.replace.regex.as_ref()?;
        Some((regex.clone(), query.use_regex))
    }

    /// Drop the results of files a replace wrote; the rest stay listed.
    fn drop_files(&mut self, paths: &[PathBuf]) {
        self.files.retain(|file| !paths.contains(&file.path));
        self.total = self
            .files
            .iter()
            .fold(0usize, |total, file| total.saturating_add(file.hits.len()));
        self.selected = self.selected.min(self.total.saturating_sub(1));
        self.preview = None;
        self.reveal = true;
        self.replace.shown_for = None;
    }
}

#[cfg(test)]
mod tests;
