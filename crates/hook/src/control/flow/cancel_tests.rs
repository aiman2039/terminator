use super::cancel::Cancelled;
use super::tests::runner_of;
use super::*;
use terminator_core::flow::StepStatus;

/// Two steps that can start at the same time. Gates start with no daemon.
const TWO: &str = "version = 1\nname = \"n\"\n[[step]]\nid = \"one\"\ngate = \"One?\"\n[[step]]\nid = \"two\"\ngate = \"Two?\"\n";

#[test]
fn cancel_during_a_launch_starts_no_later_step() {
    let (_data, dir, mut runner, _interrupted) = runner_of(TWO);
    let one = runner.ready().remove(0);
    runner.start(&one);
    // Cancel comes while the first launch runs.
    std::fs::write(dir.join(CANCEL), b"").unwrap();
    runner.start_ready();
    assert_eq!(runner.state().status("one"), StepStatus::Running);
    assert_eq!(runner.state().status("two"), StepStatus::Waiting);
    // The run ends as cancelled: no step failed, the later one never ran.
    runner.run().unwrap();
    let state = control::read_state(&dir).unwrap();
    for id in ["one", "two"] {
        assert_eq!(state.status(id), StepStatus::Cancelled, "{id}");
    }
    assert_eq!(state.steps["two"].started, None);
}

#[test]
fn a_signal_during_a_launch_starts_no_later_step() {
    let (_data, _dir, mut runner, interrupted) = runner_of(TWO);
    interrupted.store(true, std::sync::atomic::Ordering::Relaxed);
    runner.start_ready();
    assert_eq!(runner.state().status("one"), StepStatus::Waiting);
    assert_eq!(runner.state().status("two"), StepStatus::Waiting);
}

#[test]
fn launch_stopped_by_cancel_leaves_its_step_waiting_not_failed() {
    let (_data, _dir, mut runner, _interrupted) = runner_of(TWO);
    let one = runner.ready().remove(0);
    runner.started(&one, Err(Cancelled.into()));
    let run = &runner.state().steps["one"];
    assert_eq!(
        (run.status, run.note.as_deref()),
        (StepStatus::Waiting, None)
    );
}
