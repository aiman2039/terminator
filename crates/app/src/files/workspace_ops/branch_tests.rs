use super::super::tests::{git, repo};
use super::*;

fn text(root: &Path, args: &[&str]) -> String {
    git_text(root, args).unwrap()
}

fn branch(root: &Path, op: BranchOp) -> Result<Report> {
    perform(root, &op)
}

/// A second commit on `feature`, which `main` does not have.
fn feature(root: &Path) -> String {
    git(root, &["switch", "-qc", "feature"]);
    fs::write(root.join("b.txt"), "feature\n").unwrap();
    git(root, &["add", "b.txt"]);
    git(root, &["commit", "-qm", "feature"]);
    git(root, &["switch", "-q", "main"]);
    text(root, &["rev-parse", "feature"])
}

#[test]
fn new_branch_is_created_and_checked_out() {
    let dir = repo();
    let root = dir.path();
    branch(
        root,
        BranchOp::New {
            name: "topic/one".into(),
            start: None,
        },
    )
    .unwrap();
    assert_eq!(text(root, &["branch", "--show-current"]), "topic/one");
}

#[test]
fn typed_names_that_look_like_options_never_reach_git() {
    let dir = repo();
    let root = dir.path();
    for name in ["-D", "--force", "two words", "bad..name", " "] {
        let new = BranchOp::New {
            name: name.into(),
            start: None,
        };
        assert!(branch(root, new).is_err(), "{name}");
    }
    assert!(branch(root, BranchOp::Merge("--abort".into())).is_err());
    assert!(branch(root, BranchOp::CherryPick("-n".into())).is_err());
    assert!(branch(root, BranchOp::Checkout("HEAD~1".into())).is_err());
    assert_eq!(text(root, &["branch", "--show-current"]), "main");
}

#[test]
fn new_branch_from_a_commit_starts_there() {
    let dir = repo();
    let root = dir.path();
    let base = text(root, &["rev-parse", "HEAD"]);
    feature(root);
    let start = Some(base.get(..10).unwrap().to_owned());
    let new = BranchOp::New {
        name: "from-base".into(),
        start,
    };
    branch(root, new).unwrap();
    assert_eq!(text(root, &["rev-parse", "HEAD"]), base);
}

#[test]
fn rename_moves_the_current_branch() {
    let dir = repo();
    let root = dir.path();
    let rename = BranchOp::Rename {
        from: "main".into(),
        to: "trunk".into(),
    };
    branch(root, rename).unwrap();
    assert_eq!(text(root, &["branch", "--show-current"]), "trunk");
}

#[test]
fn delete_of_an_unmerged_branch_needs_force() {
    let dir = repo();
    let root = dir.path();
    feature(root);
    let soft = BranchOp::Delete {
        name: "feature".into(),
        force: false,
    };
    let error = format!("{:#}", branch(root, soft).unwrap_err());
    assert!(error.contains(NOT_MERGED), "{error}");
    assert!(git_text(root, &["rev-parse", "--verify", "refs/heads/feature"]).is_ok());
    let force = BranchOp::Delete {
        name: "feature".into(),
        force: true,
    };
    branch(root, force).unwrap();
    assert!(
        git_text(
            root,
            &["rev-parse", "--verify", "--quiet", "refs/heads/feature"]
        )
        .is_err()
    );
}

#[test]
fn merge_and_rebase_take_a_branch() {
    let dir = repo();
    let root = dir.path();
    let tip = feature(root);
    branch(root, BranchOp::Merge("feature".into())).unwrap();
    assert_eq!(text(root, &["rev-parse", "HEAD"]), tip, "fast-forward");
    git(root, &["switch", "-qc", "side", "HEAD~1"]);
    fs::write(root.join("c.txt"), "side\n").unwrap();
    git(root, &["add", "c.txt"]);
    git(root, &["commit", "-qm", "side"]);
    branch(root, BranchOp::Rebase("main".into())).unwrap();
    assert_eq!(text(root, &["rev-parse", "HEAD~1"]), tip);
}

#[test]
fn commit_actions_checkout_cherry_pick_and_revert() {
    let dir = repo();
    let root = dir.path();
    let tip = feature(root);
    branch(root, BranchOp::CherryPick(tip.clone())).unwrap();
    assert!(root.join("b.txt").is_file());
    let picked = text(root, &["rev-parse", "HEAD"]);
    branch(root, BranchOp::Revert(picked)).unwrap();
    assert!(!root.join("b.txt").exists());
    branch(root, BranchOp::Checkout(tip.clone())).unwrap();
    assert_eq!(text(root, &["rev-parse", "HEAD"]), tip);
    assert_eq!(text(root, &["branch", "--show-current"]), "");
}

#[test]
fn push_without_an_upstream_sets_it_on_origin_and_fetch_pull_follow() {
    let dir = repo();
    let root = dir.path();
    let origin = tempfile::tempdir().unwrap();
    git(origin.path(), &["init", "-q", "--bare", "-b", "main"]);
    let url = origin.path().to_string_lossy().into_owned();
    git(root, &["remote", "add", "origin", &url]);
    branch(root, BranchOp::Push).unwrap();
    assert_eq!(
        text(root, &["rev-parse", "@{upstream}"]),
        text(root, &["rev-parse", "HEAD"])
    );
    branch(root, BranchOp::Fetch).unwrap();
    branch(root, BranchOp::Pull).unwrap();
}

#[test]
fn push_with_no_remote_says_why() {
    let dir = repo();
    let error = format!("{:#}", branch(dir.path(), BranchOp::Push).unwrap_err());
    assert!(error.contains("origin"), "{error}");
}
