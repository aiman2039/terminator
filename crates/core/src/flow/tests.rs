use super::*;

const PLAN_AND_BUILD: &str = r#"
version = 1
name = "Plan and build"

[[step]]
id = "plan"
command = "claude"
prompt = "Write the plan.\nKeep it short."
worktree = "plan"

[[step]]
id = "approve"
gate = "Is the plan correct?"
needs = ["plan"]

[[step]]
id = "build"
command = "codex"
cwd = "crates/app"
needs = ["approve"]

[[step]]
id = "docs"
command = "claude"
needs = ["approve"]
"#;

fn flow() -> Flow {
    parse(PLAN_AND_BUILD).unwrap()
}

fn one_step(body: &str) -> Result<Flow, FlowError> {
    parse(&format!("version = 1\nname = \"n\"\n[[step]]\n{body}\n"))
}

#[test]
fn valid_file_keeps_steps_in_file_order() {
    let flow = flow();
    assert_eq!(flow.name, "Plan and build");
    let ids: Vec<_> = flow.steps.iter().map(|step| step.id.as_str()).collect();
    assert_eq!(ids, ["plan", "approve", "build", "docs"]);
    assert_eq!(
        flow.step("approve").unwrap().kind,
        StepKind::Gate {
            message: "Is the plan correct?".into(),
            options: Vec::new(),
            pick: None,
        }
    );
    assert_eq!(
        flow.step("build").unwrap().kind,
        StepKind::Command {
            command: "codex".into(),
            prompt: None,
            cwd: Some("crates/app".into()),
            worktree: None,
            race: None,
        }
    );
}

#[test]
fn unknown_key_is_refused() {
    let error = one_step("id = \"a\"\ncommand = \"x\"\nneed = [\"b\"]").unwrap_err();
    assert!(matches!(error, FlowError::Syntax(text) if text.contains("need")));
    let error = parse("version = 1\nname = \"n\"\nextra = 1\n").unwrap_err();
    assert!(matches!(error, FlowError::Syntax(_)));
}

#[test]
fn missing_or_other_version_is_refused() {
    assert!(matches!(
        parse("name = \"n\"\n[[step]]\nid = \"a\"\ncommand = \"x\"\n"),
        Err(FlowError::Syntax(_))
    ));
    assert_eq!(
        parse("version = 2\nname = \"n\"\n[[step]]\nid = \"a\"\ncommand = \"x\"\n"),
        Err(FlowError::Version(2))
    );
}

#[test]
fn flow_without_steps_is_refused() {
    assert_eq!(parse("version = 1\nname = \"n\"\n"), Err(FlowError::Empty));
}

#[test]
fn bad_and_duplicate_ids_are_refused() {
    assert_eq!(
        one_step("id = \"Plan 1\"\ncommand = \"x\""),
        Err(FlowError::BadId("Plan 1".into()))
    );
    assert_eq!(
        one_step("id = \"a\"\ncommand = \"x\"\n[[step]]\nid = \"a\"\ncommand = \"y\""),
        Err(FlowError::DuplicateId("a".into()))
    );
}

#[test]
fn step_needs_exactly_one_of_command_or_gate() {
    assert_eq!(one_step("id = \"a\""), Err(FlowError::Kind("a".into())));
    assert_eq!(
        one_step("id = \"a\"\ncommand = \"x\"\ngate = \"y\""),
        Err(FlowError::Kind("a".into()))
    );
    assert_eq!(
        one_step("id = \"a\"\ngate = \"y\"\nprompt = \"p\""),
        Err(FlowError::GateField("a".into()))
    );
}

#[test]
fn command_with_a_line_break_is_refused() {
    assert_eq!(
        one_step("id = \"a\"\ncommand = \"echo 1\\nrm -rf x\""),
        Err(FlowError::Text("a".into(), "command"))
    );
    assert_eq!(
        one_step("id = \"a\"\ncommand = \"x\"\nprompt = \"a\\u001bb\""),
        Err(FlowError::Text("a".into(), "prompt"))
    );
}

