use super::{control::GateAnswer, *};

/// build, then a review gate: ship goes to `release`, rework goes to `fix`,
/// which runs the loop again from `build` (3 rounds at most). `report`
/// needs both branches.
const REVIEW: &str = r#"
version = 1
name = "Review"

[[step]]
id = "build"
command = "claude"

[[step]]
id = "review"
gate = "Ship it?"
needs = ["build"]
options = ["ship", "rework"]

[[step]]
id = "release"
command = "echo release"
needs = ["review"]
when = "review.ship"

[[step]]
id = "fix"
command = "echo fix"
needs = ["review"]
when = "review.rework"
loop_to = "build"
max_rounds = 3

[[step]]
id = "report"
command = "echo report"
needs = ["release", "fix"]
"#;

fn review() -> Flow {
    parse(REVIEW).unwrap()
}

fn with(lines: &str) -> Result<Flow, FlowError> {
    parse(&format!(
        "version = 1\nname = \"n\"\n[[step]]\nid = \"a\"\ncommand = \"x\"\n\
         [[step]]\nid = \"g\"\ngate = \"ok?\"\nneeds = [\"a\"]\noptions = [\"yes\", \"no\"]\n\
         [[step]]\nid = \"b\"\ncommand = \"y\"\nneeds = [\"g\"]\n{lines}\n"
    ))
}

#[test]
fn gate_options_when_and_loop_are_read_and_written_back() {
    let flow = review();
    let StepKind::Gate { options, .. } = &flow.step("review").unwrap().kind else {
        panic!("not a gate");
    };
    assert_eq!(options, &["ship", "rework"]);
    let fix = flow.step("fix").unwrap();
    assert_eq!(fix.when.as_ref().unwrap().to_string(), "review.rework");
    assert_eq!(
        fix.loop_back,
        Some(LoopBack {
            to: "build".into(),
            max_rounds: 3
        })
    );
    let text = Draft::from(&flow).to_toml().unwrap();
    assert_eq!(parse(&text).unwrap(), flow);
}

#[test]
fn bad_gate_options_are_refused() {
    for options in [
        r#"["one"]"#,
        r#"["a", "a"]"#,
        r#"["Ship", "no"]"#,
        r#"["1", "2", "3", "4", "5", "6", "7", "8", "9"]"#,
    ] {
        let text = format!(
            "version = 1\nname = \"n\"\n[[step]]\nid = \"g\"\ngate = \"ok?\"\noptions = {options}\n"
        );
        assert_eq!(
            parse(&text),
            Err(FlowError::Options("g".into())),
            "{options}"
        );
    }
    let command = "version = 1\nname = \"n\"\n[[step]]\nid = \"c\"\ncommand = \"x\"\noptions = [\"a\", \"b\"]\n";
    assert_eq!(parse(command), Err(FlowError::Options("c".into())));
}

#[test]
fn when_must_name_a_needed_gate_and_one_of_its_options() {
    assert!(with("when = \"g.yes\"").is_ok());
    for (when, part) in [
        ("g.maybe", "not one of yes, no"),
        ("a.yes", "must be in needs"),
        ("g", "must be GATE.OPTION"),
    ] {
        let error = with(&format!("when = \"{when}\"")).unwrap_err().to_string();
        assert!(error.contains(part), "{when}: {error}");
    }
    let plain = parse(
        "version = 1\nname = \"n\"\n[[step]]\nid = \"g\"\ngate = \"ok?\"\n\
         [[step]]\nid = \"b\"\ncommand = \"x\"\nneeds = [\"g\"]\nwhen = \"g.y\"\n",
    )
    .unwrap_err();
    assert!(
        plain
            .to_string()
            .contains("must be a gate step with options"),
        "{plain}"
    );
    // The gate must be a direct need, not only an earlier step.
    let error = parse(
        "version = 1\nname = \"n\"\n[[step]]\nid = \"g\"\ngate = \"ok?\"\noptions = [\"y\", \"n\"]\n\
         [[step]]\nid = \"a\"\ncommand = \"x\"\nneeds = [\"g\"]\n\
         [[step]]\nid = \"b\"\ncommand = \"x\"\nneeds = [\"a\"]\nwhen = \"g.y\"\n",
    )
    .unwrap_err();
    assert!(error.to_string().contains("must be in needs"), "{error}");
}

#[test]
fn loop_must_go_back_to_an_earlier_step_with_bounded_rounds() {
    assert!(with("loop_to = \"a\"\nmax_rounds = 2").is_ok());
    for (lines, part) in [
        ("loop_to = \"a\"", "set together"),
        ("max_rounds = 3", "set together"),
        ("loop_to = \"a\"\nmax_rounds = 1", "from 2 to 10"),
        ("loop_to = \"a\"\nmax_rounds = 11", "from 2 to 10"),
        (
            "loop_to = \"b\"\nmax_rounds = 2",
            "must be a step that this step needs",
        ),
        (
            "loop_to = \"nope\"\nmax_rounds = 2",
            "must be a step that this step needs",
        ),
    ] {
        let error = with(lines).unwrap_err().to_string();
        assert!(error.contains(part), "{lines}: {error}");
    }
}

