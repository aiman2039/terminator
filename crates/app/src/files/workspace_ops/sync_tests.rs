use super::super::tests::{git, repo};
use super::*;
use terminator_core::async_process::Processes;

fn text(root: &Path, args: &[&str]) -> String {
    super::super::git_text(root, args).unwrap()
}

async fn sync(root: &Path, op: BranchOp) -> Result<Report, String> {
    let stop = CancellationToken::new();
    let (processes, actor) = Processes::new(stop.clone());
    let owner = tokio::spawn(actor);
    let run = SyncRun {
        op,
        cancel: CancellationToken::new(),
    };
    let result = perform_sync(&processes, root, &run).await;
    stop.cancel();
    owner.await.unwrap().unwrap();
    result
}

#[tokio::test]
async fn push_without_an_upstream_sets_it_on_origin_and_fetch_pull_follow() {
    let dir = repo();
    let root = dir.path();
    let origin = tempfile::tempdir().unwrap();
    git(origin.path(), &["init", "-q", "--bare", "-b", "main"]);
    let url = origin.path().to_string_lossy().into_owned();
    git(root, &["remote", "add", "origin", &url]);
    let pushed = sync(root, BranchOp::Push).await.unwrap();
    assert!(
        matches!(&pushed, Report::Message(text) if text.contains("set the upstream")),
        "{pushed:?}"
    );
    assert_eq!(
        text(root, &["rev-parse", "@{upstream}"]),
        text(root, &["rev-parse", "HEAD"])
    );
    sync(root, BranchOp::Fetch).await.unwrap();
    sync(root, BranchOp::Pull).await.unwrap();
    assert!(matches!(
        sync(root, BranchOp::Push).await,
        Ok(Report::Message(text)) if text == "Pushed"
    ));
}

#[tokio::test]
async fn push_with_no_remote_says_why() {
    let dir = repo();
    let error = sync(dir.path(), BranchOp::Push).await.unwrap_err();
    assert!(error.contains("origin"), "{error}");
}

#[tokio::test]
async fn only_fetch_pull_and_push_run_as_a_sync_job() {
    let dir = repo();
    let error = sync(dir.path(), BranchOp::Merge("main".into()))
        .await
        .unwrap_err();
    assert!(error.contains("Only Fetch"), "{error}");
    assert!(BranchOp::Fetch.is_sync() && BranchOp::Pull.is_sync() && BranchOp::Push.is_sync());
    assert!(!BranchOp::Checkout("abc1234".into()).is_sync());
}

#[test]
fn a_second_sync_in_one_repository_waits_and_another_repository_runs() {
    let mut slots = SyncSlots::default();
    let (one, two) = (Path::new("/one"), Path::new("/two"));
    assert!(slots.start(one, BranchOp::Merge("a".into())).is_none());
    let run = slots.start(one, BranchOp::Pull).unwrap();
    assert_eq!(slots.progress(one).unwrap().line(), "Pulling…");
    assert!(slots.start(one, BranchOp::Fetch).is_none());
    assert!(slots.start(two, BranchOp::Fetch).is_some());
    slots.progress(one).unwrap().cancel.cancel();
    assert!(run.cancel.is_cancelled());
    assert_eq!(slots.progress(one).unwrap().line(), "Stopping…");
    slots.finish(one);
    assert!(slots.progress(one).is_none());
    assert!(slots.start(one, BranchOp::Push).is_some());
}