#[test]
fn cwd_outside_the_project_is_refused() {
    for cwd in ["../other", "/etc"] {
        assert_eq!(
            one_step(&format!("id = \"a\"\ncommand = \"x\"\ncwd = \"{cwd}\"")),
            Err(FlowError::Cwd("a".into()))
        );
    }
}

#[test]
fn worktree_name_that_looks_like_an_option_is_refused() {
    for name in ["-f", "a..b", "a b", "/a", "a/"] {
        assert_eq!(
            one_step(&format!(
                "id = \"a\"\ncommand = \"x\"\nworktree = \"{name}\""
            )),
            Err(FlowError::Worktree("a".into()))
        );
    }
}

#[test]
fn unknown_need_and_cycle_are_refused() {
    assert_eq!(
        one_step("id = \"a\"\ncommand = \"x\"\nneeds = [\"b\"]"),
        Err(FlowError::UnknownNeed {
            step: "a".into(),
            needs: "b".into()
        })
    );
    let cycle = one_step(
        "id = \"a\"\ncommand = \"x\"\nneeds = [\"b\"]\n[[step]]\nid = \"b\"\ncommand = \"y\"\nneeds = [\"a\"]",
    );
    assert!(matches!(cycle, Err(FlowError::Cycle(_))));
    assert_eq!(
        one_step("id = \"a\"\ncommand = \"x\"\nneeds = [\"a\"]"),
        Err(FlowError::Cycle("a".into()))
    );
}

fn ready_ids(flow: &Flow, state: &RunState) -> Vec<String> {
    flow.ready(state)
        .into_iter()
        .map(|step| step.id.clone())
        .collect()
}

#[test]
fn step_is_ready_only_when_every_needed_step_is_done() {
    let flow = flow();
    let mut state = RunState::new(&flow, "plan", "p");
    assert_eq!(ready_ids(&flow, &state), ["plan"]);
    state.set("plan", StepStatus::Running);
    assert!(ready_ids(&flow, &state).is_empty());
    assert_eq!(flow.outcome(&state), Outcome::Active);
    state.set("plan", StepStatus::Done);
    assert_eq!(ready_ids(&flow, &state), ["approve"]);
    state.set("approve", StepStatus::Done);
    assert_eq!(ready_ids(&flow, &state), ["build", "docs"]);
}

#[test]
fn rejected_gate_stops_the_steps_after_it() {
    let flow = flow();
    let mut state = RunState::new(&flow, "plan", "p");
    state.set("plan", StepStatus::Done);
    state.set("approve", StepStatus::Failed);
    assert!(ready_ids(&flow, &state).is_empty());
    assert_eq!(flow.outcome(&state), Outcome::Stopped);
    assert_eq!(state.status("build"), StepStatus::Waiting);
}

#[test]
fn run_is_done_when_every_step_is_done() {
    let flow = flow();
    let mut state = RunState::new(&flow, "plan", "p");
    for step in &flow.steps {
        state.set(&step.id, StepStatus::Done);
    }
    assert_eq!(flow.outcome(&state), Outcome::Done);
}

#[test]
fn cancel_keeps_ended_steps_and_ends_the_run() {
    let flow = flow();
    let mut state = RunState::new(&flow, "plan", "p");
    state.set("plan", StepStatus::Done);
    state.set("approve", StepStatus::Running);
    state.cancel();
    assert_eq!(state.status("plan"), StepStatus::Done);
    assert_eq!(state.status("approve"), StepStatus::Cancelled);
    assert_eq!(state.status("build"), StepStatus::Cancelled);
    assert_eq!(flow.outcome(&state), Outcome::Stopped);
}

