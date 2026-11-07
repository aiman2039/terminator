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
    /// Commit page prefix on the remote host, when known.
    commit_web: Option<String>,
    annotations: Arc<Vec<Annotation>>,
    /// Blank column of the same width, shown while lines have moved.
    blank: Arc<Vec<Annotation>>,
    /// Line hashes of the text sent with the request in flight, and of
    /// the text the answer describes.
    sent: Option<(u64, Arc<Vec<u64>>)>,
    answered_lines: Arc<Vec<u64>>,
    /// The answer moved onto a newer revision (see [`shift_annotations`]).
    shifted: Option<(u64, Arc<Vec<Annotation>>)>,
}

impl EditorBlame {
    /// The column at the buffer's revision: current annotations, or
    /// while a fresh blame is pending the last answer moved onto the
    /// edited lines (blank only where lines changed).
    pub(crate) fn column(&mut self, buffer: &Doc) -> Arc<Vec<Annotation>> {
        let revision = buffer.revision();
        if self.answered.is_none() {
            return Arc::clone(&self.blank);
        }
        if self.answered == Some(revision) {
            return Arc::clone(&self.annotations);
        }
        if let Some((shifted, column)) = &self.shifted
            && *shifted == revision
        {
            return Arc::clone(column);
        }
        let blank = self.blank.first().cloned().unwrap_or(Annotation {
            text: "".into(),
            hover: "".into(),
            group: 0,
        });
        let column = Arc::new(shift_annotations(
            &self.annotations,
            (&self.answered_lines, buffer),
            &blank,
        ));
        self.shifted = Some((revision, Arc::clone(&column)));
        column
    }
}

/// Hash of each buffer line, to line up an older blame with new text.
fn line_hashes(buffer: &Doc) -> Vec<u64> {
    (0..buffer.line_count())
        .map(|line| buffer.line_hash(line))
        .collect()
}

/// Move `annotations` (for line hashes `old`) onto the lines of `new`:
/// the unchanged lines before and after the edited region keep their
/// annotation, the edited lines in between get `blank`. Only the lines
/// up to the first difference from each end are hashed, so typing
/// stays cheap.
fn shift_annotations(
    annotations: &[Annotation],
    (old, new): (&[u64], &Doc),
    blank: &Annotation,
) -> Vec<Annotation> {
    let new_len = new.line_count();
    let same = |hash: &u64, line: usize| new.line_hash(line) == *hash;
    let prefix = old
        .iter()
        .zip(0..new_len)
        .take_while(|(hash, line)| same(hash, *line))
        .count();
    let room = old.len().min(new_len).saturating_sub(prefix);
    let suffix = old
        .iter()
        .rev()
        .zip((0..new_len).rev())
        .take(room)
        .take_while(|(hash, line)| same(hash, *line))
        .count();
    let tail_start = new_len.saturating_sub(suffix);
    (0..new_len)
        .map(|line| {
            let source = if line < prefix {
                Some(line)
            } else if line >= tail_start {
                old.len().checked_sub(new_len.saturating_sub(line))
            } else {
                None
            };
            source
                .and_then(|at| annotations.get(at))
                .unwrap_or(blank)
                .clone()
        })
        .collect()
}

