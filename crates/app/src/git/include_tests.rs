//! `.worktreeinclude` copies against real `git` in temporary repositories.
use super::task_tests::{git, repo_with_task};
use super::worktree_include::{CopyReport, copy_file, copy_included};
use std::path::Path;

#[test]
fn listed_ignored_files_are_copied_and_nothing_else() {
    let (_dir, source, task) = repo_with_task();
    std::fs::write(source.join(".gitignore"), ".env\nsecrets/\nnode_modules/\n").unwrap();
    std::fs::write(source.join(".worktreeinclude"), ".env\nsecrets/\n").unwrap();
    std::fs::write(source.join(".env"), "KEY=1\n").unwrap();
    std::fs::create_dir_all(source.join("secrets/deep")).unwrap();
    std::fs::write(source.join("secrets/deep/key.pem"), "pem\n").unwrap();
    std::fs::create_dir(source.join("node_modules")).unwrap();
    std::fs::write(source.join("node_modules/x.js"), "x\n").unwrap();
    // Untracked but not ignored: uncommitted work, never copied.
    std::fs::write(source.join("draft.txt"), "draft\n").unwrap();
    let report = copy_included(&source, &task);
    assert_eq!(report.copied, 2, "{report:?}");
    assert_eq!(
        std::fs::read_to_string(task.join(".env")).unwrap(),
        "KEY=1\n"
    );
    assert!(task.join("secrets/deep/key.pem").is_file());
    assert!(!task.join("node_modules").exists());
    assert!(!task.join("draft.txt").exists());
    assert!(report.summary().unwrap().contains("copied 2"));
    // The index and the tracked files are untouched.
    assert_eq!(git(&task, &["diff", "--name-only", "HEAD"]), "");
}

#[test]
fn a_pattern_for_one_child_of_an_ignored_folder_copies_only_that_child() {
    let (_dir, source, task) = repo_with_task();
    std::fs::write(source.join(".gitignore"), "node_modules/\nbuild/\n").unwrap();
    std::fs::write(
        source.join(".worktreeinclude"),
        "node_modules/.cache/keep.json\nbuild/config/*.env\n",
    )
    .unwrap();
    for (path, text) in [
        ("node_modules/.cache/keep.json", "keep\n"),
        ("node_modules/.cache/other.json", "other\n"),
        ("node_modules/pkg/index.js", "js\n"),
        ("build/config/app.env", "KEY=1\n"),
        ("build/config/app.txt", "txt\n"),
        ("build/out.bin", "bin\n"),
    ] {
        let file = source.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    let report = copy_included(&source, &task);
    assert_eq!(report.copied, 2, "{report:?}");
    assert!(report.errors.is_empty(), "{report:?}");
    assert_eq!(
        std::fs::read_to_string(task.join("node_modules/.cache/keep.json")).unwrap(),
        "keep\n"
    );
    assert!(task.join("build/config/app.env").is_file());
    assert!(!task.join("node_modules/.cache/other.json").exists());
    assert!(!task.join("node_modules/pkg").exists());
    assert!(!task.join("build/config/app.txt").exists());
    assert!(!task.join("build/out.bin").exists());
}

#[test]
fn an_included_folder_keeps_its_excluded_child_behind() {
    let (_dir, source, task) = repo_with_task();
    std::fs::write(source.join(".gitignore"), "secrets/\n").unwrap();
    std::fs::write(
        source.join(".worktreeinclude"),
        "secrets/\n!secrets/private.key\n",
    )
    .unwrap();
    std::fs::create_dir_all(source.join("secrets/deep")).unwrap();
    std::fs::write(source.join("secrets/public.pem"), "pub\n").unwrap();
    std::fs::write(source.join("secrets/private.key"), "private\n").unwrap();
    std::fs::write(source.join("secrets/deep/token"), "token\n").unwrap();
    let report = copy_included(&source, &task);
    assert_eq!(report.copied, 2, "{report:?}");
    assert!(task.join("secrets/public.pem").is_file());
    assert!(task.join("secrets/deep/token").is_file());
    assert!(!task.join("secrets/private.key").exists());
}

#[test]
fn an_existing_file_is_kept_and_links_are_not_followed() {
    let (_dir, source, task) = repo_with_task();
    std::fs::write(source.join(".gitignore"), ".env*\n").unwrap();
    std::fs::write(source.join(".worktreeinclude"), ".env*\n").unwrap();
    std::fs::write(source.join(".env"), "source\n").unwrap();
    std::fs::write(task.join(".env"), "mine\n").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), "outside\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path().join("secret"), source.join(".env.link")).unwrap();
    let report = copy_included(&source, &task);
    assert_eq!(report.copied, 0, "{report:?}");
    assert_eq!(
        std::fs::read_to_string(task.join(".env")).unwrap(),
        "mine\n"
    );
    assert!(!task.join(".env.link").exists());
}