#[test]
fn a_cancelled_step_waits_for_nothing() {
    let mut state = RunState::new(&flow(), "plan", "p");
    state.set("plan", StepStatus::Running);
    let waits = Some("The agent's first hook report".to_owned());
    state.steps.get_mut("plan").unwrap().waits_for = waits.clone();
    state.set("approve", StepStatus::Done);
    state.steps.get_mut("approve").unwrap().waits_for = waits.clone();
    state.cancel();
    assert_eq!(state.steps["plan"].waits_for, None);
    assert_eq!(
        state.steps["approve"].waits_for, waits,
        "an ended step keeps it"
    );
}

#[test]
fn run_state_round_trips_as_json_and_keeps_the_session() {
    let flow = flow();
    let mut state = RunState::new(&flow, "plan", "p");
    state.steps.get_mut("plan").unwrap().session = Some("s1".into());
    state.set("plan", StepStatus::Running);
    let text = serde_json::to_string(&state).unwrap();
    assert!(text.contains("\"status\":\"running\""));
    let back: RunState = serde_json::from_str(&text).unwrap();
    assert_eq!(back, state);
    assert_eq!(back.steps["plan"].session.as_deref(), Some("s1"));
}

fn save_run(data: &std::path::Path, name: &str, state: &RunState) -> std::path::PathBuf {
    let dir = data.join(RUN_DIR).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(STATE_FILE), serde_json::to_vec(state).unwrap()).unwrap();
    dir
}

#[test]
fn runs_lists_readable_runs_and_skips_damaged_ones() {
    let data = tempfile::tempdir().unwrap();
    assert!(control::runs(data.path()).is_empty());
    let state = RunState::new(&flow(), "plan", "p");
    let good = save_run(data.path(), "a-1", &state);
    let damaged = data.path().join(RUN_DIR).join("b-2");
    std::fs::create_dir_all(&damaged).unwrap();
    std::fs::write(damaged.join(STATE_FILE), "{").unwrap();
    let runs = control::runs(data.path());
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0, good);
    assert_eq!(runs[0].1, state);
}

#[test]
fn cancel_and_answers_are_refused_for_an_ended_run() {
    let data = tempfile::tempdir().unwrap();
    let mut state = RunState::new(&flow(), "plan", "p");
    state.set("approve", StepStatus::Running);
    let dir = save_run(data.path(), "a-1", &state);
    let _runner = control::hold_runner(&dir).unwrap();
    control::cancel(&dir).unwrap();
    assert!(dir.join(control::CANCEL).exists());
    state.ended = true;
    let ended = save_run(data.path(), "a-2", &state);
    assert!(matches!(
        control::cancel(&ended),
        Err(control::RunError::Ended)
    ));
    assert!(matches!(
        control::answer_gate(&ended, "approve", control::GateAnswer::Approve),
        Err(control::RunError::Ended)
    ));
    assert!(!ended.join(control::CANCEL).exists());
}

#[test]
fn recent_runs_reads_the_newest_folders_and_every_run_that_did_not_end() {
    let data = tempfile::tempdir().unwrap();
    let mut state = RunState::new(&flow(), "plan", "p");
    let age = |dir: &std::path::Path, secs| {
        let time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
        std::fs::File::options()
            .write(true)
            .open(dir.join(STATE_FILE))
            .unwrap()
            .set_modified(time)
            .unwrap();
    };
    // An old run that did not end (a gate can wait for days) and an old
    // ended run.
    let open = save_run(data.path(), "a-1", &state);
    age(&open, 1_000);
    state.ended = true;
    let old = save_run(data.path(), "a-2", &state);
    age(&old, 2_000);
    let new = save_run(data.path(), "a-3", &state);
    let mut ended = control::EndedRuns::new();
    let dirs = |runs: Vec<(std::path::PathBuf, RunState)>| -> Vec<_> {
        runs.into_iter().map(|(dir, _)| dir).collect()
    };
    let recent = control::recent_runs(data.path(), 1, &mut ended);
    assert_eq!(dirs(recent), [new.clone(), open.clone()]);
    assert_eq!(ended.keys().collect::<Vec<_>>(), [&old]);
    // The next read skips the known ended run without reading it.
    std::fs::write(old.join(STATE_FILE), "{").unwrap();
    age(&old, 2_000);
    let recent = control::recent_runs(data.path(), 1, &mut ended);
    assert_eq!(dirs(recent), [new, open]);
    assert_eq!(
        control::runs(data.path()).len(),
        2,
        "a damaged file is skipped"
    );
}

