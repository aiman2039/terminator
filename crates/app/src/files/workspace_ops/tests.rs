use super::*;

pub(super) fn git(root: &Path, args: &[&str]) {
    run_command(git::command(root, args), git_options()).unwrap();
}

pub(super) fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "core.autocrlf", "false"]);
    git(dir.path(), &["config", "user.email", "ops@example.com"]);
    git(dir.path(), &["config", "user.name", "Ops"]);
    fs::write(dir.path().join("a.txt"), "base\n").unwrap();
    git(dir.path(), &["add", "a.txt"]);
    git(dir.path(), &["commit", "-qm", "base"]);
    dir
}

#[test]
fn new_file_with_a_nested_name_creates_its_folders() {
    let dir = repo();
    let root = dir.path();
    perform(root, Op::CreateFile(root.join("src/deep/mod.rs"))).unwrap();
    assert!(root.join("src/deep/mod.rs").is_file());
    perform(root, Op::CreateDir(root.join("docs/guide/img"))).unwrap();
    assert!(root.join("docs/guide/img").is_dir());
    assert!(perform(root, Op::CreateFile(root.join("x/../../escape.rs"))).is_err());
    assert!(!root.parent().unwrap().join("escape.rs").exists());
}

#[test]
fn head_message_returns_the_full_last_message() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.txt"), "next\n").unwrap();
    git(root, &["commit", "-qam", "subject\n\nbody line"]);
    let Ok(Report::HeadMessage(message)) = perform(root, Op::HeadMessage) else {
        panic!("expected the HEAD message");
    };
    assert_eq!(message, "subject\n\nbody line");
}

#[test]
fn folders_rename_duplicate_and_delete() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir_all(root.join("pkg/sub")).unwrap();
    fs::write(root.join("pkg/sub/x.rs"), "x").unwrap();
    perform(root, Op::Duplicate(root.join("pkg"))).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("pkg copy/sub/x.rs")).unwrap(),
        "x"
    );
    perform(
        root,
        Op::Rename {
            from: root.join("pkg copy"),
            to: root.join("lib"),
        },
    )
    .unwrap();
    assert!(root.join("lib/sub/x.rs").is_file());
    perform(root, Op::Delete(root.join("lib"))).unwrap();
    assert!(!root.join("lib").exists());
    // The root itself is never a delete target.
    assert!(perform(root, Op::Delete(root.to_path_buf())).is_err());
}

#[test]
fn move_and_copy_into_folders() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join("dest")).unwrap();
    fs::write(root.join("b.txt"), "b").unwrap();
    perform(
        root,
        Op::Move {
            from: vec![root.join("b.txt")],
            to_dir: root.join("dest"),
        },
    )
    .unwrap();
    assert!(root.join("dest/b.txt").is_file());
    assert!(!root.join("b.txt").exists());
    // A folder cannot move into its own subtree.
    assert!(
        perform(
            root,
            Op::Move {
                from: vec![root.join("dest")],
                to_dir: root.join("dest"),
            },
        )
        .is_err()
    );
    // Copies from outside the root (a Finder drop) get a free name.
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("b.txt"), "outside").unwrap();
    perform(
        root,
        Op::Copy {
            from: vec![outside.path().join("b.txt")],
            to_dir: root.join("dest"),
        },
    )
    .unwrap();
    assert_eq!(fs::read_to_string(root.join("dest/b.txt")).unwrap(), "b");
    assert_eq!(
        fs::read_to_string(root.join("dest/b copy.txt")).unwrap(),
        "outside"
    );
    // Destinations stay inside the root.
    assert!(
        perform(
            root,
            Op::Copy {
                from: vec![root.join("a.txt")],
                to_dir: outside.path().to_path_buf(),
            },
        )
        .is_err()
    );
}