/// `YYYY-MM-DD` for Unix seconds (UTC; proleptic Gregorian).
#[expect(
    clippy::arithmetic_side_effects,
    reason = "`doe` < 146097 bounds every step."
)]
pub(crate) fn ymd(seconds: i64) -> String {
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
/// `web_host` names the remote host that Cmd/Ctrl+click opens.
pub(crate) fn annotations(entries: &[BlameEntry], web_host: Option<&str>) -> Vec<Annotation> {
    let web_hint = web_host.map_or(String::new(), |host| {
        let key = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        format!("\n{key}+click to open the commit on {host}")
    });
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
                    hover: "".into(),
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
                text: format!("{date} {}", entry.author).into(),
                hover: format!(
                    "{short} · {} · {date}\n{}\n\nClick to show this commit's change to the file{web_hint}",
                    entry.author, entry.summary
                )
                .into(),
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
                commit_web: None,
                annotations: Arc::new(Vec::new()),
                blank: Arc::new(Vec::new()),
                sent: None,
                answered_lines: Arc::new(Vec::new()),
                shifted: None,
            },
        );
    }

    /// The blame column for `path` at its buffer's revision.
    #[cfg(test)]
    pub(crate) fn editor_blame_annotations(
        &mut self,
        path: &Path,
        buffer: &Doc,
    ) -> Option<Arc<Vec<Annotation>>> {
        Some(self.editor_blame.get_mut(path)?.column(buffer))
    }

    /// Ask for blame once the buffer has been quiet for a moment (right
    /// away for the first answer). Runs every frame the editor paints.
    pub(crate) fn request_editor_blame(&mut self, ctx: &egui::Context, path: &Path) {
        let Some(native) = self.native_docs.get(path) else {
            return;
        };
        let Some(buffer) = native.doc.as_ref() else {
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
        // `git blame --contents` matches lines against the committed bytes:
        // send the file's own line endings, not the LF-only buffer.
        blame.sent = Some((revision, Arc::new(line_hashes(buffer))));
        let text = native.line_ending.encode(buffer.text());
        let _ = self
            .jobs
            .send(Job::EditorBlame(path.to_owned(), revision, text));
    }

    pub(crate) fn apply_editor_blame(
        &mut self,
        path: &Path,
        revision: u64,
        result: Result<crate::git_log::BlameData, String>,
    ) {
        let Some(blame) = self.editor_blame.get_mut(path) else {
            return;
        };
        if blame.pending == Some(revision) {
            blame.pending = None;
        }
        match result {
            Ok(data) => {
                let host = data.commit_web.as_deref().map(crate::git_log::commit_host);
                let annotations = annotations(&data.entries, host);
                let width = annotations
                    .iter()
                    .map(|annotation| annotation.text.chars().count())
                    .max()
                    .unwrap_or(0);
                blame.blank = Arc::new(vec![Annotation {
                    text: " ".repeat(width).into(),
                    hover: "".into(),
                    group: 0,
                }]);
                blame.annotations = Arc::new(annotations);
                blame.entries = data.entries;
                blame.commit_web = data.commit_web;
                blame.answered = Some(revision);
                blame.shifted = None;
                if let Some((_, lines)) = blame.sent.take_if(|(sent, _)| *sent == revision) {
                    blame.answered_lines = lines;
                }
            }
            Err(error) => {
                self.editor_blame.remove(path);
                self.error.set(format!("Git blame unavailable: {error}"));
            }
        }
    }

    /// Open the commit behind the clicked annotation: its diff of this
    /// file, or with `web` its page on the remote host in a Browser tab.
    pub(crate) fn open_blame_commit(&mut self, path: &Path, line: usize, web: bool) {
        let Some(blame) = self.editor_blame.get(path) else {
            return;
        };
        let Some(entry) = blame.entries.get(line) else {
            return;
        };
        if entry.uncommitted() {
            self.info = Some("This line is not committed yet".into());
            return;
        }
        if web
            && let Some(base) = &blame.commit_web
            && let Some(project) = self.selected.clone()
        {
            let url = format!("{base}{}", entry.hash);
            if let Err(error) = self.open_browser_url(&project, &url, None) {
                self.error.set(format!("{error:#}"));
            }
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
        let built = annotations(
            &[
                entry(&"a".repeat(40), 1_709_251_200),
                entry(&"a".repeat(40), 1_709_251_200),
                entry(&"b".repeat(40), 0),
                entry(&zero, 0),
            ],
            Some("github.com"),
        );
        assert_eq!(&*built[0].text, "2024-03-01 Ada");
        assert!(built[0].hover.starts_with("aaaaaaaa · Ada"));
        assert_eq!(
            built.iter().map(|a| a.group).collect::<Vec<_>>(),
            [0, 0, 1, 2]
        );
        assert_eq!(&*built[3].text, "Not committed");
        assert!(built[3].hover.is_empty());
        assert!(built[0].hover.contains("open the commit on github.com"));
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
        let entries = crate::git_log::blame_with_contents(root, &path, Some(&text)).unwrap();
        app.apply_editor_blame(&path, revision, Ok(entries));
        let mut buffer = app.native_docs[&path].doc.clone().unwrap();
        let shown = app.editor_blame_annotations(&path, &buffer).unwrap();
        assert!(shown[0].text.ends_with(" Ada"), "{:?}", shown[0]);
        assert_eq!(&*shown[1].text, "Not committed");
        // Typing a line above keeps the column: old lines move down, and
        // only the new line is blank until blame catches up.
        buffer.insert(0, "zero\n");
        let moved = app.editor_blame_annotations(&path, &buffer).unwrap();
        assert!(moved[0].text.trim().is_empty());
        assert_eq!(moved[1].text, shown[0].text);
        assert_eq!(&*moved[2].text, "Not committed");
        app.toggle_editor_blame(&path);
        assert!(app.editor_blame_annotations(&path, &buffer).is_none());
    }
}
