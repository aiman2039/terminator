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
        "{hash}\n1111111\nJane\njane@example.invalid\n1700000000\n{parent}\nHEAD -> refs/heads/main, tag: refs/tags/v1\nsubject line\n\nbody line one\nbody line two\n>>>>>END<<<<<\n"
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
    let refs: Vec<(&str, RefLabelKind)> = commit
        .refs
        .iter()
        .map(|label| (label.name.as_str(), label.kind))
        .collect();
    assert_eq!(
        refs,
        [
            ("HEAD", RefLabelKind::Head),
            ("main", RefLabelKind::LocalBranch),
            ("v1", RefLabelKind::Tag)
        ]
    );
    assert_eq!(commit.timestamp, 1_700_000_000);
    // The parent is outside the log: its lane runs off the bottom.
    assert_eq!(commit.graph.down, [LaneLine { col: 0, color: 0 }]);
}

fn labels(decorations: &str) -> Vec<(String, RefLabelKind)> {
    parse_ref_labels(decorations)
        .into_iter()
        .map(|label| (label.name, label.kind))
        .collect()
}

#[test]
fn ref_labels_list_head_then_local_remote_and_tag_refs() {
    use RefLabelKind::{Head, LocalBranch, RemoteBranch, Tag};
    let parsed = labels(
        "tag: refs/tags/v1.0, HEAD -> refs/heads/main, refs/remotes/origin/main, \
         refs/remotes/origin/HEAD, refs/heads/feature/login, tag: refs/tags/nightly",
    );
    let want = [
        ("HEAD", Head),
        ("main", LocalBranch),
        // A slash in a local branch name does not make it remote.
        ("feature/login", LocalBranch),
        ("origin/main", RemoteBranch),
        ("v1.0", Tag),
        ("nightly", Tag),
    ];
    assert_eq!(
        parsed,
        want.map(|(name, kind)| (name.to_owned(), kind)).to_vec()
    );
}

#[test]
fn ref_labels_show_a_detached_head_and_skip_other_refs() {
    use RefLabelKind::{Head, RemoteBranch};
    assert_eq!(
        labels("HEAD, grafted, refs/notes/commits, refs/remotes/upstream/dev"),
        [
            ("HEAD".to_owned(), Head),
            ("upstream/dev".to_owned(), RemoteBranch)
        ]
    );
    assert!(labels("").is_empty());
}

#[test]
fn fetch_log_lays_out_merge_lanes_and_leaves_out_stash_entries() {
    let dir = repo();
    let root = dir.path();
    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "base"]);
    git_cmd(root, &["checkout", "-qb", "feature/x"]);
    std::fs::write(root.join("b.txt"), "b\n").unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "on feature"]);
    git_cmd(root, &["checkout", "-q", "main"]);
    std::fs::write(root.join("c.txt"), "c\n").unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "on main"]);
    git_cmd(root, &["tag", "v1"]);
    git_cmd(
        root,
        &["merge", "-q", "--no-ff", "feature/x", "-m", "merge x"],
    );
    std::fs::write(root.join("a.txt"), "dirty\n").unwrap();
    git_cmd(root, &["stash", "-q"]);

    let log = fetch_log(root).expect("log should load");
    let subjects: Vec<&str> = log.commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(
        subjects.len(),
        4,
        "stash entries are not history: {subjects:?}"
    );
    // Children always come before their parents.
    for (index, commit) in log.commits.iter().enumerate() {
        for parent in &commit.parents {
            let at = log.commits.iter().position(|c| &c.hash == parent);
            assert!(at.is_none_or(|at| at > index), "{subjects:?}");
        }
    }
    let row = |subject: &str| {
        let commit = log.commits.iter().find(|c| c.subject == subject).unwrap();
        let names: Vec<&str> = commit.refs.iter().map(|r| r.name.as_str()).collect();
        (commit.graph.clone(), names)
    };
    let (merge, merge_refs) = row("merge x");
    assert_eq!(merge.down.len(), 2, "{merge:?}");
    assert_eq!(merge_refs, ["HEAD", "main"]);
    assert_eq!(row("on feature").1, ["feature/x"]);
    assert_eq!(row("on main").1, ["v1"]);
    let (base, _) = row("base");
    assert_eq!(base.up.len(), 2, "both lines join at the fork: {base:?}");
    assert_eq!(log.lanes, 2);
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
fn commit_files_with_non_ascii_names_open_as_real_paths() {
    let dir = repo();
    let root = dir.path();
    std::fs::write(root.join("naïve.md"), "one\n").unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "add"]);
    git_cmd(root, &["mv", "naïve.md", "naïve 2.md"]);
    git_cmd(root, &["commit", "-qm", "rename"]);
    let history = file_history(root, &root.join("naïve 2.md")).unwrap();
    let head = history[0].hash.as_str();
    let changed = commit_files(root, head).unwrap();
    let names: Vec<_> = changed
        .iter()
        .map(|(status, path)| (*status, path.file_name().unwrap().to_owned()))
        .collect();
    assert_eq!(names, [('R', "naïve 2.md".into())]);
    let before = revision_document(RevisionDocumentInput {
        cwd: root,
        path: &root.join("naïve 2.md"),
        left: &format!("{head}^"),
        right: head,
    })
    .unwrap();
    assert_eq!(before.left_text, "one\n", "the old name is read before");
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

#[test]
fn compare_with_a_branch_reads_the_name_the_file_had_there() {
    let dir = repo();
    let root = dir.path();
    let text = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\nline 8\n";
    std::fs::write(root.join("old.txt"), text).unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "add old"]);
    git_cmd(root, &["branch", "base"]);
    git_cmd(root, &["mv", "old.txt", "new.txt"]);
    git_cmd(root, &["commit", "-qm", "rename"]);
    std::fs::write(root.join("new.txt"), format!("{text}line 9\n")).unwrap();
    let file = root.join("new.txt");
    // Compare with… → `base`: no commit of the file's own log.
    let diff = revision_document(RevisionDocumentInput {
        cwd: root,
        path: &file,
        left: "base",
        right: "",
    })
    .unwrap();
    assert_eq!(diff.left_text, text, "not an empty, all-new left side");
    assert!(diff.left_label.ends_with("old.txt"), "{}", diff.left_label);
}

#[test]
fn old_commit_date_shows_real_calendar_day() {
    // 2024-03-15 12:00:00 UTC.
    assert_eq!(relative_date(1_710_504_000), "2024-03-15");
    assert_eq!(relative_date(0), "");
}