#[test]
fn remove_deletes_only_an_ended_run_folder() {
    let data = tempfile::tempdir().unwrap();
    let mut state = RunState::new(&flow(), "plan", "p");
    let running = save_run(data.path(), "a-1", &state);
    let runner = control::hold_runner(&running).unwrap();
    assert!(matches!(
        control::remove(data.path(), &running),
        Err(control::RunError::NotEnded)
    ));
    assert!(running.join(STATE_FILE).is_file(), "a running run stays");
    drop(runner);
    state.ended = true;
    let ended = save_run(data.path(), "a-2", &state);
    std::fs::create_dir_all(ended.join(control::ANSWERS)).unwrap();
    control::remove(data.path(), &ended).unwrap();
    assert!(!ended.exists());
    // The runner of the first run is gone now: that run can go too.
    control::remove(data.path(), &running).unwrap();
    assert!(control::runs(data.path()).is_empty());
}

#[test]
fn a_run_whose_runner_is_gone_takes_no_answer_and_can_be_removed() {
    let data = tempfile::tempdir().unwrap();
    let mut state = RunState::new(&flow(), "plan", "p");
    state.set("approve", StepStatus::Running);
    let dir = save_run(data.path(), "a-1", &state);
    let status = || control::read_run(&dir).unwrap().status;
    // No lock file: no runner can be seen.
    assert!(!control::alive(&dir));
    assert_eq!(status(), control::RunStatus::RunnerGone);
    let runner = control::hold_runner(&dir).unwrap();
    assert!(control::alive(&dir), "the runner holds its lock");
    assert!(control::alive(&dir), "a check does not take the lock away");
    assert_eq!(status(), control::RunStatus::Running);
    // The OS lets the lock go when the runner ends in any way.
    drop(runner);
    let view = control::read_run(&dir).unwrap();
    assert_eq!(view.status, control::RunStatus::RunnerGone);
    assert_eq!(view.status.label(), "stopped (runner gone)");
    // Its steps that did not end show as cancelled; the file keeps them.
    assert_eq!(view.state.status("approve"), StepStatus::Cancelled);
    assert_eq!(
        control::read_state(&dir).unwrap().status("approve"),
        StepStatus::Running
    );
    let gone = |result: Result<(), control::RunError>| {
        matches!(result, Err(control::RunError::RunnerGone))
    };
    assert!(gone(control::cancel(&dir)));
    assert!(gone(control::answer_gate(
        &dir,
        "approve",
        control::GateAnswer::Approve
    )));
    std::fs::create_dir_all(dir.join(control::QUESTIONS)).unwrap();
    std::fs::write(dir.join(control::QUESTIONS).join("plan"), "A?").unwrap();
    assert!(gone(control::answer_question(&dir, "plan", "A")));
    assert!(!dir.join(control::CANCEL).exists());
    assert!(!dir.join(control::ANSWERS).exists());
    control::remove(data.path(), &dir).unwrap();
    assert!(!dir.exists());
    // An ended run is never gone: it ended.
    state.ended = true;
    let ended = save_run(data.path(), "a-2", &state);
    assert_eq!(
        control::read_run(&ended).unwrap().status,
        control::RunStatus::Stopped
    );
    for step in &flow().steps {
        state.set(&step.id, StepStatus::Done);
    }
    assert_eq!(
        control::RunStatus::of(&state, false),
        control::RunStatus::Done
    );
}

