use super::*;
use std::sync::mpsc;

fn app(dir: &tempfile::TempDir) -> (App, mpsc::Receiver<Job>) {
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, terminator_core::Paths::at(dir.path().into()));
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.context.replace(crate::services::ContextData {
        cwd: "/a".into(),
        root: Some("/a".into()),
        perforce: None,
        git_dirs: vec!["/a/.git".into()],
        branch: "main".into(),
        changes: Vec::new(),
        decorations: Default::default(),
        stats: Default::default(),
        error: None,
    });
    (app, requests)
}

fn branch_ops(requests: &mpsc::Receiver<Job>) -> Vec<BranchOp> {
    requests
        .try_iter()
        .filter_map(|job| match job {
            Job::Workspace(_, Op::Branch(op)) => Some(op),
            _ => None,
        })
        .collect()
}

#[test]
fn sync_and_commit_actions_run_on_their_click() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, requests) = app(&dir);
    app.branch_request(BranchRequest::Run(BranchOp::Pull));
    assert_eq!(branch_ops(&requests), vec![BranchOp::Pull]);
    assert!(app.git_extras.dialog.is_none());
}

#[test]
fn naming_and_destructive_actions_ask_first_and_send_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, requests) = app(&dir);
    app.branch_request(BranchRequest::Ask(BranchDialog::Delete("feature".into())));
    assert!(branch_ops(&requests).is_empty());
    let pending = app.git_extras.dialog.clone().unwrap();
    assert_eq!(pending.dialog, BranchDialog::Delete("feature".into()));
    // Delete is destructive: Enter never presses it.
    assert_eq!(pending.dialog.destructive(), Some("Delete"));
}

#[test]
fn a_new_branch_without_a_name_cannot_be_created() {
    let empty = BranchDialog::New {
        start: None,
        name: "  ".into(),
    };
    assert_eq!(empty.op(), None);
    let named = BranchDialog::New {
        start: Some("abc1234".into()),
        name: " topic ".into(),
    };
    assert_eq!(
        named.op(),
        Some(BranchOp::New {
            name: "topic".into(),
            start: Some("abc1234".into()),
        })
    );
}

#[test]
fn an_unmerged_branch_asks_again_with_force_delete() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _requests) = app(&dir);
    let op = BranchOp::Delete {
        name: "feature".into(),
        force: false,
    };
    let error = Err(format!(
        "Delete branch failed: error: the branch 'feature' is {NOT_MERGED}."
    ));
    app.branch_op_finished(Path::new("/a"), &op, &error);
    let pending = app.git_extras.dialog.clone().unwrap();
    assert_eq!(pending.dialog, BranchDialog::ForceDelete("feature".into()));
    assert_eq!(
        pending.dialog.op(),
        Some(BranchOp::Delete {
            name: "feature".into(),
            force: true,
        })
    );
    // Force is never the default action.
    assert_eq!(pending.dialog.destructive(), Some("Force delete"));
}
