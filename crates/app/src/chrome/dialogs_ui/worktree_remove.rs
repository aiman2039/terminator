//! Remove worktree: a checklist of what blocks the removal, read before the
//! user confirms. The checks are advisory and read-only (live sessions from
//! the GUI state, `git status` and ignored files from a background job);
//! the daemon still refuses a blocked removal, and that refusal still shows
//! in the status bar. The branch is never deleted and nothing is forced.
use crate::*;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// What Git reports for the checkout. `None` fields could not be read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GitPreflight {
    /// Uncommitted (tracked or untracked) entries.
    pub changes: Option<usize>,
    /// Ignored files and folders Git would delete, as `git` lists them.
    pub ignored: Option<Vec<String>>,
    /// Commits not on the upstream; `None` without an upstream.
    pub unpushed: Option<usize>,
}

/// The Git check of the open Remove worktree dialog.
#[derive(Default)]
pub(crate) struct RemoveCheck {
    project: String,
    running: Option<Receiver<GitPreflight>>,
    result: Option<GitPreflight>,
}

impl RemoveCheck {
    /// Start the check for `project` at `path` once; later calls poll it.
    fn poll(&mut self, project: &str, path: &Path) -> Option<&GitPreflight> {
        if self.project != project {
            *self = Self {
                project: project.to_owned(),
                ..Self::default()
            };
            let (sender, receiver) = channel();
            let path = path.to_path_buf();
            let spawned = std::thread::Builder::new()
                .name("worktree-preflight".into())
                .spawn(move || {
                    let _ = sender.send(git_preflight(&path));
                });
            if spawned.is_ok() {
                self.running = Some(receiver);
            } else {
                self.result = Some(GitPreflight::default());
            }
        }
        if let Some(receiver) = &self.running {
            match receiver.try_recv() {
                Ok(result) => {
                    self.result = Some(result);
                    self.running = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.result = Some(GitPreflight::default());
                    self.running = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        self.result.as_ref()
    }
}

/// Read-only Git queries for one checkout.
fn git_preflight(path: &Path) -> GitPreflight {
    let changes = git_lines(path, &["status", "--porcelain"]).map(|lines| lines.len());
    let ignored = git_lines(
        path,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ],
    );
    let unpushed = git_lines(path, &["rev-list", "--count", "@{upstream}..HEAD"])
        .and_then(|lines| lines.first().and_then(|count| count.trim().parse().ok()));
    GitPreflight {
        changes,
        ignored,
        unpushed,
    }
}

fn git_lines(path: &Path, args: &[&str]) -> Option<Vec<String>> {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(path)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then(|| {
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

/// One checklist row: whether it blocks, its label and its detail.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CheckRow {
    pub blocks: bool,
    /// Still being read.
    pub pending: bool,
    pub label: &'static str,
    pub detail: String,
}

/// The four rows, in order: Sessions, Changes, Ignored files, Branch. Only
/// the Branch row never blocks.
pub(crate) fn check_rows(live: usize, git: Option<&GitPreflight>) -> Vec<CheckRow> {
    let sessions = CheckRow::done(
        "Sessions",
        live > 0,
        if live == 0 {
            "No live sessions in this checkout".into()
        } else {
            format!(
                "{} running here. Stop them first.",
                appearance::count_noun(live, "live session")
            )
        },
    );
    let Some(git) = git else {
        return std::iter::once(sessions)
            .chain(["Changes", "Ignored files", "Branch"].map(CheckRow::pending))
            .collect();
    };
    vec![
        sessions,
        changes_row(git),
        ignored_row(git),
        branch_row(git),
    ]
}

impl CheckRow {
    fn done(label: &'static str, blocks: bool, detail: String) -> Self {
        Self {
            blocks,
            pending: false,
            label,
            detail,
        }
    }

    fn pending(label: &'static str) -> Self {
        Self {
            pending: true,
            ..Self::done(label, false, "Checking…".into())
        }
    }

    fn unknown(label: &'static str) -> Self {
        Self::done(label, false, "Could not read".into())
    }
}

fn changes_row(git: &GitPreflight) -> CheckRow {
    let Some(count) = git.changes else {
        return CheckRow::unknown("Changes");
    };
    let detail = if count == 0 {
        "No uncommitted files".into()
    } else {
        format!(
            "{}. Commit or discard them in Git first.",
            appearance::count_noun(count, "uncommitted file")
        )
    };
    CheckRow::done("Changes", count > 0, detail)
}

fn ignored_row(git: &GitPreflight) -> CheckRow {
    let Some(paths) = &git.ignored else {
        return CheckRow::unknown("Ignored files");
    };
    let detail = if paths.is_empty() {
        "None".into()
    } else {
        format!(
            "{}. Git would delete them: move or delete them first.",
            ignored_summary(paths)
        )
    };
    CheckRow::done("Ignored files", !paths.is_empty(), detail)
}

/// Unpushed commits never block: the branch is kept.
fn branch_row(git: &GitPreflight) -> CheckRow {
    let detail = match git.unpushed {
        None => "No upstream. The branch is kept.".into(),
        Some(0) => "Pushed. The branch is kept.".into(),
        Some(count) => format!(
            "{} not pushed. The branch is kept.",
            appearance::count_noun(count, "commit")
        ),
    };
    CheckRow::done("Branch", false, detail)
}

/// The first three ignored paths, then how many more.
fn ignored_summary(paths: &[String]) -> String {
    let shown: Vec<&str> = paths.iter().take(3).map(String::as_str).collect();
    let more = paths.len().saturating_sub(shown.len());
    if more == 0 {
        shown.join(", ")
    } else {
        format!("{} and {more} more", shown.join(", "))
    }
}

/// Why Remove is disabled: the first blocking row, or a running check.
fn remove_blocker(rows: &[CheckRow]) -> Option<String> {
    if let Some(row) = rows.iter().find(|row| row.blocks) {
        return Some(row.detail.clone());
    }
    rows.iter()
        .any(|row| row.pending)
        .then(|| "Checking the worktree…".to_owned())
}

impl App {
    /// Live sessions (terminals and editors, of any project) whose folder
    /// is in the checkout or that belong to its project.
    fn worktree_live_sessions(&self, project_id: &str, path: &Path) -> usize {
        self.state
            .sessions
            .iter()
            .filter(|session| session.lifecycle.live())
            .filter(|session| session.project_id == project_id || session.cwd.starts_with(path))
            .count()
    }

    pub(crate) fn worktree_remove_dialog(&mut self, ctx: &egui::Context) {
        let Some(project_id) = self.worktree_remove.clone() else {
            self.worktree_remove_check = RemoveCheck::default();
            return;
        };
        let Some(record) = self.managed_worktree(&project_id).cloned() else {
            self.worktree_remove = None;
            return;
        };
        let name = self
            .state
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .map(|project| project.name.clone())
            .unwrap_or_else(|| "Worktree".into());
        let path: PathBuf = record.path.clone();
        let live = self.worktree_live_sessions(&project_id, &path);
        let git = self.worktree_remove_check.poll(&project_id, &path).cloned();
        if git.is_none() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let rows = check_rows(live, git.as_ref());
        let blocker = remove_blocker(&rows);
        let window = self.popups.window(ctx, "Remove worktree?");
        let dialog = appearance::Confirm::new(
            format!("Remove {name} at {}?", path.display()),
            "Git refuses a checkout with changes, ignored files or a lock. The branch is kept.",
        )
        .destructive(
            appearance::ConfirmButton::new("Remove worktree")
                .id("remove-worktree")
                .disabled(blocker.as_deref()),
        );
        let mut open_git = false;
        let answer = appearance::confirm(ctx, window, dialog, |ui| {
            open_git = self.worktree_check_rows(ui, &rows);
        });
        if open_git {
            self.worktree_remove = None;
            self.select_project(project_id);
            self.show_git_sidebar();
            return;
        }
        match answer {
            Some(appearance::ConfirmAnswer::Destructive) => {
                self.send(Request::WorktreeRemove {
                    project: project_id.clone(),
                });
                self.worktree_remove = None;
            }
            Some(_) => self.worktree_remove = None,
            None => {}
        }
    }

    /// The checklist; returns true when Open Git was clicked.
    fn worktree_check_rows(&self, ui: &mut egui::Ui, rows: &[CheckRow]) -> bool {
        let mut open_git = false;
        ui.add_space(8.0);
        egui::Grid::new("worktree-remove-checks")
            .num_columns(3)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                for row in rows {
                    let (icon, tint) = if row.pending {
                        ("LoaderCircle", ui.visuals().weak_text_color())
                    } else if row.blocks {
                        ("CircleAlert", appearance::color(&self.theme.status_waiting))
                    } else {
                        ("CircleCheck", appearance::color(&self.theme.status_running))
                    };
                    ui.add(
                        egui::Image::new(crate::icons::source(icon))
                            .tint(tint)
                            .fit_to_exact_size(egui::Vec2::splat(appearance::TOOLBAR_ICON)),
                    );
                    ui.label(row.label);
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(&row.detail).weak()).wrap());
                        if row.label == "Changes" && row.blocks && ui.button("Open Git").clicked() {
                            open_git = true;
                        }
                    });
                    #[cfg(feature = "test-support")]
                    diagnostics::record_ui(
                        ui,
                        &format!("worktree-check:{}", row.label),
                        ui.min_rect(),
                    );
                    ui.end_row();
                }
            });
        open_git
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_checkout_without_sessions_can_be_removed() {
        let git = GitPreflight {
            changes: Some(0),
            ignored: Some(Vec::new()),
            unpushed: Some(2),
        };
        let rows = check_rows(0, Some(&git));
        assert_eq!(remove_blocker(&rows), None);
        // Unpushed commits never block: the branch is kept.
        assert_eq!(rows[3].detail, "2 commits not pushed. The branch is kept.");
    }

    #[test]
    fn the_first_blocker_explains_the_disabled_remove() {
        let git = GitPreflight {
            changes: Some(3),
            ignored: Some(vec!["target/".into(), ".env".into()]),
            unpushed: None,
        };
        let rows = check_rows(2, Some(&git));
        assert_eq!(
            remove_blocker(&rows).as_deref(),
            Some("2 live sessions running here. Stop them first.")
        );
        let rows = check_rows(0, Some(&git));
        assert!(
            remove_blocker(&rows)
                .unwrap()
                .starts_with("3 uncommitted files")
        );
        let clean = GitPreflight {
            changes: Some(0),
            ..git
        };
        let rows = check_rows(0, Some(&clean));
        assert_eq!(
            remove_blocker(&rows).as_deref(),
            Some("target/, .env. Git would delete them: move or delete them first.")
        );
    }

    #[test]
    fn remove_waits_for_the_git_check_but_an_unreadable_check_does_not_block() {
        assert_eq!(
            remove_blocker(&check_rows(0, None)).as_deref(),
            Some("Checking the worktree…")
        );
        let unreadable = GitPreflight::default();
        assert_eq!(remove_blocker(&check_rows(0, Some(&unreadable))), None);
    }

    #[test]
    fn many_ignored_paths_name_three_and_count_the_rest() {
        let paths: Vec<String> = ["a/", "b/", "c/", "d/", "e/"].map(str::to_owned).to_vec();
        assert_eq!(ignored_summary(&paths), "a/, b/, c/ and 2 more");
    }

    #[test]
    fn preflight_reads_changes_and_ignored_files_of_a_real_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(repo)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        };
        if !git(&["init", "-q"]) {
            return;
        }
        std::fs::write(repo.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir(repo.join("target")).unwrap();
        std::fs::write(repo.join("target/out"), "x").unwrap();
        let found = git_preflight(repo);
        assert_eq!(found.changes, Some(1), ".gitignore is untracked");
        assert_eq!(found.ignored, Some(vec!["target/".to_owned()]));
        assert_eq!(found.unpushed, None);
    }
}
