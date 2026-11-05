//! Diff and gutter writes queue with their repository's workspace actions,
//! where the commit job is admitted: a write sent before Commit lands
//! before Git commits.
use super::routing::JobRouting;
use crate::diff::actions::{Kind, Request, Source, Undo};
use crate::workspace_ops::Op;
use crate::{Job, Tab, Update};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use terminator_core::async_service::{CancellationToken, Failure};

fn key(job: &Job) -> (&'static str, String) {
    let context = JobRouting::context(job);
    (context.subsystem, context.resource)
}

fn commit(root: &Path) -> Job {
    let op = Op::Commit {
        message: "m".into(),
        amend: false,
        signoff: false,
        push: false,
        cancel: CancellationToken::new(),
    };
    Job::Workspace(root.into(), op)
}

fn diff_tab(root: &Path) -> Tab {
    Tab::Diff {
        remote: None,
        cwd: root.into(),
        path: root.join("src/a.rs"),
        staged: false,
    }
}

/// A hunk action on a document of the repository `root`, if known.
fn hunk(root: Option<&Path>, kind: Kind) -> Box<Request> {
    let document = crate::diff::DiffDocument {
        source: root.map(|root| Source::fixture(root, Path::new("src/a.rs"))),
        ..Default::default()
    };
    Box::new(Request::Hunk {
        document: Arc::new(document),
        hunk: 0,
        kind,
        lines: None,
    })
}

fn gutter(cwd: &Path, path: PathBuf) -> Job {
    Job::StageGutterBlock {
        cwd: cwd.into(),
        path,
        lines: 0..1,
        text: String::new(),
    }
}

#[test]
fn diff_and_gutter_writes_queue_with_their_repository_before_its_commit() {
    let root = Path::new("/repo");
    let commit = key(&commit(root));
    for kind in [Kind::Stage, Kind::Unstage, Kind::Discard] {
        let job = Job::DiffAction(diff_tab(root), hunk(Some(root), kind));
        assert_eq!(key(&job), commit, "{kind:?}");
    }
    for index in [true, false] {
        let undo = Request::Undo(Undo::fixture(root, Path::new("src/a.rs"), index));
        let job = Job::DiffAction(diff_tab(root), Box::new(undo));
        assert_eq!(key(&job), commit, "undo, index {index}");
    }
    // The gutter job runs Git in the file's repository root.
    let staged = gutter(root, root.join("src/a.rs"));
    assert_eq!(key(&staged), commit);
    // Another repository has its own queue.
    let other = Job::DiffAction(diff_tab(root), hunk(Some(Path::new("/other")), Kind::Stage));
    assert_ne!(key(&other), commit);
}

#[test]
fn a_failed_diff_write_still_answers_its_tab_or_gutter() {
    let root = Path::new("/repo");
    let tab = diff_tab(root);
    let failed = |job: &Job, failure: &Failure| {
        crate::service_failure_update(&JobRouting::context(job), failure)
    };
    let staged = Job::DiffAction(tab.clone(), hunk(Some(root), Kind::Stage));
    assert!(matches!(
        failed(&staged, &Failure::Timeout),
        Some(Update::DiffActionFailed(key, _)) if key == tab.key()
    ));
    // A document with no source keeps the tab's own queue.
    let sourceless = Job::DiffAction(tab.clone(), hunk(None, Kind::Stage));
    assert_eq!(key(&sourceless).0, "diff-action");
    assert!(matches!(
        failed(&sourceless, &Failure::Panicked),
        Some(Update::DiffActionFailed(key, _)) if key == tab.key()
    ));
    let path = root.join("src/a.rs");
    let block = gutter(root, path.clone());
    assert!(matches!(
        failed(&block, &Failure::Timeout),
        Some(Update::DiffActionFailed(key, _)) if key == format!("gutter:{}", path.display())
    ));
    // Other workspace actions report in the status bar.
    let stage = Job::Workspace(root.into(), Op::Stage(path));
    assert!(matches!(
        failed(&stage, &Failure::Timeout),
        Some(Update::Error(_))
    ));
}