#[test]
fn two_loops_that_share_a_step_are_refused() {
    let error = parse(
        "version = 1\nname = \"n\"\n[[step]]\nid = \"a\"\ncommand = \"x\"\n\
         [[step]]\nid = \"b\"\ncommand = \"x\"\nneeds = [\"a\"]\nloop_to = \"a\"\nmax_rounds = 2\n\
         [[step]]\nid = \"c\"\ncommand = \"x\"\nneeds = [\"b\"]\nloop_to = \"a\"\nmax_rounds = 2\n",
    )
    .unwrap_err();
    assert!(error.to_string().contains("shares step"), "{error}");
}

#[test]
fn run_state_carries_options_conditions_and_rounds() {
    let state = RunState::new(&review(), "review", "p");
    assert_eq!(state.steps["review"].options, ["ship", "rework"]);
    assert_eq!(state.steps["fix"].when.as_deref(), Some("review.rework"));
    assert_eq!(state.steps["fix"].loop_to.as_deref(), Some("build"));
    for id in ["build", "review", "fix"] {
        assert_eq!(
            state.steps[id].round_label().as_deref(),
            Some("round 1 of 3"),
            "{id}"
        );
    }
    assert_eq!(state.steps["release"].round_label(), None);
}

/// `build` done and `review` answered with `option`.
fn chose(flow: &Flow, option: &str) -> RunState {
    let mut state = RunState::new(flow, "review", "p");
    state.set("build", StepStatus::Done);
    state.set("review", StepStatus::Done);
    state.steps.get_mut("review").unwrap().choice = Some(option.into());
    state
}

#[test]
fn branch_that_was_not_chosen_is_skipped_and_the_join_runs() {
    let flow = review();
    let mut state = chose(&flow, "ship");
    assert_eq!(flow.skip(&mut state), ["fix"]);
    let ready: Vec<_> = flow
        .ready(&state)
        .iter()
        .map(|step| step.id.clone())
        .collect();
    assert_eq!(ready, ["release"]);
    state.set("release", StepStatus::Done);
    // `report` needs one done and one skipped step: it runs.
    assert!(flow.skip(&mut state).is_empty());
    assert_eq!(flow.ready(&state)[0].id, "report");
    state.set("report", StepStatus::Done);
    assert_eq!(flow.outcome(&state), Outcome::Done);
    assert_eq!(
        control::RunStatus::of(
            &RunState {
                ended: true,
                ..state
            },
            false
        ),
        control::RunStatus::Done
    );
}

#[test]
fn step_that_needs_only_skipped_steps_is_skipped_too() {
    let mut flow = review();
    flow.steps.retain(|step| step.id != "release");
    flow.steps
        .iter_mut()
        .for_each(|step| step.needs.retain(|id| id != "release"));
    let mut state = chose(&flow, "ship");
    assert_eq!(flow.skip(&mut state), ["fix", "report"]);
    assert_eq!(flow.outcome(&state), Outcome::Done);
}

#[test]
fn loop_step_runs_the_loop_again_until_its_last_round() {
    let flow = review();
    let mut state = chose(&flow, "rework");
    assert_eq!(flow.skip(&mut state), ["release"]);
    state.set("fix", StepStatus::Done);
    state.steps.get_mut("fix").unwrap().note = Some("fix the tests".into());
    let mut again = flow.loop_back(&mut state, "fix");
    again.sort();
    assert_eq!(again, ["build", "fix", "release", "report", "review"]);
    for id in ["build", "review", "fix"] {
        let run = &state.steps[id];
        assert_eq!((run.status, run.round()), (StepStatus::Waiting, 2), "{id}");
        assert_eq!(run.choice, None);
    }
    assert_eq!(state.steps["release"].status, StepStatus::Waiting);
    assert_eq!(state.steps["release"].round_label(), None);
    assert_eq!(
        state.steps["build"].loop_note.as_deref(),
        Some("fix the tests")
    );
    assert_eq!(flow.ready(&state)[0].id, "build");
    // Rounds 2 and 3; after round 3 the loop step stays done.
    for round in [2, 3] {
        let mut state_round = chose(&flow, "rework");
        for id in ["build", "review", "fix"] {
            state_round.steps.get_mut(id).unwrap().round = round;
        }
        flow.skip(&mut state_round);
        state_round.set("fix", StepStatus::Done);
        let again = flow.loop_back(&mut state_round, "fix");
        assert_eq!(again.is_empty(), round == 3, "round {round}");
    }
}

#[test]
fn retry_keeps_the_round_and_the_gate_options() {
    let flow = review();
    let mut state = RunState::new(&flow, "review", "p");
    state.steps.get_mut("build").unwrap().round = 2;
    state.fail("build", "broke");
    assert!(state.retry("build"));
    let build = &state.steps["build"];
    assert_eq!((build.round(), build.attempt()), (2, 2));
    state.fail("review", "Rejected");
    assert!(state.retry("review"));
    assert_eq!(state.steps["review"].options, ["ship", "rework"]);
}

#[test]
fn gate_answer_text_round_trips() {
    for answer in [
        GateAnswer::Approve,
        GateAnswer::Reject,
        GateAnswer::Choose("ship"),
    ] {
        assert_eq!(GateAnswer::parse(&answer.text()), Some(answer));
    }
    assert_eq!(GateAnswer::parse("maybe"), None);
}