#[test]
fn an_existing_destination_file_is_kept_and_no_staged_file_is_left() {
    let source = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let dest = std::fs::canonicalize(dest.path()).unwrap();
    let from = source.path().join(".env");
    std::fs::write(&from, "source\n").unwrap();
    std::fs::create_dir(dest.join("cfg")).unwrap();
    std::fs::write(dest.join("cfg/.env"), "mine\n").unwrap();
    let mut report = CopyReport::default();
    copy_file(&from, (&dest, Path::new("cfg/.env")), &mut report);
    assert_eq!((report.copied, report.kept), (0, 1), "{report:?}");
    assert_eq!(
        std::fs::read_to_string(dest.join("cfg/.env")).unwrap(),
        "mine\n"
    );
    assert_eq!(std::fs::read_dir(dest.join("cfg")).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn a_failed_copy_leaves_the_destination_free_for_a_retry() {
    let source = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let dest = std::fs::canonicalize(dest.path()).unwrap();
    // A folder opens like a file on unix, but reading it fails mid copy.
    let broken = source.path().join("folder");
    std::fs::create_dir(&broken).unwrap();
    let mut report = CopyReport::default();
    copy_file(&broken, (&dest, Path::new("cfg/.env")), &mut report);
    assert_eq!(report.errors.len(), 1, "{report:?}");
    assert!(!dest.join("cfg/.env").exists());
    assert_eq!(
        std::fs::read_dir(dest.join("cfg")).unwrap().count(),
        0,
        "no staged file is left"
    );
    let from = source.path().join(".env");
    std::fs::write(&from, "KEY=1\n").unwrap();
    let mut retry = CopyReport::default();
    copy_file(&from, (&dest, Path::new("cfg/.env")), &mut retry);
    assert_eq!(retry.copied, 1, "{retry:?}");
    assert_eq!(
        std::fs::read_to_string(dest.join("cfg/.env")).unwrap(),
        "KEY=1\n"
    );
}

#[cfg(unix)]
#[test]
fn a_linked_folder_in_the_worktree_gets_no_new_folders_outside() {
    let (_dir, source, task) = repo_with_task();
    std::fs::write(source.join(".gitignore"), "cache/\n").unwrap();
    std::fs::write(source.join(".worktreeinclude"), "cache/\n").unwrap();
    std::fs::create_dir_all(source.join("cache/deep")).unwrap();
    std::fs::write(source.join("cache/deep/key"), "k\n").unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), task.join("cache")).unwrap();
    let report = copy_included(&source, &task);
    assert_eq!(report.copied, 0, "{report:?}");
    assert!(
        !outside.path().join("deep").exists(),
        "no folder created through the link"
    );
}

#[test]
fn the_setup_command_is_typed_before_the_agent_on_one_line_without_enter() {
    use crate::worktree_ui::TaskStart;
    let start = |setup: &str, agent: &str| TaskStart {
        setup_command: setup.into(),
        agent_command: agent.into(),
        ..TaskStart::default()
    };
    assert_eq!(
        start("npm install", "claude").typed_input(),
        "npm install && claude"
    );
    assert_eq!(start("", "claude").typed_input(), "claude");
    assert_eq!(start("make\nsetup", "").typed_input(), "make setup");
    let text = start("a\r\nb", "codex").typed_input();
    let bytes = egui_term::paste_input(&text, egui_term::TerminalMode::empty());
    assert!(!bytes.contains(&b'\r') && !bytes.contains(&b'\n'));
}

#[test]
fn no_include_file_copies_nothing() {
    let (_dir, source, task) = repo_with_task();
    std::fs::write(source.join(".gitignore"), ".env\n").unwrap();
    std::fs::write(source.join(".env"), "KEY=1\n").unwrap();
    assert_eq!(copy_included(&source, &task), CopyReport::default());
    assert!(!task.join(".env").exists());
    assert!(CopyReport::default().summary().is_none());
}