#[test]
fn delete_leaves_the_index_alone() {
    let dir = repo();
    let root = dir.path();
    perform(root, Op::Delete(root.join("a.txt"))).unwrap();
    assert!(!root.join("a.txt").exists());
    let output = run_command(
        git::command(root, &["status", "--porcelain"]),
        git_options(),
    )
    .unwrap();
    // Unstaged deletion (` D`), not a staged `git rm` (`D `).
    assert_eq!(String::from_utf8(output.stdout).unwrap(), " D a.txt\n");
}

#[test]
fn stage_commit_and_discard_stay_inside_the_repo() {
    let dir = repo();
    let root = dir.path();
    let path = root.join("a.txt");
    fs::write(&path, "next\n").unwrap();
    perform(root, Op::Stage(path.clone())).unwrap();
    fs::write(root.join("new.txt"), "new\n").unwrap();
    perform(root, Op::Stage(root.join("new.txt"))).unwrap();
    git(root, &["commit", "-qm", "next"]);
    fs::write(&path, "dirty\n").unwrap();
    perform(
        root,
        Op::Discard {
            path: path.clone(),
            untracked: false,
        },
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "next\n");
    fs::write(root.join("scratch.txt"), "x").unwrap();
    perform(
        root,
        Op::Discard {
            path: root.join("scratch.txt"),
            untracked: true,
        },
    )
    .unwrap();
    assert!(!root.join("scratch.txt").exists());
    let outside = dir.path().join("../nope.txt");
    assert!(perform(root, Op::Stage(outside)).is_err());
}

#[test]
fn shelve_unshelve_and_drop_roundtrip() {
    let dir = repo();
    let root = dir.path();
    assert!(valid_shelf_ref("stash@{0}").is_ok());
    assert!(valid_shelf_ref("stash@{12}").is_ok());
    assert!(valid_shelf_ref("--all").is_err());
    assert!(valid_shelf_ref("stash@{x}").is_err());
    assert!(perform(root, Op::Shelve(vec![])).is_err());
    fs::write(root.join("a.txt"), "shelved\n").unwrap();
    perform(root, Op::Stage(root.join("a.txt"))).unwrap();
    perform(root, Op::Shelve(vec![root.join("a.txt")])).unwrap();
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "base\n");
    let Report::Shelves(list) = perform(root, Op::Shelves).unwrap() else {
        panic!("shelves report");
    };
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].0,
        git_text(root, &["rev-parse", "stash@{0}"]).unwrap().trim()
    );
    assert!(list[0].1.contains("Shelve (Terminator)"));
    assert!(perform(root, Op::Unshelve("--all".into())).is_err());
    perform(root, Op::Unshelve(list[0].0.clone())).unwrap();
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "shelved\n");
    let Report::Shelves(list) = perform(root, Op::Shelves).unwrap() else {
        panic!("shelves report");
    };
    assert!(list.is_empty());
    // Drop removes a shelf without touching the worktree.
    perform(root, Op::Stage(root.join("a.txt"))).unwrap();
    perform(root, Op::Shelve(vec![root.join("a.txt")])).unwrap();
    let Report::Shelves(list) = perform(root, Op::Shelves).unwrap() else {
        panic!("shelves report");
    };
    perform(root, Op::DropShelf(list[0].0.clone())).unwrap();
    let Report::Shelves(list) = perform(root, Op::Shelves).unwrap() else {
        panic!("shelves report");
    };
    assert!(list.is_empty());
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "base\n");
}

#[test]
fn drop_after_a_new_stash_elsewhere_drops_the_listed_shelf() {
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.txt"), "mine\n").unwrap();
    perform(root, Op::Stage(root.join("a.txt"))).unwrap();
    perform(root, Op::Shelve(vec![root.join("a.txt")])).unwrap();
    let Report::Shelves(listed) = perform(root, Op::Shelves).unwrap() else {
        panic!("shelves report");
    };
    // An agent stashes in its terminal: every position moves by one.
    fs::write(root.join("a.txt"), "agent\n").unwrap();
    git(root, &["stash", "push", "-q", "-m", "agent work"]);
    perform(root, Op::DropShelf(listed[0].0.clone())).unwrap();
    let left = git_text(root, &["stash", "list", "--format=%gs"]).unwrap();
    assert!(left.contains("agent work"), "{left}");
    assert!(!left.contains("Shelve (Terminator)"), "{left}");
    assert!(
        perform(root, Op::Unshelve(listed[0].0.clone())).is_err(),
        "a shelf that is gone is refused"
    );
}

