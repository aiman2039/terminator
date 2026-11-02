use super::*;
fn git(root: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    );
}
#[test]
fn add_list_and_remove_preserve_dirty_checkouts_and_branch_refs() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "Fixture"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(repo.join("file"), "base").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    // std canonicalization supplies a verbatim path on Windows. Git must
    // receive a compatible path even when callers supply that form.
    let common = common_dir(&repo).unwrap().canonicalize().unwrap();
    let path = dunce::canonicalize(dir.path())
        .unwrap()
        .join("space checkout");
    let requested = dir.path().canonicalize().unwrap().join("space checkout");
    let record = add(AddInput {
        common: &common,
        path: &requested,
        branch: Some("task-fixture"),
        start: "HEAD",
    })
    .unwrap();
    assert_eq!(record.path, path);
    assert_eq!(record.branch.as_deref(), Some("task-fixture"));
    assert_eq!(list(&common).unwrap().len(), 2);
    std::fs::write(path.join("untracked"), "keep").unwrap();
    assert!(remove(&common, &requested).is_err());
    assert!(path.join("untracked").exists());
    std::fs::remove_file(path.join("untracked")).unwrap();
    remove(&common, &requested).unwrap();
    assert!(!path.exists());
    git(&repo, &["show-ref", "--verify", "refs/heads/task-fixture"]);
    assert!(
        add(AddInput {
            common: &common,
            path: &repo,
            branch: Some("other"),
            start: "HEAD"
        })
        .is_err()
    );
    assert!(
        add(AddInput {
            common: &common,
            path: &path,
            branch: Some("--bad"),
            start: "HEAD"
        })
        .is_err()
    );
}
#[test]
fn locked_clean_checkout_is_preserved_until_unlocked() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "Fixture"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(repo.join("file"), "base").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    // std canonicalization supplies a verbatim path on Windows. Git must
    // receive a compatible path even when callers supply that form.
    let common = common_dir(&repo).unwrap().canonicalize().unwrap();
    let path = dunce::canonicalize(dir.path())
        .unwrap()
        .join("locked checkout");
    add(AddInput {
        common: &common,
        path: &path,
        branch: Some("locked-fixture"),
        start: "HEAD",
    })
    .unwrap();
    let head = resolve_start(&repo, "refs/heads/locked-fixture").unwrap();
    git(
        &repo,
        &[
            "worktree",
            "lock",
            "--reason",
            "fixture lock",
            path.to_str().unwrap(),
        ],
    );

    let error = remove(&common, &path).unwrap_err();
    assert!(error.to_string().contains("locked"), "{error:#}");
    assert_eq!(std::fs::read(path.join("file")).unwrap(), b"base");
    let records = list(&common).unwrap();
    assert_eq!(records.len(), 2);
    let retained = records.iter().find(|w| w.path == path).unwrap();
    assert!(retained.locked);
    assert_eq!(retained.branch.as_deref(), Some("locked-fixture"));
    assert_eq!(
        resolve_start(&repo, "refs/heads/locked-fixture").unwrap(),
        head
    );

    git(&repo, &["worktree", "unlock", path.to_str().unwrap()]);
    remove(&common, &path).unwrap();
    assert!(!path.exists());
    assert!(!list(&common).unwrap().iter().any(|w| w.path == path));
    assert_eq!(
        resolve_start(&repo, "refs/heads/locked-fixture").unwrap(),
        head
    );
}
#[test]
fn porcelain_records_keep_spaces_and_lock_metadata() {
    let records=parse(b"worktree /a path\0HEAD 123\0branch refs/heads/topic\0locked reason\0\0worktree /bare\0bare\0\0").unwrap();
    assert_eq!(records[0].path, PathBuf::from("/a path"));
    assert!(records[0].locked);
    assert!(records[1].bare);
}

#[test]
fn remove_refuses_a_checkout_with_ignored_files() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "Fixture"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(repo.join(".gitignore"), ".env\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    let common = common_dir(&repo).unwrap().canonicalize().unwrap();
    let path = dunce::canonicalize(dir.path()).unwrap().join("ignored");
    add(AddInput {
        common: &common,
        path: &path,
        branch: None,
        start: "HEAD",
    })
    .unwrap();
    std::fs::write(path.join(".env"), "SECRET=1").unwrap();
    let error = remove(&common, &path).unwrap_err();
    assert!(error.to_string().contains(".env"), "{error:#}");
    assert_eq!(std::fs::read(path.join(".env")).unwrap(), b"SECRET=1");
    std::fs::remove_file(path.join(".env")).unwrap();
    remove(&common, &path).unwrap();
    assert!(!path.exists());
}

#[test]
fn remove_succeeds_when_checkout_folder_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "Fixture"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(repo.join("file"), "base").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "base"]);
    let common = common_dir(&repo).unwrap().canonicalize().unwrap();
    let path = dunce::canonicalize(dir.path()).unwrap().join("gone");
    add(AddInput {
        common: &common,
        path: &path,
        branch: Some("gone-fixture"),
        start: "HEAD",
    })
    .unwrap();
    std::fs::remove_dir_all(&path).unwrap();
    ensure_unused(&path, "other-project", &[]).unwrap();
    remove(&common, &path).unwrap();
    assert!(!list(&common).unwrap().iter().any(|w| w.path == path));
    git(&repo, &["show-ref", "--verify", "refs/heads/gone-fixture"]);
}
