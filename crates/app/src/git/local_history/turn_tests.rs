use super::*;
use crate::Paths;
use crate::native_editor::NativeDoc;
use terminator_core::local_history::{Entry, Source, TurnFile, TurnMark};

fn entry(id: char, path: &str) -> Entry {
    Entry {
        id: id.to_string().repeat(64),
        path: path.into(),
        timestamp_ms: 1,
        size: 1,
        label: None,
        source: Source::External,
    }
}

fn file(path: &str, base: Option<char>, index: bool, newest: char) -> TurnFile {
    TurnFile {
        path: path.into(),
        base: base.map(|id| entry(id, path)),
        base_is_git_index: index,
        newest: Some(newest.to_string().repeat(64)),
    }
}

fn turn() -> Turn {
    Turn {
        session: "s".into(),
        project_root: "/p".into(),
        mark: TurnMark {
            invocation_id: "i".into(),
            agent_kind: "claude".into(),
            started_ms: 10,
            ended_ms: Some(20),
        },
        files: vec![
            file("/p/a.rs", Some('a'), false, 'b'),
            file("/p/b.rs", Some('c'), true, 'd'),
            file("/p/new.rs", None, false, 'e'),
            file("/p/same.rs", Some('f'), false, 'f'),
        ],
        truncated: false,
    }
}

fn app_with_turn(dir: &std::path::Path) -> (App, std::sync::mpsc::Receiver<Job>) {
    let ctx = eframe::egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.into()));
    let (jobs, requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    app.state.capabilities.push(TURN_CAPABILITY.into());
    app.open_last_turn("s".into());
    app.handle_turn_reply(TurnReply::Loaded("s".into(), Ok(Some(Box::new(turn())))));
    let _ = requests.try_iter().count();
    (app, requests)
}

#[test]
fn revert_plan_keeps_new_and_unchanged_files_and_git_index_bases_apart() {
    let plan = plan(&turn());
    let paths = |items: &[RevertItem]| -> Vec<PathBuf> {
        items.iter().map(|item| item.path.clone()).collect()
    };
    assert_eq!(paths(&plan.items), [PathBuf::from("/p/a.rs")]);
    assert_eq!(paths(&plan.index_items), [PathBuf::from("/p/b.rs")]);
    assert_eq!(
        plan.skipped.len(),
        2,
        "new and unchanged files are not written"
    );
    assert_eq!(plan.chosen(false).len(), 1, "Git index bases are opt in");
    assert_eq!(plan.chosen(true).len(), 2);
}

#[test]
fn revert_last_turn_refuses_an_unsaved_buffer_before_any_write() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, requests) = app_with_turn(dir.path());
    app.native_docs.insert(
        "/p/a.rs".into(),
        NativeDoc::dirty_for_test("/p/a.rs".into()),
    );
    app.revert_last_turn();
    assert!(
        !requests.try_iter().any(|job| matches!(
            job,
            Job::LocalHistory(crate::local_history::Op::Turn(TurnOp::Revert(..)))
        )),
        "nothing is sent"
    );
    assert!(
        app.error
            .text()
            .unwrap_or_default()
            .contains("Save /p/a.rs first")
    );
    assert!(!app.git_extras.last_turn.as_ref().unwrap().busy);
}

#[test]
fn revert_last_turn_refuses_while_the_agent_is_still_working() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, requests) = app_with_turn(dir.path());
    app.state.agents.push(terminator_core::Agent {
        invocation_id: "i".into(),
        session_id: "s".into(),
        kind: "claude".into(),
        provider_session_id: None,
        state: AgentState::Running,
        sequence: None,
        updated: 1,
        resume: None,
        process: None,
    });
    app.revert_last_turn();
    assert_eq!(requests.try_iter().count(), 0);
    assert!(
        app.error
            .text()
            .unwrap_or_default()
            .contains("still working")
    );
}

#[test]
fn a_clean_revert_sends_only_the_files_with_a_turn_start_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, requests) = app_with_turn(dir.path());
    app.revert_last_turn();
    let sent: Vec<Vec<RevertItem>> = requests
        .try_iter()
        .filter_map(|job| match job {
            Job::LocalHistory(crate::local_history::Op::Turn(TurnOp::Revert(_, items))) => {
                Some(items)
            }
            _ => None,
        })
        .collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].len(), 1);
    assert_eq!(sent[0][0].base, "a".repeat(64));
    assert!(app.git_extras.last_turn.as_ref().unwrap().busy);
}

#[test]
fn a_partial_revert_names_every_file_it_did_not_revert() {
    let report = RevertReport {
        reverted: vec!["/p/a.rs".into()],
        failed: Some(("/p/b.rs".into(), "disk full".into())),
        not_tried: vec!["/p/c.rs".into()],
        ..RevertReport::default()
    };
    assert!(!report.complete());
    let text = report.summary();
    assert!(text.contains("Reverted 1 file(s)"));
    assert!(text.contains("/p/b.rs: disk full"));
    assert!(text.contains("Not reverted: /p/c.rs"));
    let refused = RevertReport {
        refused: vec![("/p/a.rs".into(), "changed".into())],
        ..RevertReport::default()
    };
    assert!(
        refused
            .summary()
            .starts_with("Revert refused; no file was changed.")
    );
}

#[test]
fn a_reply_for_another_session_is_ignored_and_leaving_drops_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _requests) = app_with_turn(dir.path());
    app.handle_turn_reply(TurnReply::Loaded("other".into(), Ok(None)));
    assert!(matches!(
        app.git_extras.last_turn.as_ref().unwrap().turn,
        Some(Ok(Some(_)))
    ));
    app.hide_center_overlay();
    assert!(app.git_extras.last_turn.is_none());
}

#[test]
fn an_older_daemon_never_gets_the_turn_request() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = eframe::egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let (jobs, requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    app.open_last_turn("s".into());
    assert_eq!(requests.try_iter().count(), 0);
    assert!(app.git_extras.last_turn.is_none());
}
