//! Worktree hand-back and cleanup against real `git` in temporary
//! repositories; no real remote or user configuration.
use super::apply::{SOURCE_DIRTY, apply};
use super::probe::{disk_size, main_checkout, probe};
use super::*;
use std::fs;

fn git(cwd: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["-c", "user.email=t@example.invalid", "-c", "user.name=T"])
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn head(cwd: &Path) -> String {
    probe::git_out(cwd, &["rev-parse", "HEAD"]).unwrap()
}

/// A main checkout on `main` and a task worktree on `task` with one commit.
fn task_repo() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let (main, task) = (base.join("main"), base.join("task"));
    fs::create_dir(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    // Apply to source runs plain `git`: local config wins over a global one.
    for (key, value) in [
        ("user.email", "t@example.invalid"),
        ("user.name", "T"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", "/dev/null"),
    ] {
        git(&main, &["config", key, value]);
    }
    fs::write(main.join("a.txt"), "base\n").unwrap();
    git(&main, &["add", "a.txt"]);
    git(&main, &["commit", "-qm", "base"]);
    let start = head(&main);
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "task",
            &task.to_string_lossy(),
        ],
    );
    fs::write(task.join("b.txt"), "task\n").unwrap();
    git(&task, &["add", "b.txt"]);
    git(&task, &["commit", "-qm", "task work"]);
    (dir, main, task, start)
}

fn request(mode: ApplyMode, main: &Path, task: &Path, base: Option<String>) -> ApplyRequest {
    ApplyRequest {
        mode,
        worktree: task.to_path_buf(),
        source: main.to_path_buf(),
        base,
    }
}

#[test]
fn the_main_checkout_is_the_first_worktree_record() {
    let porcelain = b"worktree /r\0HEAD abc\0branch refs/heads/main\0\0worktree /w\0HEAD d\0\0";
    assert_eq!(
        main_checkout(porcelain),
        Some((PathBuf::from("/r"), Some("main".into())))
    );
    let detached = b"worktree /r\0HEAD abc\0detached\0\0";
    assert_eq!(main_checkout(detached), Some((PathBuf::from("/r"), None)));
}

#[test]
fn probe_reads_source_merged_mark_last_commit_and_size() {
    let (_dir, main, task, _) = task_repo();
    let found = probe(&task);
    assert_eq!(found.source.as_deref(), Some(main.as_path()));
    assert_eq!(found.source_branch.as_deref(), Some("main"));
    assert_eq!(found.merged, Some(false), "main lacks the task commit");
    assert!(found.last_commit.is_some_and(|time| time > 0));
    assert!(found.size.is_some_and(|(bytes, cut)| bytes > 0 && !cut));
    git(&main, &["merge", "-q", "--no-edit", "refs/heads/task"]);
    assert_eq!(probe(&task).merged, Some(true));
}

#[test]
fn merge_brings_the_task_branch_into_a_clean_source() {
    let (_dir, main, task, _) = task_repo();
    let message = apply(&request(ApplyMode::Merge, &main, &task, None)).unwrap();
    assert_eq!(message, "Merged task into the source checkout");
    assert!(main.join("b.txt").is_file());
    assert_eq!(probe(&task).merged, Some(true));
}

#[test]
fn a_dirty_source_is_refused_before_git_runs() {
    let (_dir, main, task, _) = task_repo();
    fs::write(main.join("a.txt"), "edited\n").unwrap();
    let before = head(&main);
    let error = apply(&request(ApplyMode::Merge, &main, &task, None)).unwrap_err();
    assert_eq!(error, SOURCE_DIRTY);
    assert_eq!(head(&main), before);
    assert_eq!(fs::read_to_string(main.join("a.txt")).unwrap(), "edited\n");
}

#[test]
fn cherry_pick_takes_the_commits_since_the_task_base() {
    let (_dir, main, task, start) = task_repo();
    fs::write(task.join("c.txt"), "more\n").unwrap();
    git(&task, &["add", "c.txt"]);
    git(&task, &["commit", "-qm", "more work"]);
    let picked = request(ApplyMode::CherryPick, &main, &task, Some(start));
    assert_eq!(
        apply(&picked).unwrap(),
        "Cherry-picked 2 commits into the source checkout"
    );
    assert!(main.join("b.txt").is_file() && main.join("c.txt").is_file());
    // Without a saved base, the merge base counts; nothing is left.
    let again = request(ApplyMode::CherryPick, &main, &task, None);
    assert!(apply(&again).is_err());
}

