use super::*;
use std::collections::BTreeMap;

const WITH_INPUTS: &str = r#"
version = 1
name = "Fix"
rules = "Work on issue {{inputs.issue}}."

[inputs.issue]
description = "Issue number"

[inputs.branch]
default = "main"

[[step]]
id = "fix"
command = "claude --name {{inputs.branch}}"
prompt = "Fix issue {{inputs.issue}} on {{inputs.branch}}."

[[step]]
id = "ok"
gate = "Is issue {{inputs.issue}} fixed?"
needs = ["fix"]
"#;

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn sets(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    values(pairs).into_iter().collect()
}

/// A one-step flow with these inputs and this command.
fn command_flow(command: &str) -> Result<Flow, FlowError> {
    parse(&format!(
        "version = 1\nname = \"n\"\n[inputs.v]\n[[step]]\nid = \"s\"\ncommand = '''{command}'''\n"
    ))
}

#[test]
fn declared_inputs_fill_prompt_command_gate_and_rules() {
    let flow = parse(WITH_INPUTS).unwrap();
    assert!(flow.inputs["issue"].required());
    assert!(!flow.inputs["branch"].required());
    let values = flow.input_values(&sets(&[("issue", "42")])).unwrap();
    assert_eq!(values, self::values(&[("branch", "main"), ("issue", "42")]));
    let run = flow.with_values(&values).unwrap();
    assert_eq!(run.rules.as_deref(), Some("Work on issue 42."));
    let StepKind::Command {
        command, prompt, ..
    } = &run.step("fix").unwrap().kind
    else {
        panic!("not a command");
    };
    assert_eq!(command, "claude --name 'main'");
    assert_eq!(prompt.as_deref(), Some("Fix issue 42 on main."));
    assert_eq!(
        run.step("ok").unwrap().kind,
        StepKind::Gate {
            message: "Is issue 42 fixed?".into()
        }
    );
}

#[test]
fn undeclared_input_or_unknown_placeholder_is_a_file_error() {
    let error = one_command("echo {{inputs.nope}}").unwrap_err();
    assert!(
        matches!(&error, FlowError::Placeholder { step: Some(step), field: "command", .. } if step == "s"),
        "{error}"
    );
    assert!(error.to_string().contains("nope"), "{error}");
    for text in ["echo {{name}}", "echo {{ inputs.v }}", "echo {{inputs.v"] {
        assert!(command_flow(text).is_err(), "{text}");
    }
    let rules = "version = 1\nname = \"n\"\nrules = \"{{inputs.x}}\"\n[[step]]\nid = \"s\"\ncommand = \"c\"\n";
    assert!(matches!(
        parse(rules),
        Err(FlowError::Placeholder {
            step: None,
            field: "rules",
            ..
        })
    ));
}

fn one_command(command: &str) -> Result<Flow, FlowError> {
    parse(&format!(
        "version = 1\nname = \"n\"\n[[step]]\nid = \"s\"\ncommand = '''{command}'''\n"
    ))
}

#[test]
fn input_names_follow_the_step_id_rules_and_text_is_bounded() {
    for name in ["Issue", "\"a b\"", "\"a.b\"", "\"\""] {
        let text = format!(
            "version = 1\nname = \"n\"\n[inputs.{name}]\n[[step]]\nid = \"s\"\ncommand = \"c\"\n"
        );
        assert!(
            matches!(parse(&text), Err(FlowError::BadInput(_))),
            "{name}"
        );
    }
    let bad_default = "version = 1\nname = \"n\"\n[inputs.v]\ndefault = \"a\\nb\"\n[[step]]\nid = \"s\"\ncommand = \"c\"\n";
    assert_eq!(
        parse(bad_default),
        Err(FlowError::InputText("v".into(), "default"))
    );
    let unknown_key = "version = 1\nname = \"n\"\n[inputs.v]\nrequired = true\n[[step]]\nid = \"s\"\ncommand = \"c\"\n";
    assert!(matches!(parse(unknown_key), Err(FlowError::Syntax(_))));
}

#[test]
fn missing_unknown_twice_or_bad_values_refuse_the_run() {
    let flow = parse(WITH_INPUTS).unwrap();
    assert_eq!(
        flow.input_values(&[]),
        Err(InputError::Missing("issue".into()))
    );
    assert_eq!(
        flow.input_values(&sets(&[("issue", "1"), ("nope", "2")])),
        Err(InputError::Unknown("nope".into()))
    );
    let twice = vec![("issue".into(), "1".into()), ("issue".into(), "2".into())];
    assert_eq!(
        flow.input_values(&twice),
        Err(InputError::Twice("issue".into()))
    );
    assert_eq!(
        flow.input_values(&sets(&[("issue", "1\n2")])),
        Err(InputError::Value("issue".into()))
    );
    // An empty value is a value; a set value wins over the default.
    let values = flow
        .input_values(&sets(&[("issue", ""), ("branch", "dev")]))
        .unwrap();
    assert_eq!(values["branch"], "dev");
}

#[test]
fn set_takes_name_equals_value() {
    assert_eq!(
        inputs::parse_set("issue=4=2").unwrap(),
        ("issue".into(), "4=2".into())
    );
    assert_eq!(inputs::parse_set("issue=").unwrap().1, "");
    assert!(inputs::parse_set("issue").is_err());
    assert!(inputs::parse_set("=1").is_err());
}

