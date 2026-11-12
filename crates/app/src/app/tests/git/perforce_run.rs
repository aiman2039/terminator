//! Long Perforce actions run one at a time as cancellable jobs.
use super::*;
use crate::perforce::{Op as P4, RUN_BUSY, RUN_CANCELLED};
use crate::workspace_ops::{Op, Report};

fn perforce_app() -> (App, egui::Context, tempfile::TempDir, mpsc::Receiver<Job>) {
    let (mut app, ctx, dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    app.context = Some(services::ContextData {
        cwd: "/a".into(),
        root: Some("/a".into()),
        perforce: None,
        git_dirs: vec![],
        branch: String::new(),
        changes: vec![],
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    });
    (app, ctx, dir, requests)
}

fn long_run(requests: &mpsc::Receiver<Job>) -> (PathBuf, Op) {
    match requests.try_recv() {
        Ok(Job::Workspace(root, op @ Op::PerforceRun(_))) => (root, op),
        _ => panic!("expected a long Perforce job"),
    }
}

#[test]
fn a_second_long_perforce_action_waits_for_the_first_reply() {
    let (mut app, ctx, _dir, requests) = perforce_app();
    app.queue_perforce(P4::Sync);
    let (root, op) = long_run(&requests);
    app.queue_perforce(P4::Submit("7".into()));
    assert!(requests.try_recv().is_err(), "one run at a time");
    assert_eq!(app.info.as_deref(), Some(RUN_BUSY));
    // A short action still queues on the root's queue.
    app.queue_perforce(P4::Edit("/a/x.txt".into()));
    assert!(matches!(
        requests.try_recv(),
        Ok(Job::Workspace(_, Op::Perforce(P4::Edit(_))))
    ));
    app.update_tx
        .send(Update::Workspace(
            root,
            op,
            Ok(Report::Message("synced".into())),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.git.perforce.progress().is_none());
    app.queue_perforce(P4::Submit("7".into()));
    long_run(&requests);
}

#[test]
fn a_cancelled_perforce_action_is_a_note_not_an_error() {
    let (mut app, ctx, _dir, requests) = perforce_app();
    app.queue_perforce(P4::Shelve("7".into()));
    let (root, op) = long_run(&requests);
    let Op::PerforceRun(run) = &op else {
        unreachable!("long_run returns a run");
    };
    app.git
        .perforce
        .progress()
        .expect("a run is in flight")
        .cancel
        .cancel();
    assert!(run.cancel.is_cancelled(), "Cancel reaches the job's p4");
    app.update_tx
        .send(Update::Workspace(root, op, Err(RUN_CANCELLED.into())))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.git.perforce.progress().is_none());
    assert_eq!(app.info.as_deref(), Some(RUN_CANCELLED));
}
