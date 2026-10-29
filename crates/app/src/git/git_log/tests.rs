use super::*;
use std::process::Command;

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
    git_cmd(dir.path(), &["init", "-q", "-b", "main"]);
    git_cmd(
        dir.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    git_cmd(dir.path(), &["config", "user.name", "Fixture"]);
    dir
}

#[test]
fn fetch_log_reads_commits_refs_and_the_active_branch() {
    let dir = repo();
    let root = dir.path();
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "first commit"]);
    git_cmd(root, &["branch", "feature"]);
    git_cmd(root, &["tag", "v1"]);
    std::fs::write(root.join("a.txt"), "two\n").unwrap();
    git_cmd(root, &["commit", "-qam", "second commit"]);

    let log = fetch_log(root).expect("log should load");
    assert!(log.error.is_none());
    assert_eq!(log.active_branch, "main");
    assert!(log.commits.iter().any(|c| c.subject == "second commit"));
    assert!(log.commits.iter().any(|c| c.subject == "first commit"));
    assert!(log.commits.iter().all(|c| !c.short_hash.is_empty()));
    assert!(log.commits.iter().all(|c| c.author == "Fixture"));
    assert!(
        log.refs.iter().any(|r| r.name == "feature"),
        "branch ref missing: {:?}",
        log.refs
    );
    assert!(
        log.refs
            .iter()
            .any(|r| r.name == "v1" && r.kind == RefKind::Tag),
        "tag ref missing: {:?}",
        log.refs
    );
}

#[test]
fn fetch_log_reports_a_directory_outside_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    let log = fetch_log(dir.path());
    assert!(log.is_err(), "non-repository should error: {log:?}");
}

#[test]
fn parse_log_keeps_multi_line_messages_parents_and_refs() {
    let hash = "1111111111111111111111111111111111111111";
    let parent = "2222222222222222222222222222222222222222";
    let text = format!(
        "{hash}\n1111111\nJane\njane@example.invalid\n1700000000\n{parent}\nHEAD -> main, tag: v1\nsubject line\n\nbody line one\nbody line two\n>>>>>END<<<<<\n"
    );
    let commits = parse_log(&text);
    assert_eq!(commits.len(), 1);
    let commit = &commits[0];
    assert_eq!(commit.hash, hash);
    assert_eq!(commit.short_hash, "1111111");
    assert_eq!(commit.author, "Jane");
    assert_eq!(commit.subject, "subject line");
    assert_eq!(
        commit.message,
        "subject line\n\nbody line one\nbody line two"
    );
    assert_eq!(commit.parents, vec![parent.to_string()]);
    assert_eq!(commit.refs, vec!["HEAD -> main", "tag: v1"]);
    assert_eq!(commit.timestamp, 1_700_000_000);
}

#[test]
fn parse_log_ignores_empty_entries() {
    assert!(parse_log("").is_empty());
    assert!(parse_log(">>>>>END<<<<<\n").is_empty());
}

#[test]
fn fetch_blame_annotates_every_line() {
    let dir = repo();
    let root = dir.path();
    std::fs::write(root.join("blame.txt"), "alpha\nbeta\n").unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "base"]);

    let blame = fetch_blame(root, Path::new("blame.txt")).expect("blame should load");
    assert!(blame.error.is_none());
    assert_eq!(blame.entries.len(), 2);
    assert_eq!(blame.entries[0].line, 0);
    assert_eq!(blame.entries[1].line, 1);
    assert!(blame.entries.iter().all(|e| e.author == "Fixture"));
    assert!(blame.entries[0].code.contains("alpha"));
    assert!(blame.entries[1].code.contains("beta"));
    // Every entry carries the real commit, its time, and its subject.
    let head = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert!(blame.entries.iter().all(|e| e.hash == head.trim()));
    assert!(
        blame
            .entries
            .iter()
            .all(|e| e.summary == "base" && e.time > 0)
    );
    // Blaming unsaved text marks the edited line as uncommitted.
    let edited = blame_with_contents(root, Path::new("blame.txt"), Some("alpha\nBETA\nnew\n"))
        .expect("contents blame");
    let flags: Vec<bool> = edited.entries.iter().map(BlameEntry::uncommitted).collect();
    assert_eq!(flags, [false, true, true]);
}

