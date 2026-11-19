//! Run inputs and Retry: `--set` values (shell-quoted in a command), a
//! missing required input that refuses the run before anything starts,
//! and a failed step that the waiting runner starts again as attempt 2 in
//! a new terminal while the old terminal stays.
use super::{Case, bin, id, is, read_state, until, wait_child};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{fs, time::Duration};
use terminator_core::{
    flow::{FLOW_DIR, RUN_DIR, control},
    quote,
};

/// The first attempt of `flaky` fails; the next one is done with the
/// `word` input as its note.
const FLOW: &str = r#"version = 1
name = "Retry"

[inputs.word]
description = "A word for the note"

[inputs.mark]
default = "tried"

[[step]]
id = "flaky"
command = "if [ -f {{inputs.mark}} ]; then HOOK flow report done --note {{inputs.word}}; else touch {{inputs.mark}}; HOOK flow report failed --note first; fi"

[[step]]
id = "after"
needs = ["flaky"]
command = "HOOK flow report done"
"#;

/// A value with shell syntax: it must arrive as text and run nothing.
const WORD: &str = "a b'; touch injected; echo '$(touch injected)";

impl Case<'_> {
    fn session_count(&self) -> Result<usize> {
        Ok(self
            .h
            .state()?
            .get("sessions")
            .and_then(Value::as_array)
            .map_or(0, Vec::len))
    }

    /// A required input with no value, or an unknown one, refuses the run:
    /// no run folder and no terminal.
    pub(super) fn inputs_refuse_a_run_without_a_required_value(&self) -> Result<()> {
        let hook = quote(&bin().join("terminator-hook").to_string_lossy());
        let text = FLOW.replace("HOOK", &hook);
        fs::write(self.root.join(FLOW_DIR).join("retry.toml"), text)?;
        let before = self.session_count()?;
        let project = id(&self.project);
        for more in [&[][..], &["--set", "word=x", "--set", "nope=1"]] {
            let mut args = vec!["run", project, "retry", "--run", "retry-0"];
            args.extend_from_slice(more);
            ensure!(self.flow(&args).is_err(), "A run took {more:?}");
        }
        ensure!(
            !self.h.root.join(RUN_DIR).join("retry-0").exists(),
            "A refused run made a run folder"
        );
        ensure!(
            self.session_count()? == before,
            "A refused run made a terminal"
        );
        Ok(())
    }

    /// The runner waits after the failure; Retry starts attempt 2 in a new
    /// terminal, the step after it runs, and the run ends as done.
    pub(super) fn retry_runs_a_new_attempt_with_inputs(&self) -> Result<()> {
        let set = format!("word={WORD}");
        let more = ["--set", set.as_str()];
        let (mut runner, dir) = self.spawn_run_with("retry", ("retry-1", &more), |state| {
            is(state, "/steps/flaky/status", "failed")
        })?;
        let first = read_state(&dir)?
            .pointer("/steps/flaky/session")
            .and_then(Value::as_str)
            .context("no first session")?
            .to_owned();
        let shown: Value = serde_json::from_slice(&self.flow(&["status", "retry-1"])?)?;
        ensure!(
            is(&shown, "/status", "running")
                && shown.get("waits_for_retry") == Some(&Value::Bool(true)),
            "The runner did not wait for Retry: {shown}"
        );
        ensure!(
            self.flow(&["retry", "retry-1", "after"]).is_err(),
            "A step that did not fail took a retry"
        );
        self.flow(&["retry", "retry-1", "flaky"])?;
        let status = wait_child(&mut runner.0, Duration::from_mins(1))?;
        ensure!(
            status.success(),
            "The retried run did not end as done: {status}"
        );
        let state = read_state(&dir)?;
        ensure!(
            is(&state, "/steps/flaky/status", "done")
                && state.pointer("/steps/flaky/attempt") == Some(&Value::from(2))
                && is(&state, "/steps/flaky/note", WORD)
                && is(&state, "/steps/after/status", "done")
                && is(&state, "/inputs/word", WORD)
                && is(&state, "/inputs/mark", "tried"),
            "{state}"
        );
        let second = state
            .pointer("/steps/flaky/session")
            .and_then(Value::as_str);
        ensure!(
            second.is_some_and(|second| second != first),
            "Attempt 2 used the old terminal"
        );
        ensure!(
            !self.root.join("injected").exists(),
            "An input value ran as shell syntax"
        );
        // The first attempt's terminal stays: Retry closes nothing.
        until(5, "the first attempt's terminal", || {
            let live = self
                .h
                .state()?
                .get("sessions")
                .and_then(Value::as_array)
                .is_some_and(|all| all.iter().any(|s| id(s) == first));
            Ok(live.then_some(()))
        })?;
        ensure!(
            control::retry(&dir, "flaky").is_err(),
            "An ended run took a retry"
        );
        Ok(())
    }
}
