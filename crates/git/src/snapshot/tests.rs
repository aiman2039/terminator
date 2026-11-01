use super::*;
use std::process::Command;

fn options() -> CommandOptions {
    CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        ..Default::default()
    }
}

fn git_cmd(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git_cmd(dir.path(), &["init", "-q"]);
    git_cmd(
        dir.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    git_cmd(dir.path(), &["config", "user.name", "Fixture"]);
    dir
}

#[test]
fn partially_staged_snapshots_show_the_correct_side() {
    let dir = repo();
    let root = dir.path();
    let name = Path::new("space file.txt");
    std::fs::write(root.join(name), "base\n").unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "fixture"]);
    std::fs::write(root.join(name), "staged\n").unwrap();
    git_cmd(root, &["add", "."]);
    std::fs::write(root.join(name), "working\n").unwrap();
    let options = options();
    let (left, right) = snapshots(SnapshotsInput {
        root,
        path: name,
        staged: true,
        options: &options,
    })
    .unwrap();
    assert_eq!(left, b"base\n");
    assert_eq!(right, b"staged\n");
    let (left, right) = snapshots(SnapshotsInput {
        root,
        path: name,
        staged: false,
        options: &options,
    })
    .unwrap();
    assert_eq!(left, b"staged\n");
    assert_eq!(right, b"working\n");
}

#[test]
fn native_and_canonical_roots_resolve_existing_and_deleted_files() {
    let dir = repo();
    let root = dir.path();
    std::fs::write(root.join("present.rs"), "old\n").unwrap();
    let canonical = root.canonicalize().unwrap();
    for root in [root, canonical.as_path()] {
        for name in ["present.rs", "deleted.rs"] {
            for path in [dir.path().join(name), canonical.join(name)] {
                assert_eq!(relative_path(root, &path).unwrap(), PathBuf::from(name));
            }
        }
    }
}

#[test]
#[cfg(unix)]
fn symlink_reviews_reject_the_link_instead_of_reviewing_its_target() {
    let dir = repo();
    let root = dir.path();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(root.join("target.rs"), "old\n").unwrap();
    std::fs::write(outside.path().join("target.rs"), "outside\n").unwrap();
    std::os::unix::fs::symlink("target.rs", root.join("inside.rs")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("target.rs"), root.join("outside.rs")).unwrap();
    std::os::unix::fs::symlink("missing.rs", root.join("dangling.rs")).unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "base"]);
    for name in ["inside.rs", "outside.rs", "dangling.rs"] {
        for path in [PathBuf::from(name), root.join(name)] {
            let error = relative_path(root, &path).unwrap_err();
            assert!(
                error.to_string().contains("symlinks"),
                "{path:?}: {error:#}"
            );
        }
    }
}

#[test]
#[cfg(unix)]
fn directory_aliases_and_deleted_files_keep_their_git_identity() {
    let dir = repo();
    let root = dir.path();
    std::fs::write(root.join("file.rs"), "old\n").unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "base"]);
    let alias_dir = tempfile::tempdir().unwrap();
    let alias = alias_dir.path().join("repo");
    std::os::unix::fs::symlink(root, &alias).unwrap();
    std::fs::remove_file(root.join("file.rs")).unwrap();
    let relative = relative_path(&alias, &alias.join("file.rs")).unwrap();
    assert_eq!(relative, PathBuf::from("file.rs"));
    let options = options();
    let (left, right) = snapshots(SnapshotsInput {
        root,
        path: &relative,
        staged: false,
        options: &options,
    })
    .unwrap();
    assert_eq!(left, b"old\n");
    assert!(right.is_empty());
}

#[test]
fn untracked_and_binary_and_outside_paths_are_explicit() {
    let dir = repo();
    let root = dir.path();
    let options = options();
    std::fs::write(root.join("new.rs"), "fn new() {}\n").unwrap();
    let (left, right) = snapshots(SnapshotsInput {
        root,
        path: Path::new("new.rs"),
        staged: false,
        options: &options,
    })
    .unwrap();
    assert!(left.is_empty());
    assert!(check_text(&left) && check_text(&right));
    std::fs::write(root.join("new.rs"), b"binary\0").unwrap();
    let (_, right) = snapshots(SnapshotsInput {
        root,
        path: Path::new("new.rs"),
        staged: false,
        options: &options,
    })
    .unwrap();
    assert!(!check_text(&right));
    assert!(
        snapshots(SnapshotsInput {
            root,
            path: Path::new("../outside.rs"),
            staged: false,
            options: &options
        })
        .is_err()
    );
}

#[test]
fn untracked_bracket_name_is_not_hidden_by_a_tracked_glob_match() {
    let dir = repo();
    let root = dir.path();
    std::fs::write(root.join("a.txt"), "tracked\n").unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "base"]);
    std::fs::write(root.join("[ab].txt"), "new\n").unwrap();
    let options = options();
    let (left, right) = snapshots(SnapshotsInput {
        root,
        path: Path::new("[ab].txt"),
        staged: false,
        options: &options,
    })
    .unwrap();
    assert!(left.is_empty());
    assert_eq!(right, b"new\n");
}

#[test]
fn single_file_review_works_when_the_full_diff_is_too_large() {
    let dir = repo();
    let root = dir.path();
    std::fs::write(root.join("file.rs"), "old\n").unwrap();
    for index in 0..200 {
        std::fs::write(root.join(format!("bulk-{index}.txt")), "a\n").unwrap();
    }
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "base"]);
    for index in 0..200 {
        std::fs::write(root.join(format!("bulk-{index}.txt")), "b\n").unwrap();
    }
    std::fs::write(root.join("file.rs"), "new\n").unwrap();
    // The full raw diff is far above this limit; the scoped one is not.
    let options = CommandOptions {
        stdout_limit: 1024,
        ..Default::default()
    };
    let (left, right) = snapshots(SnapshotsInput {
        root,
        path: Path::new("file.rs"),
        staged: false,
        options: &options,
    })
    .unwrap();
    assert_eq!(left, b"old\n");
    assert_eq!(right, b"new\n");
}

#[test]
fn staged_rename_target_still_shows_the_rename_source() {
    let dir = repo();
    let root = dir.path();
    std::fs::write(root.join("old.rs"), "fn body() {}\n// same\n").unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "base"]);
    git_cmd(root, &["mv", "old.rs", "new.rs"]);
    let options = options();
    let (left, right) = snapshots(SnapshotsInput {
        root,
        path: Path::new("new.rs"),
        staged: true,
        options: &options,
    })
    .unwrap();
    assert_eq!(left, right);
    assert!(!left.is_empty());
}
