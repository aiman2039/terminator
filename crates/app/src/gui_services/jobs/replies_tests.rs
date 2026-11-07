//! A job that ends without its own reply still answers the view that
//! waits for it (#267).
use super::replies::{Owed, owed};
use super::routing::JobRouting;
use crate::{Job, Update};
use std::path::PathBuf;
use terminator_core::async_service::{Failure, OperationId};

fn vnc_frame(session: &str) -> Job {
    Job::VncFrame {
        session: session.into(),
        since: 0,
        regions: false,
        shm: None,
    }
}

fn remote_list(project: &str) -> Job {
    Job::RemoteRead {
        project: project.into(),
        operation: terminator_core::remote::Operation::List {
            path: String::new(),
        },
    }
}

/// Record `job` as operation `id`, as `Services::send` does.
fn record(owed_jobs: &mut Owed, id: u64, job: &Job) {
    let reply = owed(job).expect("the job owes a reply");
    owed_jobs.record(OperationId(id), &JobRouting::context(job), reply);
}

#[test]
fn service_timeout_clears_vnc_fetching() {
    let mut owed_jobs = Owed::default();
    record(&mut owed_jobs, 1, &vnc_frame("s1"));
    let replies = owed_jobs.settle(OperationId(1), Err(&Failure::Timeout));
    assert!(matches!(
        replies.as_slice(),
        [Update::VncFrame(session, Err(_))] if session == "s1"
    ));
    assert_eq!(owed_jobs.len(), 0);
}

#[test]
fn panicked_remote_read_answers_the_sidebar_and_success_owes_nothing() {
    let mut owed_jobs = Owed::default();
    record(&mut owed_jobs, 1, &remote_list("p"));
    record(&mut owed_jobs, 2, &remote_list("q"));
    assert!(matches!(
        owed_jobs.settle(OperationId(1), Err(&Failure::Panicked)).as_slice(),
        [Update::RemoteReply(project, _, Err(_))] if project == "p"
    ));
    assert!(owed_jobs.settle(OperationId(2), Ok(())).is_empty());
    // Settled once: a second completion for the same id owes nothing.
    assert!(
        owed_jobs
            .settle(OperationId(1), Err(&Failure::Timeout))
            .is_empty()
    );
}

#[test]
fn replaced_read_owes_nothing_but_a_read_cancelled_alone_still_answers() {
    let mut owed_jobs = Owed::default();
    record(&mut owed_jobs, 1, &remote_list("p"));
    record(&mut owed_jobs, 2, &remote_list("p"));
    // Its successor answers the sidebar.
    assert!(
        owed_jobs
            .settle(OperationId(1), Err(&Failure::Cancelled))
            .is_empty()
    );
    // Quit cancelled the newest read; nothing else will answer.
    assert!(matches!(
        owed_jobs
            .settle(OperationId(2), Err(&Failure::Cancelled))
            .as_slice(),
        [Update::RemoteReply(_, _, Err(_))]
    ));
}

#[test]
fn waiting_views_have_failure_replies() {
    let search = Job::Search {
        id: 7,
        root: PathBuf::from("/repo"),
        query: crate::search::Query::default(),
        show_ignored: false,
    };
    assert!(matches!(
        JobRouting::rejection(&search).as_slice(),
        [Update::Search(7, Err(_))]
    ));
    let base = Job::GitBase(PathBuf::from("/repo/a.rs"), 3);
    assert!(matches!(
        JobRouting::rejection(&base).as_slice(),
        [Update::GitBaseFailed(path, 3)] if path == &PathBuf::from("/repo/a.rs")
    ));
    assert!(matches!(
        JobRouting::rejection(&vnc_frame("s")).as_slice(),
        [Update::VncFrame(..)]
    ));
    // A job nobody waits for owes nothing.
    assert!(owed(&Job::HookStatus).is_none());
}

#[test]
fn refused_vnc_frame_answers_its_view() {
    let dir = tempfile::tempdir().unwrap();
    let (updates, received) = std::sync::mpsc::channel();
    let (service, _owner) = crate::gui_services::Services::new(
        terminator_core::Paths::at(dir.path().into()),
        eframe::egui::Context::default(),
        updates,
    )
    .unwrap();
    service.handle().close_admission();
    assert!(service.send(vnc_frame("s1")).is_err());
    assert!(
        received
            .try_iter()
            .any(|update| matches!(update, Update::VncFrame(s, Err(_)) if s == "s1"))
    );
}

#[test]
fn failed_head_read_keeps_markers_and_clears_pending() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = eframe::egui::Context::default();
    let mut app = crate::App::with_context(&ctx, terminator_core::Paths::at(dir.path().into()));
    let path = PathBuf::from("/repo/a.rs");
    app.git_base_pending.insert(path.clone());
    app.git_bases
        .insert(path.clone(), (1, Some("old\n".into()), None));
    app.git_base_failed(path.clone(), 2);
    assert!(!app.git_base_pending.contains(&path));
    let (generation, base, _) = &app.git_bases[&path];
    assert_eq!((*generation, base.as_deref()), (2, Some("old\n")));
}

#[test]
fn unowed_vnc_failure_still_clears_its_view() {
    let context = JobRouting::context(&vnc_frame("s1"));
    assert!(matches!(
        crate::service_failure_update(&context, &Failure::Overloaded),
        Some(Update::VncFrame(session, Err(_))) if session == "s1"
    ));
}