#[test]
fn abort_fails_running_steps_and_cancels_the_rest() {
    let mut state = RunState::new(&flow(), "plan", "p");
    state.set("plan", StepStatus::Done);
    state.set("approve", StepStatus::Running);
    state.steps.get_mut("approve").unwrap().question = Some("A?".into());
    state.abort("The runner stopped: disk full");
    assert_eq!(state.status("plan"), StepStatus::Done);
    assert_eq!(state.status("approve"), StepStatus::Failed);
    assert_eq!(
        state.steps["approve"].note.as_deref(),
        Some("The runner stopped: disk full")
    );
    assert_eq!(state.steps["approve"].question, None);
    assert_eq!(state.status("build"), StepStatus::Cancelled);
    assert_eq!(flow().outcome(&state), Outcome::Stopped);
}

#[test]
fn run_state_keeps_its_flow_file_and_a_state_without_one_is_unreadable() {
    let state = RunState::new(&flow(), "plan", "p");
    let text = serde_json::to_string(&state).unwrap();
    assert!(text.contains("\"file\":\"plan\""), "{text}");
    assert_eq!(serde_json::from_str::<RunState>(&text).unwrap(), state);
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value.as_object_mut().unwrap().remove("file");
    assert!(serde_json::from_value::<RunState>(value).is_err());
}

#[test]
fn run_order_goes_one_depth_at_a_time_in_file_order() {
    let at = |order: Vec<Ordered>| -> Vec<(usize, Option<usize>)> {
        order.into_iter().map(|o| (o.index, o.depth)).collect()
    };
    // plan, approve, build, docs: build and docs both need approve.
    assert_eq!(
        at(flow().run_order()),
        [(0, Some(0)), (1, Some(1)), (2, Some(2)), (3, Some(2))]
    );
    // File order differs from run order; a step that needs two steps
    // waits for the deeper one.
    let draft = Draft::new(
        "n",
        vec![
            draft_step("c", &["b", "a"]),
            draft_step("b", &["a"]),
            draft_step("a", &[]),
            draft_step("d", &[]),
        ],
    );
    assert_eq!(
        at(draft.run_order()),
        [(2, Some(0)), (3, Some(0)), (1, Some(1)), (0, Some(2))]
    );
    // A cycle, a step after it and an unknown need come last, in file order.
    let draft = Draft::new(
        "n",
        vec![
            draft_step("x", &["y"]),
            draft_step("a", &[]),
            draft_step("y", &["x"]),
            draft_step("z", &["y"]),
            draft_step("u", &["missing"]),
            draft_step("s", &["s"]),
        ],
    );
    assert_eq!(
        at(draft.run_order()),
        [
            (1, Some(0)),
            (0, None),
            (2, None),
            (3, None),
            (4, None),
            (5, None)
        ]
    );
}

fn draft_step(id: &str, needs: &[&str]) -> DraftStep {
    DraftStep {
        id: id.into(),
        needs: needs.iter().map(|&needs| needs.to_owned()).collect(),
        command: Some("x".into()),
        ..DraftStep::default()
    }
}

#[test]
fn project_folders_stay_inside_the_project() {
    for inside in ["", ".", "a", "a/b", "./a/./b"] {
        assert!(inside_project(Path::new(inside)), "{inside}");
    }
    for outside in ["..", "../a", "a/../b", "/etc"] {
        assert!(!inside_project(Path::new(outside)), "{outside}");
    }
}

#[test]
fn two_steps_with_one_worktree_name_are_refused() {
    let two = one_step(
        "id = \"a\"\ncommand = \"x\"\nworktree = \"w\"\n[[step]]\nid = \"b\"\ncommand = \"y\"\nworktree = \"W\"",
    );
    assert_eq!(
        two,
        Err(FlowError::DuplicateWorktree {
            step: "b".into(),
            first: "a".into(),
            name: "W".into(),
        })
    );
    let other = one_step(
        "id = \"a\"\ncommand = \"x\"\nworktree = \"w\"\n[[step]]\nid = \"b\"\ncommand = \"y\"\nworktree = \"v\"",
    );
    assert!(other.is_ok());
}

