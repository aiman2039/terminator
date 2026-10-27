//! Git blame in the editor gutter (`IntelliJ` Annotate with Git Blame):
//! a date and author per line, the commit summary on hover, and a click
//! opens that commit's diff of the file. Blame runs in the background on
//! the buffer's own text (`git blame --contents`), so annotations line up
//! with unsaved edits, which read as "Not committed".

use super::*;
use crate::git_log::BlameEntry;
use std::sync::Arc;
use terminator_native_edit::view::Annotation;

/// Quiet time after an edit before blame runs again.
const BLAME_SETTLE: Duration = Duration::from_millis(700);

/// Annotation state for one file; present while annotations are on.
pub(crate) struct EditorBlame {
    /// Buffer revision the entries describe.
    answered: Option<u64>,
    /// Buffer revision of the request in flight.
    pending: Option<u64>,
    /// Last buffer revision seen, and when it was first seen.
    seen: (u64, Instant),
    entries: Vec<BlameEntry>,
    annotations: Arc<Vec<Annotation>>,
    /// Blank column of the same width, shown while lines have moved.
    blank: Arc<Vec<Annotation>>,
}

/// `YYYY-MM-DD` for Unix seconds (UTC; proleptic Gregorian).
#[expect(
    clippy::arithmetic_side_effects,
    reason = "`doe` < 146097 bounds every step."
)]
fn ymd(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Gutter annotations for blame entries: `date author` per line, commits
/// alternating groups so neighboring commits read as separate blocks.
pub(crate) fn annotations(entries: &[BlameEntry]) -> Vec<Annotation> {
    let mut group = 0u32;
    let mut previous: Option<&str> = None;
    entries
        .iter()
        .map(|entry| {
            if previous.is_some_and(|hash| hash != entry.hash) {
                group = group.wrapping_add(1);
            }
            previous = Some(&entry.hash);
            if entry.uncommitted() {
                return Annotation {
                    text: "Not committed".into(),
                    hover: String::new(),
                    group,
                };
            }
            let date = if entry.time > 0 {
                ymd(entry.time)
            } else {
                String::new()
            };
            let short: String = entry.hash.chars().take(8).collect();
            Annotation {
                text: format!("{date} {}", entry.author),
                hover: format!(
                    "{short} · {} · {date}\n{}\n\nClick to show this commit's change to the file",
                    entry.author, entry.summary
                ),
                group,
            }
        })
        .collect()
}

impl App {
    /// Show or hide blame annotations for `path`.
    pub(crate) fn toggle_editor_blame(&mut self, path: &Path) {
        if self.editor_blame.remove(path).is_some() {
            return;
        }
        self.editor_blame.insert(
            path.to_owned(),
            EditorBlame {
                answered: None,
                pending: None,
                seen: (0, Instant::now()),
                entries: Vec::new(),
                annotations: Arc::new(Vec::new()),
                blank: Arc::new(Vec::new()),
            },
        );
    }

    /// The column for `path` at buffer `revision`: current annotations, or
    /// a blank column of the same width while a fresh blame is pending.
    pub(crate) fn editor_blame_annotations(
        &self,
        path: &Path,
        revision: u64,
    ) -> Option<Arc<Vec<Annotation>>> {
        let blame = self.editor_blame.get(path)?;
        Some(if blame.answered == Some(revision) {
            Arc::clone(&blame.annotations)
        } else {
            Arc::clone(&blame.blank)
        })
    }

    /// Ask for blame once the buffer has been quiet for a moment (right
    /// away for the first answer). Runs every frame the editor paints.
    pub(crate) fn request_editor_blame(&mut self, ctx: &egui::Context, path: &Path) {
        let Some(buffer) = self.native_docs.get(path).and_then(|doc| doc.doc.as_ref()) else {
            return;
        };
        let revision = buffer.revision();
        let Some(blame) = self.editor_blame.get_mut(path) else {
            return;
        };
        if blame.seen.0 != revision {
            blame.seen = (revision, Instant::now());
        }
        if blame.answered == Some(revision) || blame.pending == Some(revision) {
            return;
        }
        let waited = blame.seen.1.elapsed();
        if blame.answered.is_some() && waited < BLAME_SETTLE {
            ctx.request_repaint_after(BLAME_SETTLE.saturating_sub(waited));
            return;
        }
        blame.pending = Some(revision);
        let text = buffer.text();
        let _ = self
            .jobs
            .send(Job::EditorBlame(path.to_owned(), revision, text));
    }

    pub(crate) fn apply_editor_blame(
        &mut self,
        path: &Path,
        revision: u64,
        result: Result<Vec<BlameEntry>, String>,
    ) {
        let Some(blame) = self.editor_blame.get_mut(path) else {
            return;
        };
        if blame.pending == Some(revision) {
            blame.pending = None;
        }
        match result {
            Ok(entries) => {
                let annotations = annotations(&entries);
                let width = annotations
                    .iter()
                    .map(|annotation| annotation.text.chars().count())
                    .max()
                    .unwrap_or(0);
                blame.blank = Arc::new(vec![Annotation {
                    text: " ".repeat(width),
                    hover: String::new(),
                    group: 0,
                }]);
                blame.annotations = Arc::new(annotations);
                blame.entries = entries;
                blame.answered = Some(revision);
            }
            Err(error) => {
                self.editor_blame.remove(path);
                self.error = Some(format!("Git blame unavailable: {error}"));
            }
        }
    }

    /// Open the commit behind the clicked annotation: its diff of this file.
    pub(crate) fn open_blame_commit(&mut self, path: &Path, line: usize) {
        let Some(entry) = self
            .editor_blame
            .get(path)
            .and_then(|blame| blame.entries.get(line))
        else {
            return;
        };
        if entry.uncommitted() {
            self.info = Some("This line is not committed yet".into());
            return;
        }
        let Some(cwd) = path.parent().map(Path::to_path_buf) else {
            return;
        };
        let hash = entry.hash.clone();
        self.open_git_tab(Tab::RevisionDiff {
            cwd,
            path: path.to_owned(),
            left: format!("{hash}^"),
            right: hash,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(hash: &str, time: i64) -> BlameEntry {
        BlameEntry {
            hash: hash.into(),
            author: "Ada".into(),
            date: String::new(),
            line: 0,
            code: String::new(),
            time,
            summary: "fix".into(),
        }
    }

    #[test]
    fn annotations_show_date_and_author_and_alternate_by_commit() {
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(ymd(1_709_251_200), "2024-03-01");
        let zero = "0".repeat(40);
        let built = annotations(&[
            entry(&"a".repeat(40), 1_709_251_200),
            entry(&"a".repeat(40), 1_709_251_200),
            entry(&"b".repeat(40), 0),
            entry(&zero, 0),
        ]);
        assert_eq!(built[0].text, "2024-03-01 Ada");
        assert!(built[0].hover.starts_with("aaaaaaaa · Ada"));
        assert_eq!(
            built.iter().map(|a| a.group).collect::<Vec<_>>(),
            [0, 0, 1, 2]
        );
        assert_eq!(built[3].text, "Not committed");
        assert!(built[3].hover.is_empty());
    }

    #[test]
    fn blame_runs_on_the_buffer_and_blanks_while_an_edit_is_pending() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(root)
                    .env("GIT_AUTHOR_NAME", "Ada")
                    .env("GIT_AUTHOR_EMAIL", "a@example.com")
                    .env("GIT_COMMITTER_NAME", "Ada")
                    .env("GIT_COMMITTER_EMAIL", "a@example.com")
                    .status()
                    .unwrap()
                    .success()
            );
        };
        git(&["init", "-q"]);
        let path = root.canonicalize().unwrap().join("a.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "start"]);
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, terminator_core::Paths::at(root.join("state")));
        let (jobs, received) = std::sync::mpsc::channel();
        app.jobs = jobs.into();
        app.native_docs
            .insert(path.clone(), NativeDoc::loaded(path.clone(), "one\nTWO\n"));
        app.toggle_editor_blame(&path);
        app.request_editor_blame(&ctx, &path);
        let Some(Job::EditorBlame(asked, revision, text)) = received.try_iter().next() else {
            panic!("blame requested at once");
        };
        assert_eq!(
            (asked.as_path(), text.as_str()),
            (path.as_path(), "one\nTWO\n")
        );
        // A second frame does not ask again for the same revision.
        app.request_editor_blame(&ctx, &path);
        assert!(received.try_iter().next().is_none());
        let entries = crate::git_log::blame_with_contents(root, &path, Some(&text))
            .unwrap()
            .entries;
        app.apply_editor_blame(&path, revision, Ok(entries));
        let shown = app.editor_blame_annotations(&path, revision).unwrap();
        assert!(shown[0].text.ends_with(" Ada"), "{:?}", shown[0]);
        assert_eq!(shown[1].text, "Not committed");
        // After an edit the column stays, blank, until blame catches up.
        let blank = app
            .editor_blame_annotations(&path, revision.wrapping_add(1))
            .unwrap();
        assert!(
            blank
                .iter()
                .all(|annotation| annotation.text.trim().is_empty())
        );
        app.toggle_editor_blame(&path);
        assert!(app.editor_blame_annotations(&path, revision).is_none());
    }
}