#[test]
fn a_conflict_is_left_for_the_terminal_with_a_clear_message() {
    let (_dir, main, task, _) = task_repo();
    fs::write(task.join("a.txt"), "task side\n").unwrap();
    git(&task, &["commit", "-qam", "task edit"]);
    fs::write(main.join("a.txt"), "main side\n").unwrap();
    git(&main, &["commit", "-qam", "main edit"]);
    let error = apply(&request(ApplyMode::Merge, &main, &task, None)).unwrap_err();
    assert!(error.contains("stopped on conflicts"), "{error}");
    assert!(error.contains("git merge --continue"), "{error}");
    assert!(probe::git_out(&main, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_some());
}

#[test]
fn disk_size_counts_files_and_never_follows_links() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("sub/a"), vec![0u8; 100]).unwrap();
    fs::write(dir.path().join("b"), vec![0u8; 20]).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/", dir.path().join("root-link")).unwrap();
    assert_eq!(disk_size(dir.path()), (120, false));
}

fn app(dir: &tempfile::TempDir) -> App {
    let ctx = egui::Context::default();
    App::with_context(&ctx, terminator_core::Paths::at(dir.path().join("data")))
}

/// `app` knows the task worktree `project` at `task`, made from `main`.
fn register(app: &mut App, main: &Path, task: &Path) {
    app.state.projects.push(terminator_core::Project {
        id: "task".into(),
        name: "task".into(),
        path: task.to_path_buf(),
        layout: serde_json::Value::Null,
    });
    app.state
        .worktrees
        .push(terminator_core::worktrees::Registration {
            project_id: "task".into(),
            common_dir: main.join(".git"),
            path: task.to_path_buf(),
            created: 0,
            removed: false,
        });
    app.git_extras
        .cleanup
        .probes
        .insert(task.to_path_buf(), probe(task));
}

#[test]
fn apply_to_source_waits_for_a_commit_running_in_the_source() {
    let (dir, main, task, _) = task_repo();
    let mut app = app(&dir);
    register(&mut app, &main, &task);
    app.git
        .start_commit(main.clone(), None, "msg".into())
        .unwrap();
    assert!(app.apply_blocker(&task).is_some());
    app.start_apply("task", ApplyMode::Merge);
    assert!(app.git_extras.cleanup.applying.is_none(), "no job is sent");
    assert!(!main.join("b.txt").exists());
    // The push stage still holds HEAD writers.
    assert!(app.git.commit_pushing());
    app.start_apply("task", ApplyMode::Merge);
    assert!(app.git_extras.cleanup.applying.is_none());
    app.git.finish_commit(&Ok("Committed and pushed".into()));
    assert_eq!(app.apply_blocker(&task), None);
}

#[test]
fn a_removed_worktree_forgets_its_task_base() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(&dir);
    let (gone, kept) = (PathBuf::from("/w/gone"), PathBuf::from("/w/kept"));
    for path in [&gone, &kept] {
        app.preferences
            .task_bases
            .insert(path.clone(), crate::worktree_stats::TaskBase::default());
        app.state
            .worktrees
            .push(terminator_core::worktrees::Registration {
                project_id: path.display().to_string(),
                common_dir: "/w/.git".into(),
                path: path.clone(),
                created: 0,
                removed: path == &gone,
            });
    }
    app.prune_task_bases();
    assert!(!app.preferences.task_bases.contains_key(&gone));
    assert!(app.preferences.task_bases.contains_key(&kept));
}

#[test]
fn remove_selected_asks_the_daemon_for_each_and_clears_the_ticks() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app(&dir);
    let (jobs, requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    app.git_extras.cleanup.selected = ["a".to_owned(), "b".to_owned()].into();
    app.remove_worktrees(&["a".to_owned(), "b".to_owned()]);
    let removed: Vec<String> = requests
        .try_iter()
        .filter_map(|job| match job {
            Job::Control(request, _) => match *request {
                Request::WorktreeRemove { project } => Some(project),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(removed, ["a", "b"]);
    assert!(app.git_extras.cleanup.selected.is_empty());
}

#[test]
fn the_worktrees_page_shows_each_task_with_its_facts_and_actions() {
    let (dir, main, task, _) = task_repo();
    let mut app = app(&dir);
    register(&mut app, &main, &task);
    git(&main, &["merge", "-q", "--no-edit", "refs/heads/task"]);
    app.git_extras
        .cleanup
        .probes
        .insert(task.clone(), probe(&task));
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(900.0, 700.0),
        )),
        ..Default::default()
    };
    let mut output = ctx.run_ui(raw, |ui| app.worktree_management_center(ui));
    output.textures_delta.clear();
    let texts: Vec<String> = crate::app::painted_text(&output.shapes)
        .into_iter()
        .map(|(_, text)| text)
        .collect();
    for wanted in [
        "Worktrees",
        "task",
        "Merged",
        "Apply to source",
        "Remove selected (0)…",
    ] {
        assert!(
            texts.iter().any(|text| text == wanted),
            "{wanted}: {texts:?}"
        );
    }
    assert!(
        texts
            .iter()
            .any(|text| text.contains("last commit") && text.contains("0 live sessions")),
        "{texts:?}"
    );
}