#[test]
fn history_files_and_revision_diffs_follow_commits() {
    let dir = repo();
    let root = dir.path();
    let file = root.join("a.txt");
    std::fs::write(&file, "one\n").unwrap();
    git_cmd(root, &["add", "a.txt"]);
    git_cmd(root, &["commit", "-qm", "add a"]);
    std::fs::write(&file, "one\ntwo\n").unwrap();
    std::fs::write(root.join("b.txt"), "b\n").unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "grow a, add b"]);
    std::fs::write(&file, "one\ntwo\nlocal\n").unwrap();

    let history = file_history(root, &file).unwrap();
    assert_eq!(
        history
            .iter()
            .map(|entry| entry.subject.as_str())
            .collect::<Vec<_>>(),
        ["grow a, add b", "add a"]
    );
    let newest = &history[0].hash;
    // Git uses ordinary Windows paths; canonicalize the existing fixture
    // files so the comparison accepts Rust's verbatim Windows prefix too.
    let mut changed: Vec<_> = commit_files(root, newest)
        .unwrap()
        .into_iter()
        .map(|(status, path)| (status, path.canonicalize().unwrap()))
        .collect();
    changed.sort();
    let root_real = root.canonicalize().unwrap();
    assert_eq!(
        changed,
        [
            ('A', root_real.join("b.txt")),
            ('M', root_real.join("a.txt"))
        ]
    );

    // The commit's own change: one added line.
    let commit = revision_document(RevisionDocumentInput {
        cwd: root,
        path: &file,
        left: &format!("{newest}^"),
        right: newest,
    })
    .unwrap();
    assert_eq!(commit.hunks.len(), 1);
    assert_eq!(commit.right_text, "one\ntwo\n");
    assert!(commit.read_only);
    // Against the working tree.
    let local = revision_document(RevisionDocumentInput {
        cwd: root,
        path: &file,
        left: newest,
        right: "",
    })
    .unwrap();
    assert_eq!(local.right_label, "Working tree");
    assert_eq!(local.right_text, "one\ntwo\nlocal\n");
    // A file added by the commit has an empty left side.
    let added = revision_document(RevisionDocumentInput {
        cwd: root,
        path: &root.join("b.txt"),
        left: &format!("{newest}^"),
        right: newest,
    })
    .unwrap();
    assert_eq!(added.left_text, "");
    assert_eq!(added.right_text, "b\n");
}

#[test]
fn revision_diffs_before_a_rename_read_the_old_name() {
    let dir = repo();
    let root = dir.path();
    std::fs::write(root.join("old.txt"), "one\n").unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "add old"]);
    std::fs::write(root.join("old.txt"), "one\ntwo\n").unwrap();
    git_cmd(root, &["commit", "-qam", "grow old"]);
    git_cmd(root, &["mv", "old.txt", "new.txt"]);
    git_cmd(root, &["commit", "-qm", "rename"]);
    let file = root.join("new.txt");
    let history = file_history(root, &file).unwrap();
    assert_eq!(history.len(), 3);
    // The pre-rename commit's own change, read under the old name.
    let grow = &history[1].hash;
    let before = revision_document(RevisionDocumentInput {
        cwd: root,
        path: &file,
        left: &format!("{grow}^"),
        right: grow,
    })
    .unwrap();
    assert_eq!(before.left_text, "one\n");
    assert_eq!(before.right_text, "one\ntwo\n");
    assert!(
        before.left_label.ends_with("old.txt"),
        "{}",
        before.left_label
    );
    // The rename commit itself: old name on the left, new on the right.
    let rename = &history[0].hash;
    let moved = revision_document(RevisionDocumentInput {
        cwd: root,
        path: &file,
        left: &format!("{rename}^"),
        right: rename,
    })
    .unwrap();
    assert_eq!(moved.left_text, moved.right_text);
    assert!(moved.left_label.ends_with("old.txt"));
    assert!(!moved.right_label.contains("old.txt"));
}
