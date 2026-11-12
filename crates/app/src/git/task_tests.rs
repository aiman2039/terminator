//! Task worktrees against real `git` in temporary repositories.
use super::task_changes::{TaskFile, list_changes, task_base, totals};
use std::path::{Path, PathBuf};

pub(super) fn git(root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .env("GIT_AUTHOR_NAME", "Ada")
        .env("GIT_AUTHOR_EMAIL", "ada@example.invalid")
        .env("GIT_COMMITTER_NAME", "Ada")
        .env("GIT_COMMITTER_EMAIL", "ada@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A repository on branch `main` with one commit, and a linked worktree
/// on `task` made from it, as New task makes it.
pub(super) fn repo_with_task() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("repo");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-q", "-b", "main"]);
    std::fs::write(source.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(source.join("b.txt"), "keep\n").unwrap();
    git(&source, &["add", "."]);
    git(&source, &["commit", "-q", "-m", "base"]);
    let task = dir.path().join("task");
    let head = git(&source, &["rev-parse", "HEAD"]);
    git(
        &source,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "task",
            "--",
            task.to_str().unwrap(),
            &head,
        ],
    );
    let (source, task) = (source.canonicalize().unwrap(), task.canonicalize().unwrap());
    (dir, source, task)
}

#[test]
fn task_base_names_the_source_branch_and_the_start_commit() {
    let (_dir, source, task) = repo_with_task();
    let base = task_base(&source, &task, "HEAD").unwrap();
    assert_eq!(base.name, "main");
    assert_eq!(base.commit, git(&source, &["rev-parse", "HEAD"]));
    assert_eq!(task_base(&source, &task, "v1").unwrap().name, "v1");
}

#[test]
fn changes_since_task_start_include_committed_uncommitted_and_new_files() {
    let (_dir, source, task) = repo_with_task();
    let base = git(&source, &["rev-parse", "HEAD"]);
    // The agent commits one change, leaves another, and adds a file.
    std::fs::write(task.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(&task, &["commit", "-q", "-am", "agent work"]);
    std::fs::write(task.join("b.txt"), "changed\n").unwrap();
    std::fs::write(task.join("new.txt"), "hi\n").unwrap();
    let files = list_changes(&task, &base).unwrap();
    let file = |name: &str| TaskFile {
        status: 'M',
        path: task.join(name),
        lines: None,
    };
    assert_eq!(
        files,
        vec![
            TaskFile {
                lines: Some((1, 0)),
                ..file("a.txt")
            },
            TaskFile {
                lines: Some((1, 1)),
                ..file("b.txt")
            },
            TaskFile {
                status: '?',
                ..file("new.txt")
            },
        ]
    );
    assert_eq!(totals(&files), (2, 1));
    // Nothing reached the source checkout or the index.
    assert_eq!(git(&source, &["status", "--porcelain"]), "");
}

#[test]
fn the_task_changes_page_loads_from_the_known_base_and_ignores_stale_answers() {
    use crate::{App, CenterPage, Job, Paths};
    let dir = tempfile::tempdir().unwrap();
    let ctx = eframe::egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let (jobs, requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    let project = terminator_core::Project {
        id: "w".into(),
        name: "w".into(),
        path: "/w".into(),
        layout: serde_json::Value::Null,
    };
    let mut state = app.state.clone();
    state.projects.push(project.clone());
    state.revision = state.revision.saturating_add(1);
    app.apply_state(state);
    // No base yet: the user is told, and nothing runs.
    app.open_task_changes(&project);
    assert!(app.info.is_some());
    assert!(
        !requests
            .try_iter()
            .any(|job| matches!(job, Job::TaskChanges(..)))
    );
    app.preferences.task_bases.insert(
        "/w".into(),
        crate::worktree_stats::TaskBase {
            name: "main".into(),
            commit: "abc123".into(),
        },
    );
    let git = crate::worktree_stats::WorktreeGit {
        base: Some("abc123".into()),
        ..Default::default()
    };
    app.worktree_stats.apply("/w".into(), Some(git));
    app.open_task_changes(&project);
    assert_eq!(app.center, CenterPage::TaskChanges);
    assert!(requests.try_iter().any(|job| matches!(
        job,
        Job::TaskChanges(path, base) if path == Path::new("/w") && base == "abc123"
    )));
    let page = app.task_changes.as_ref().unwrap();
    assert_eq!(page.label, "main");
    app.apply_task_changes(("/other".into(), "abc123".into()), Ok(vec![]));
    assert!(app.task_changes.as_ref().unwrap().files.is_none(), "stale");
    app.apply_task_changes(("/w".into(), "abc123".into()), Ok(vec![]));
    assert!(app.task_changes.as_ref().unwrap().files.is_some());
    app.hide_center_overlay();
    assert!(
        app.task_changes.is_none(),
        "leaving the page drops its data"
    );
}

#[test]
fn a_base_that_is_an_option_or_unknown_is_refused() {
    let (_dir, _source, task) = repo_with_task();
    assert!(list_changes(&task, "--output=/tmp/x").is_err());
    assert!(list_changes(&task, "no-such-ref").is_err());
}