#[test]
fn name_status_rename_uses_the_destination_path() {
    let root = Path::new("/repo");
    let parsed = parse_name_status(root, b"R100\0old.rs\0new.rs\0M\0keep.rs\0");
    assert_eq!(
        parsed,
        vec![(root.join("new.rs"), 'R'), (root.join("keep.rs"), 'M'),]
    );
}

#[test]
fn base_refs_reject_flags_ranges_and_missing_refs() {
    let dir = repo();
    let root = dir.path();
    assert!(valid_ref(root, "main"));
    assert!(!valid_ref(root, "--all"));
    assert!(!valid_ref(root, "a..b"));
    assert!(!valid_ref(root, "missing-ref"));
}

#[test]
fn compare_without_an_upstream_reports_root_only() {
    let dir = repo();
    let data = compare(dir.path(), None).unwrap();
    assert_eq!(data.root.as_deref(), Some(dir.path()));
    assert!(data.upstream.is_none());
    assert!(data.base.is_none());
    assert!(data.files.is_empty());
}

#[test]
fn switch_lists_branches_and_rejects_escape() {
    let dir = repo();
    let root = dir.path();
    git(root, &["branch", "feature"]);
    let branches = branches(root).unwrap();
    assert!(branches.iter().any(|name| name == "feature"));
    perform(root, Op::Switch("feature".into())).unwrap();
    assert!(perform(root, Op::Switch("--orphan".into())).is_err());
    let log = log(root).unwrap();
    assert_eq!(log[0].1, "base");
}

#[test]
fn discard_restores_the_index_not_head() {
    let dir = repo();
    let root = dir.path();
    let path = root.join("a.txt");
    fs::write(&path, "staged\n").unwrap();
    git(root, &["add", "a.txt"]);
    fs::write(&path, "dirty\n").unwrap();
    perform(
        root,
        Op::Discard {
            path: path.clone(),
            untracked: false,
        },
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "staged\n");
    let staged = git_text(root, &["diff", "--cached", "--", "a.txt"]).unwrap();
    assert!(staged.contains("+staged"), "staged hunk kept: {staged}");
}

#[test]
fn discard_restores_a_newly_staged_file_from_the_index() {
    let dir = repo();
    let root = dir.path();
    let path = root.join("new.txt");
    fs::write(&path, "v1\n").unwrap();
    git(root, &["add", "new.txt"]);
    fs::write(&path, "v2\n").unwrap();
    perform(
        root,
        Op::Discard {
            path: path.clone(),
            untracked: false,
        },
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "v1\n");
}