/// `command` with `value` for `{{inputs.v}}`, as a step types it.
fn filled(command: &str, value: &str) -> String {
    let flow = command_flow(command).unwrap();
    let run = flow.with_values(&values(&[("v", value)])).unwrap();
    let StepKind::Command { command, .. } = &run.steps[0].kind else {
        panic!("not a command");
    };
    command.clone()
}

#[test]
fn value_in_a_command_is_one_word_and_never_shell_syntax() {
    let dir = tempfile::tempdir().unwrap();
    let nasty = [
        "plain",
        "",
        "two words",
        "it's",
        "a'; touch injected; echo '",
        "\"; touch injected; echo \"",
        "$(touch injected)",
        "`touch injected`",
        "${HOME} $HOME * ?",
        "back\\slash \\' \\\"",
    ];
    // Each command prints its words in brackets on one line. One shell runs
    // all of them: a test that forks often holds other tests' locks.
    let commands = [
        // Plain words: one argument.
        ("printf '[%s]' {{inputs.v}}", "[V]"),
        // In double quotes, beside other text.
        ("printf '[%s]' \"a {{inputs.v}} b\"", "[a V b]"),
        // In single quotes.
        ("printf '[%s]' 'a {{inputs.v}} b'", "[a V b]"),
        // In a command substitution in double quotes.
        ("printf '[%s]' \"$(printf %s \"{{inputs.v}}\")\"", "[V]"),
    ];
    let mut script = String::new();
    let mut want = String::new();
    for value in nasty {
        for (command, shape) in commands {
            script.push_str(&format!("{}; echo\n", filled(command, value)));
            want.push_str(&format!("{}\n", shape.replace('V', value)));
        }
    }
    let output = std::process::Command::new("sh")
        .args(["-c", &script])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap(), want);
    assert!(
        !dir.path().join("injected").exists(),
        "a value ran as shell"
    );
}

#[test]
fn input_in_backticks_braces_or_ansi_quotes_is_refused() {
    for command in [
        "echo `x {{inputs.v}}`",
        "echo ${HOME:-{{inputs.v}}}",
        "echo $'a {{inputs.v}}'",
        "echo \"`{{inputs.v}}`\"",
    ] {
        assert!(
            matches!(command_flow(command), Err(FlowError::Placeholder { .. })),
            "{command}"
        );
    }
    // A closed construct before the input does not count.
    assert!(command_flow("echo `pwd` $'x' ${HOME} {{inputs.v}}").is_ok());
}

#[test]
fn draft_with_inputs_writes_a_file_that_reads_as_the_same_flow() {
    let flow = parse(WITH_INPUTS).unwrap();
    let text = Draft::from(&flow).to_toml().unwrap();
    assert!(text.contains("[inputs.issue]"), "{text}");
    assert_eq!(parse(&text).unwrap(), flow);
}

fn failed_run() -> RunState {
    let text = "version = 1\nname = \"n\"\n[[step]]\nid = \"a\"\ncommand = \"c\"\n[[step]]\nid = \"b\"\ncommand = \"c\"\nneeds = [\"a\"]\n[[step]]\nid = \"c\"\ncommand = \"c\"\nneeds = [\"b\"]\n[[step]]\nid = \"free\"\ncommand = \"c\"\n";
    let mut state = RunState::new(&parse(text).unwrap(), "n", "p");
    state.set("free", StepStatus::Done);
    state.fail("a", "broke");
    let a = state.steps.get_mut("a").unwrap();
    a.session = Some("s1".into());
    a.exited = true;
    state
}

#[test]
fn retry_starts_a_new_attempt_and_resets_the_steps_after_it() {
    let mut state = failed_run();
    assert!(state.held(), "a failed run with nothing to start is held");
    state.set("c", StepStatus::Cancelled);
    assert!(!state.retry("free"), "a done step is not retried");
    assert!(state.retry("a"));
    let a = &state.steps["a"];
    assert_eq!(
        (a.status, a.attempt(), a.session.as_ref(), a.note.as_ref()),
        (StepStatus::Waiting, 2, None, None)
    );
    assert!(!a.exited);
    assert_eq!(state.status("b"), StepStatus::Waiting);
    assert_eq!(state.status("c"), StepStatus::Waiting);
    assert_eq!(state.status("free"), StepStatus::Done);
    assert!(!state.held());
    // The attempt is kept in the state file.
    let text = serde_json::to_string(&state).unwrap();
    let read: RunState = serde_json::from_str(&text).unwrap();
    assert_eq!(read.steps["a"].attempt(), 2);
}

#[test]
fn retry_is_refused_unless_the_runner_lives_and_the_step_failed() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join(RUN_DIR).join("r-1");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(STATE_FILE),
        serde_json::to_vec(&failed_run()).unwrap(),
    )
    .unwrap();
    // No runner: nothing would read the request.
    assert!(matches!(
        control::retry(&dir, "a"),
        Err(control::RunError::RunnerGone)
    ));
    let runner = control::hold_runner(&dir).unwrap();
    assert!(matches!(
        control::retry(&dir, "free"),
        Err(control::RunError::NotRetryable)
    ));
    assert!(matches!(
        control::retry(&dir, "../x"),
        Err(control::RunError::UnknownStep)
    ));
    control::retry(&dir, "a").unwrap();
    let asked = std::fs::read_to_string(dir.join(control::RETRIES).join("a")).unwrap();
    assert_eq!(asked, "2");
    drop(runner);
}