#[test]
fn remove_refuses_folders_outside_the_run_folder() {
    let data = tempfile::tempdir().unwrap();
    let mut state = RunState::new(&flow(), "plan", "p");
    state.ended = true;
    save_run(data.path(), "a-1", &state);
    let not_run = |dir: &std::path::Path| {
        matches!(
            control::remove(data.path(), dir),
            Err(control::RunError::NotRun | control::RunError::Io(_))
        )
    };
    // A run folder of another data folder.
    let other = tempfile::tempdir().unwrap();
    let elsewhere = save_run(other.path(), "a-1", &state);
    assert!(not_run(&elsewhere));
    assert!(elsewhere.is_dir());
    // Path tricks: a parent step, a nested folder, the run folder itself.
    assert!(not_run(&data.path().join(RUN_DIR).join("a-1/..")));
    assert!(not_run(&data.path().join(RUN_DIR).join("..").join("x")));
    let nested = save_run(&data.path().join(RUN_DIR).join("a-1"), "b-1", &state);
    assert!(not_run(&nested));
    assert!(not_run(&data.path().join(RUN_DIR)));
    // A folder without a readable state file.
    let empty = data.path().join(RUN_DIR).join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(control::remove(data.path(), &empty).is_err());
    assert!(empty.is_dir());
    assert!(data.path().join(RUN_DIR).join("a-1").is_dir());
}

#[cfg(unix)]
#[test]
fn remove_never_follows_a_link_out_of_the_run_folder() {
    let data = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let mut state = RunState::new(&flow(), "plan", "p");
    state.ended = true;
    let target = save_run(outside.path(), "t-1", &state);
    std::fs::create_dir_all(data.path().join(RUN_DIR)).unwrap();
    let link = data.path().join(RUN_DIR).join("t-1");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(matches!(
        control::remove(data.path(), &link),
        Err(control::RunError::NotRun)
    ));
    assert!(target.join(STATE_FILE).is_file());
    // A link inside an ended run is deleted, not its target.
    let dir = save_run(data.path(), "a-1", &state);
    std::os::unix::fs::symlink(&target, dir.join("inside")).unwrap();
    control::remove(data.path(), &dir).unwrap();
    assert!(!dir.exists());
    assert!(target.join(STATE_FILE).is_file());
}

#[test]
fn question_answer_needs_an_open_question_and_bounded_text() {
    let data = tempfile::tempdir().unwrap();
    let dir = save_run(data.path(), "a-1", &RunState::new(&flow(), "plan", "p"));
    let _runner = control::hold_runner(&dir).unwrap();
    assert!(matches!(
        control::answer_question(&dir, "plan", "yes"),
        Err(control::RunError::NoQuestion)
    ));
    assert!(matches!(
        control::answer_question(&dir, "../x", "yes"),
        Err(control::RunError::UnknownStep)
    ));
    std::fs::create_dir_all(dir.join(control::QUESTIONS)).unwrap();
    std::fs::write(dir.join(control::QUESTIONS).join("plan"), "A or B?").unwrap();
    assert_eq!(control::question(&dir, "plan").as_deref(), Some("A or B?"));
    assert!(matches!(
        control::answer_question(&dir, "plan", "  "),
        Err(control::RunError::Text)
    ));
    control::answer_question(&dir, "plan", " A ").unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join(control::REPLIES).join("plan")).unwrap(),
        "A"
    );
}

