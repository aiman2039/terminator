use super::picker::{Choices, Pick, Row, pickable, rows, stepped};
use super::*;
use crate::git_log::HistoryEntry;
use crate::workspace_ops::{Op, Report};
use std::path::PathBuf;

const REFS: &str = "refs/heads/zeta\t \t100\t
refs/heads/main\t*\t50\t
refs/heads/alpha\t \t200\t
refs/remotes/origin/HEAD\t \t0\trefs/remotes/origin/main
refs/remotes/origin/main\t \t50\t
refs/tags/v1\t \t100\t
refs/tags/v2\t \t300\t
refs/tags/alpha\t \t10\t
";

fn names(targets: &[CompareTarget]) -> Vec<&str> {
    targets.iter().map(|target| target.name.as_str()).collect()
}

fn labels(rows: &[Row<'_>]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            Row::Header(text) => format!("# {text}"),
            Row::Target(target) => target.name.clone(),
            Row::Commit(entry) => entry.short_hash.clone(),
            Row::Typed(text) => format!("? {text}"),
        })
        .collect()
}

fn commit(hash: &str, author: &str, subject: &str) -> HistoryEntry {
    HistoryEntry {
        hash: hash.into(),
        short_hash: hash.chars().take(7).collect(),
        author: author.into(),
        date: "2024-03-01".into(),
        subject: subject.into(),
    }
}

#[test]
fn targets_list_the_current_branch_first_then_remotes_then_newest_tags() {
    let targets = parse_targets(REFS);
    assert_eq!(
        names(&targets),
        ["main", "alpha", "zeta", "origin/main", "v2", "v1", "alpha"],
        "origin/HEAD only names origin/main"
    );
    assert!(targets.first().is_some_and(|main| main.current));
    // A branch and a tag share `alpha`: Git gets the full ref names.
    let revisions: Vec<&str> = targets.iter().map(CompareTarget::revision).collect();
    assert_eq!(
        revisions,
        [
            "main",
            "refs/heads/alpha",
            "zeta",
            "origin/main",
            "v2",
            "v1",
            "refs/tags/alpha"
        ]
    );
}

#[test]
fn a_ref_named_like_an_option_compares_by_its_full_name() {
    let targets = parse_targets("refs/tags/--output=/tmp/x\t \t5\t\nrefs/heads/-b\t \t5\t\n");
    let revisions: Vec<&str> = targets.iter().map(CompareTarget::revision).collect();
    assert_eq!(revisions, ["refs/heads/-b", "refs/tags/--output=/tmp/x"]);
    assert_eq!(
        names(&targets),
        ["-b", "--output=/tmp/x"],
        "the list keeps the names"
    );
}

#[test]
fn ref_rows_group_matches_under_headers_and_offer_the_typed_revision() {
    let choices = Choices::Targets(parse_targets(REFS));
    let all = rows(&choices, "");
    assert_eq!(
        labels(&all),
        [
            "# LOCAL BRANCHES",
            "main",
            "alpha",
            "zeta",
            "# REMOTE BRANCHES",
            "origin/main",
            "# TAGS",
            "v2",
            "v1",
            "alpha"
        ]
    );
    // Case does not matter; text that names no ref is offered as a revision.
    assert_eq!(
        labels(&rows(&choices, " MA ")),
        [
            "# LOCAL BRANCHES",
            "main",
            "# REMOTE BRANCHES",
            "origin/main",
            "# REVISION",
            "? MA"
        ]
    );
    // An exact name needs no revision row.
    assert_eq!(
        labels(&rows(&choices, "origin/main")),
        ["# REMOTE BRANCHES", "origin/main"]
    );
    let typed = rows(&choices, "HEAD~2");
    assert_eq!(labels(&typed), ["# REVISION", "? HEAD~2"]);
    assert_eq!(pickable(&typed), [1]);
    assert_eq!(pickable(&all), [1, 2, 3, 5, 7, 8, 9]);
}

#[test]
fn history_rows_match_the_subject_author_or_hash_prefix() {
    let choices = Choices::Commits(vec![
        commit("abc1234ffff", "Ada", "Fix the parser"),
        commit("def5678eeee", "Grace", "Add tests"),
    ]);
    assert_eq!(labels(&rows(&choices, "")), ["abc1234", "def5678"]);
    assert_eq!(labels(&rows(&choices, "parser")), ["abc1234"]);
    assert_eq!(labels(&rows(&choices, "grace")), ["def5678"]);
    assert_eq!(labels(&rows(&choices, "DEF5")), ["def5678"]);
    assert!(rows(&choices, "nothing").is_empty());
    assert!(rows(&Choices::Loading, "main").is_empty());
}

#[test]
fn picks_open_refs_and_commits_and_check_typed_text_first() {
    let targets = parse_targets(REFS);
    let entry = commit("abc1234ffff", "Ada", "Fix");
    let main = targets.first().unwrap();
    let alpha = targets.get(1).unwrap();
    assert_eq!(Row::Target(main).pick(), Some(Pick::Open("main".into())));
    assert_eq!(
        Row::Target(alpha).pick(),
        Some(Pick::Open("refs/heads/alpha".into()))
    );
    assert_eq!(
        Row::Commit(&entry).pick(),
        Some(Pick::Open("abc1234ffff".into()))
    );
    assert_eq!(
        Row::Typed("HEAD~2").pick(),
        Some(Pick::Verify("HEAD~2".into()))
    );
    assert_eq!(Row::Header("TAGS").pick(), None);
}