#[test]
#[cfg(unix)]
fn symlink_actions_affect_the_link_not_the_target() {
    let dir = repo();
    let root = dir.path();
    let target = root.join("target.txt");
    fs::write(&target, "data\n").unwrap();
    std::os::unix::fs::symlink("target.txt", root.join("link.txt")).unwrap();
    perform(root, Op::Stage(root.join("link.txt"))).unwrap();
    let files = git_text(root, &["ls-files"]).unwrap();
    assert!(
        files.lines().any(|line| line == "link.txt"),
        "staged the link: {files}"
    );
    perform(
        root,
        Op::Rename {
            from: root.join("link.txt"),
            to: root.join("moved.txt"),
        },
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "data\n");
    assert!(
        fs::symlink_metadata(root.join("moved.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    perform(root, Op::Duplicate(root.join("moved.txt"))).unwrap();
    let copy = root.join("moved copy.txt");
    assert!(
        fs::symlink_metadata(&copy)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    perform(root, Op::Delete(copy)).unwrap();
    perform(root, Op::Delete(root.join("moved.txt"))).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "data\n");
    std::os::unix::fs::symlink("missing.txt", root.join("broken.txt")).unwrap();
    perform(root, Op::Delete(root.join("broken.txt"))).unwrap();
    assert!(fs::symlink_metadata(root.join("broken.txt")).is_err());
}

#[test]
fn discard_of_a_bracket_name_restores_only_that_file() {
    let dir = repo();
    let root = dir.path();
    for name in ["[ab].txt", "b.txt"] {
        fs::write(root.join(name), "base\n").unwrap();
    }
    git(root, &["add", "--", "b.txt", ":(literal)[ab].txt"]);
    git(root, &["commit", "-qm", "names"]);
    for name in ["[ab].txt", "a.txt", "b.txt"] {
        fs::write(root.join(name), "dirty\n").unwrap();
    }
    perform(
        root,
        Op::Discard {
            path: root.join("[ab].txt"),
            untracked: false,
        },
    )
    .unwrap();
    assert_eq!(fs::read_to_string(root.join("[ab].txt")).unwrap(), "base\n");
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "dirty\n");
    assert_eq!(fs::read_to_string(root.join("b.txt")).unwrap(), "dirty\n");
    perform(root, Op::Stage(root.join("[ab].txt"))).unwrap();
    let staged = git_text(root, &["diff", "--cached", "--name-only"]).unwrap();
    assert_eq!(staged, "");
}

#[test]
fn stage_all_stages_every_path_in_one_operation() {
    let dir = repo();
    let root = dir.path();
    let mut paths = vec![root.join("a.txt")];
    fs::remove_file(root.join("a.txt")).unwrap();
    for index in 0..300 {
        let path = root.join(format!("new-{index}.txt"));
        fs::write(&path, "x\n").unwrap();
        paths.push(path);
    }
    // `*` is not a valid Windows file name.
    #[cfg(unix)]
    fs::write(root.join("*"), "star\n").unwrap();
    perform(root, Op::StageAll(paths)).unwrap();
    let staged = git_text(root, &["diff", "--cached", "--name-only"]).unwrap();
    assert_eq!(staged.lines().count(), 301);
    assert!(staged.lines().any(|line| line == "a.txt"));
    assert!(!staged.lines().any(|line| line == "*"));
}

#[test]
fn files_of_a_deleted_folder_can_be_staged_and_restored() {
    let dir = repo();
    let root = dir.path();
    fs::create_dir(root.join("old")).unwrap();
    fs::write(root.join("old/a.rs"), "a\n").unwrap();
    fs::write(root.join("old/b.rs"), "b\n").unwrap();
    git(root, &["add", "old"]);
    git(root, &["commit", "-qm", "old"]);
    fs::remove_dir_all(root.join("old")).unwrap();
    perform(
        root,
        Op::Discard {
            path: root.join("old/a.rs"),
            untracked: false,
        },
    )
    .unwrap();
    assert_eq!(fs::read_to_string(root.join("old/a.rs")).unwrap(), "a\n");
    fs::remove_dir_all(root.join("old")).unwrap();
    perform(
        root,
        Op::StageAll(vec![root.join("old/a.rs"), root.join("old/b.rs")]),
    )
    .unwrap();
    let staged = git_text(root, &["diff", "--cached", "--name-only"]).unwrap();
    assert_eq!(staged.lines().collect::<Vec<_>>(), ["old/a.rs", "old/b.rs"]);
    assert!(perform(root, Op::Stage(root.join("gone/../../escape.rs"))).is_err());
}

#[cfg(unix)]
#[test]
fn create_file_never_truncates_through_an_existing_link() {
    let dir = repo();
    let root = dir.path();
    std::os::unix::fs::symlink(root.join("a.txt"), root.join("link.txt")).unwrap();
    assert!(perform(root, Op::CreateFile(root.join("link.txt"))).is_err());
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "base\n");
}