#[test]
fn flow_files_are_found_by_a_plain_name() {
    let project = tempfile::tempdir().unwrap();
    let folder = project.path().join(FLOW_DIR);
    std::fs::create_dir_all(&folder).unwrap();
    assert!(files::names(project.path()).is_empty());
    assert!(matches!(
        files::load(project.path(), "plan"),
        Err(files::LoadError::Missing(_))
    ));
    std::fs::write(folder.join("plan.toml"), PLAN_AND_BUILD).unwrap();
    std::fs::write(folder.join("notes.md"), "no flow").unwrap();
    assert_eq!(files::names(project.path()), ["plan"]);
    assert_eq!(files::load(project.path(), "plan").unwrap(), flow());
    // A name is never a path.
    assert!(matches!(
        files::load(project.path(), "../plan"),
        Err(files::LoadError::Name)
    ));
}

#[test]
fn draft_writes_a_file_that_reads_as_the_same_flow() {
    let flow = flow();
    let draft = Draft::from(&flow);
    let text = draft.to_toml().unwrap();
    assert_eq!(parse(&text).unwrap(), flow);
    // A multi-line prompt and the editor position survive.
    let mut draft = draft;
    draft.steps[0].prompt = Some("Line 1.\nLine \"2\".".into());
    draft.steps[0].at = Some([320, -40]);
    let back = parse(&draft.to_toml().unwrap()).unwrap();
    assert_eq!(back.steps[0].at, Some([320, -40]));
    assert_eq!(Draft::from(&back), draft);
}

#[test]
fn flow_rules_go_before_each_prompt() {
    let mut draft = Draft::from(&flow());
    assert_eq!(flow().full_prompt("Do it."), "Do it.");
    // The text is trimmed; an empty text is no rules.
    draft.rules = Some("   ".into());
    assert_eq!(draft.clone().check().unwrap().rules, None);
    draft.rules = Some("  Obey AGENTS.md.\n".into());
    let back = parse(&draft.to_toml().unwrap()).unwrap();
    assert_eq!(back.rules.as_deref(), Some("Obey AGENTS.md."));
    assert_eq!(back.full_prompt("Do it."), "Obey AGENTS.md.\n\nDo it.");
    draft.rules = Some("bad \u{7} text".into());
    assert_eq!(draft.check(), Err(FlowError::Rules));
}

#[test]
fn draft_that_is_not_a_correct_flow_gives_no_file_text() {
    let mut draft = Draft::from(&flow());
    draft.steps[1].needs = vec!["missing".into()];
    assert!(matches!(
        draft.to_toml(),
        Err(FlowError::UnknownNeed { .. })
    ));
    assert_eq!(Draft::new("n", Vec::new()).to_toml(), Err(FlowError::Empty));
    let step = DraftStep {
        id: "a".into(),
        ..DraftStep::default()
    };
    assert_eq!(
        Draft::new("n", vec![step]).to_toml(),
        Err(FlowError::Kind("a".into()))
    );
}

#[test]
fn save_writes_only_a_correct_flow_and_replaces_only_when_told() {
    let project = tempfile::tempdir().unwrap();
    let draft = Draft::from(&flow());
    let path = files::save(project.path(), "plan", &draft, false).unwrap();
    assert_eq!(path, project.path().join(FLOW_DIR).join("plan.toml"));
    assert_eq!(files::load(project.path(), "plan").unwrap(), flow());
    // A new flow never replaces a file.
    assert!(matches!(
        files::save(project.path(), "plan", &draft, false),
        Err(files::LoadError::Exists(_))
    ));
    // Save of an open flow replaces its own file.
    let mut changed = draft.clone();
    changed.name = "Changed".into();
    files::save(project.path(), "plan", &changed, true).unwrap();
    assert_eq!(files::load(project.path(), "plan").unwrap().name, "Changed");
    // A wrong draft or a bad name writes nothing.
    changed.steps.clear();
    assert!(files::save(project.path(), "plan", &changed, true).is_err());
    assert_eq!(files::load(project.path(), "plan").unwrap().name, "Changed");
    assert!(matches!(
        files::save(project.path(), "../x", &draft, false),
        Err(files::LoadError::Name)
    ));
}