#[test]
fn selection_steps_wrap_over_the_pickable_rows() {
    assert_eq!(stepped(0, true, 3), 1);
    assert_eq!(stepped(2, true, 3), 0);
    assert_eq!(stepped(0, false, 3), 2);
    assert_eq!(stepped(9, false, 3), 1, "a stale index stays in range");
    assert_eq!(stepped(0, true, 0), 0);
}

#[test]
fn typed_revisions_refuse_options_spaces_and_empty_text() {
    assert_eq!(typed_revision("  abc123 "), Ok("abc123"));
    assert_eq!(typed_revision("HEAD~2"), Ok("HEAD~2"));
    assert!(typed_revision("   ").is_err());
    for text in ["--output=/tmp/x", "-p", "a b", "a\u{7}b"] {
        let refused = typed_revision(text).unwrap_err();
        assert!(refused.contains("is not a revision"), "{text}: {refused}");
    }
}

#[test]
fn picker_answers_fill_the_list_and_drop_stale_ones() {
    let file = PathBuf::from("/repo/src/a.rs");
    let mut refs = ComparePicker::new(PickerMode::Refs, &file).unwrap();
    assert_eq!(refs.cwd, PathBuf::from("/repo/src"));
    let stale = Op::FileHistory(file.clone());
    assert!(refs.answer(&stale, Ok(Report::History(vec![]))).is_none());
    assert!(matches!(refs.choices, Choices::Loading));
    let listed = Ok(Report::CompareTargets(parse_targets(REFS)));
    assert!(refs.answer(&Op::CompareTargets, listed).is_none());
    assert!(matches!(&refs.choices, Choices::Targets(list) if list.len() == 7));
    // Only the typed text being checked counts; a refusal stays visible.
    refs.checking = Some("HEAD~2".into());
    let other = Op::VerifyRevision("HEAD~9".into());
    assert!(refs.answer(&other, Err("no".into())).is_none());
    assert_eq!(refs.checking.as_deref(), Some("HEAD~2"));
    let asked = Op::VerifyRevision("HEAD~2".into());
    let refused = Err("No commit matches 'HEAD~2'".to_owned());
    assert!(refs.answer(&asked, refused).is_none());
    assert_eq!(refs.note.as_deref(), Some("No commit matches 'HEAD~2'"));
    assert!(refs.checking.is_none());
    refs.checking = Some("HEAD~2".into());
    let hash = "a".repeat(40);
    let opened = refs.answer(&asked, Ok(Report::Revision(hash.clone())));
    assert_eq!(
        opened,
        Some(crate::Tab::RevisionDiff {
            cwd: "/repo/src".into(),
            path: file.clone(),
            left: hash,
            right: String::new(),
        })
    );
    let mut history = ComparePicker::new(PickerMode::History, &file).unwrap();
    let other_file = Op::FileHistory("/repo/src/b.rs".into());
    assert!(history.answer(&other_file, Err("x".into())).is_none());
    assert!(matches!(history.choices, Choices::Loading));
    assert!(
        history
            .answer(&history.load_op(), Err("git log: boom".into()))
            .is_none()
    );
    assert!(matches!(&history.choices, Choices::Failed(error) if error == "git log: boom"));
}

fn git(root: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Ada")
        .env("GIT_AUTHOR_EMAIL", "ada@example.invalid")
        .env("GIT_COMMITTER_NAME", "Ada")
        .env("GIT_COMMITTER_EMAIL", "ada@example.invalid")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A repository with two commits on `main`, a `feature` branch, a `v1`
/// tag, and `origin/main` with its symbolic `origin/HEAD`.
fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    git(root, &["add", "a.txt"]);
    git(root, &["commit", "-qm", "first"]);
    git(root, &["branch", "feature"]);
    git(root, &["tag", "v1"]);
    std::fs::write(root.join("a.txt"), "two\n").unwrap();
    git(root, &["commit", "-qam", "second"]);
    git(root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(
        root,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    dir
}

#[test]
fn list_targets_reads_branches_remotes_and_tags_from_git() {
    let dir = repository();
    let targets = list_targets(dir.path()).unwrap();
    assert_eq!(names(&targets), ["main", "feature", "origin/main", "v1"]);
    assert!(targets.first().is_some_and(|main| main.current));
    let outside = tempfile::tempdir().unwrap();
    assert!(list_targets(outside.path()).is_err());
}

#[test]
fn verify_revision_resolves_commits_and_refuses_trees_and_unknown_names() {
    let dir = repository();
    let root = dir.path();
    let first = git(root, &["rev-parse", "HEAD~1"]);
    assert_eq!(verify_revision(root, "HEAD~1"), Ok(first.clone()));
    assert_eq!(verify_revision(root, " feature "), Ok(first.clone()));
    assert_eq!(verify_revision(root, "v1"), Ok(first.clone()));
    let short = first.get(..7).unwrap().to_owned();
    assert_eq!(verify_revision(root, &short), Ok(first));
    for text in ["HEAD:a.txt", "nope", "main..feature"] {
        let refused = verify_revision(root, text).unwrap_err();
        assert_eq!(refused, format!("No commit matches '{text}'"));
    }
    // Text Git would read as an option never reaches Git.
    let option = verify_revision(root, "--output=/tmp/compare-test").unwrap_err();
    assert!(option.contains("is not a revision"), "{option}");
    assert!(!std::path::Path::new("/tmp/compare-test").exists());
}
