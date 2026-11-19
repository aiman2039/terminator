//! Gate options, branches and a bounded loop: a gate with options refuses
//! a plain approve and an unknown option; "rework" runs the loop again
//! from `build` in new terminals; "ship" runs `release` and skips `fix`;
//! and a loop ends after its last round.
use super::{Case, bin, is, read_state, until, wait_child};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{fs, path::Path, time::Duration};
use terminator_core::{flow::FLOW_DIR, quote};

const FLOW: &str = r#"version = 1
name = "Review"

[[step]]
id = "build"
command = "HOOK flow report done --note built"

[[step]]
id = "review"
gate = "Ship it?"
needs = ["build"]
options = ["ship", "rework"]

[[step]]
id = "release"
command = "HOOK flow report done"
needs = ["review"]
when = "review.ship"

[[step]]
id = "fix"
command = "HOOK flow report done --note again"
needs = ["review"]
when = "review.rework"
loop_to = "build"
max_rounds = 2
"#;

/// The value at `pointer` of the run's state, as a number.
fn number(state: &Value, pointer: &str) -> Option<u64> {
    state.pointer(pointer).and_then(Value::as_u64)
}

impl Case<'_> {
    /// Waits until the review gate of `dir` waits in round `round`.
    fn review_waits(&self, dir: &Path, round: u64) -> Result<Value> {
        until(30, &format!("the review gate in round {round}"), || {
            let state = read_state(dir)?;
            let round_is = number(&state, "/steps/review/round").unwrap_or(1) == round;
            Ok((round_is && is(&state, "/steps/review/status", "running")).then_some(state))
        })
    }

    /// Rework runs the loop again in new terminals; then ship runs the
    /// release and skips the fix.
    pub(super) fn gate_option_and_loop(&self) -> Result<()> {
        let hook = quote(&bin().join("terminator-hook").to_string_lossy());
        let text = FLOW.replace("HOOK", &hook);
        fs::write(self.root.join(FLOW_DIR).join("review.toml"), text)?;
        let (mut runner, dir) = self.spawn_run("review", "review-1", |_| true)?;
        let first = self.review_waits(&dir, 1)?;
        ensure!(
            self.flow(&["approve", "review-1", "review"]).is_err(),
            "A gate with options took a plain approve"
        );
        ensure!(
            self.flow(&["approve", "review-1", "review", "--option", "maybe"])
                .is_err(),
            "A gate took an unknown option"
        );
        self.flow(&["approve", "review-1", "review", "--option", "rework"])?;
        let second = self.review_waits(&dir, 2)?;
        let session = |state: &Value| {
            state
                .pointer("/steps/build/session")
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        ensure!(
            session(&first).is_some() && session(&first) != session(&second),
            "Round 2 of build used the old terminal"
        );
        ensure!(
            number(&second, "/steps/build/round") == Some(2)
                && is(&second, "/steps/build/status", "done")
                && is(&second, "/steps/release/status", "waiting"),
            "{second}"
        );
        self.flow(&["approve", "review-1", "review", "--option", "ship"])?;
        let status = wait_child(&mut runner.0, Duration::from_mins(1))?;
        ensure!(status.success(), "The run did not end as done: {status}");
        let state = read_state(&dir)?;
        ensure!(
            is(&state, "/steps/review/choice", "ship")
                && is(&state, "/steps/release/status", "done")
                && is(&state, "/steps/fix/status", "skipped"),
            "{state}"
        );
        Ok(())
    }

    /// Rework in every round: after its last round the fix stays done, the
    /// release is skipped, and the run ends as done.
    pub(super) fn loop_ends_after_its_last_round(&self) -> Result<()> {
        let (mut runner, dir) = self.spawn_run("review", "review-2", |_| true)?;
        for round in [1, 2] {
            self.review_waits(&dir, round)?;
            self.flow(&["approve", "review-2", "review", "--option", "rework"])?;
        }
        let status =
            wait_child(&mut runner.0, Duration::from_mins(1)).context("the loop did not end")?;
        ensure!(status.success(), "The run did not end as done: {status}");
        let state = read_state(&dir)?;
        ensure!(
            is(&state, "/steps/fix/status", "done")
                && number(&state, "/steps/fix/round") == Some(2)
                && is(&state, "/steps/release/status", "skipped"),
            "{state}"
        );
        Ok(())
    }
}
