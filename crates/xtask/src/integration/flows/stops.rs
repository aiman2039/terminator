//! How a flow run stops: a step that reports a failure and a rejected
//! gate stop only the steps after them, and Cancel starts no new step and
//! leaves the running terminals alone.
use super::{Case, bin, id, is, read_state, until, wait_child};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{fs, time::Duration};
use terminator_core::{flow::FLOW_DIR, quote};

impl Case<'_> {
    /// `bad` reports a failure and the gate is rejected: the steps after
    /// them never start (no terminal), the free branch is done, and the
    /// runner ends as stopped with a failure exit.
    pub(super) fn failure_and_reject_stop_only_the_steps_after_them(&self) -> Result<()> {
        let hook = quote(&bin().join("terminator-hook").to_string_lossy());
        let text = format!(
            r#"version = 1
name = "Stops"

[[step]]
id = "bad"
command = "{hook} flow report failed --note broke"

[[step]]
id = "after-bad"
needs = ["bad"]
command = "{hook} flow report done"

[[step]]
id = "gate"
gate = "Go?"

[[step]]
id = "after-gate"
needs = ["gate"]
command = "{hook} flow report done"

[[step]]
id = "free"
command = "{hook} flow report done"
"#
        );
        fs::write(self.root.join(FLOW_DIR).join("stops.toml"), text)?;
        let (mut runner, dir) = self.spawn_run("stops", "stops-1", |state| {
            is(state, "/steps/gate/status", "running")
                && is(state, "/steps/bad/status", "failed")
                && is(state, "/steps/free/status", "done")
        })?;
        self.flow(&["reject", "stops-1", "gate"])?;
        let status = wait_child(&mut runner.0, Duration::from_secs(30))?;
        ensure!(!status.success(), "A run with a failed step exited as done");
        let state = read_state(&dir)?;
        ensure!(state.get("ended") == Some(&Value::Bool(true)), "{state}");
        ensure!(
            is(&state, "/steps/bad/note", "broke") && is(&state, "/steps/gate/note", "Rejected"),
            "{state}"
        );
        for after in ["after-bad", "after-gate"] {
            ensure!(
                is(&state, &format!("/steps/{after}/status"), "waiting")
                    && state.pointer(&format!("/steps/{after}/session")).is_none(),
                "{after} started after a failure: {state}"
            );
        }
        let shown: Value = serde_json::from_slice(&self.flow(&["status", "stops-1"])?)?;
        ensure!(is(&shown, "/status", "stopped"), "{shown}");
        Ok(())
    }

    /// Cancel of a live run: the running step is cancelled, the step after
    /// it never starts, and the running step's terminal stays.
    pub(super) fn cancel_starts_nothing_and_leaves_sessions(&self) -> Result<()> {
        let hook = quote(&bin().join("terminator-hook").to_string_lossy());
        let text = format!(
            "version = 1\nname = \"Long\"\n\n[[step]]\nid = \"long\"\ncommand = \"sleep 600\"\n\n[[step]]\nid = \"next\"\nneeds = [\"long\"]\ncommand = \"{hook} flow report done\"\n"
        );
        fs::write(self.root.join(FLOW_DIR).join("long.toml"), text)?;
        let (mut runner, dir) = self.spawn_run("long", "long-1", |state| {
            is(state, "/steps/long/status", "running")
                && state.pointer("/steps/long/session").is_some()
        })?;
        let session = read_state(&dir)?
            .pointer("/steps/long/session")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        self.flow(&["cancel", "long-1"])?;
        let status = wait_child(&mut runner.0, Duration::from_secs(10))?;
        ensure!(!status.success(), "A cancelled run exited as done");
        let state = read_state(&dir)?;
        ensure!(
            state.get("ended") == Some(&Value::Bool(true))
                && is(&state, "/steps/long/status", "cancelled")
                && is(&state, "/steps/next/status", "cancelled")
                && state.pointer("/steps/next/session").is_none(),
            "{state}"
        );
        // The daemon keeps the step's terminal: Cancel closes nothing.
        until(5, "the step's terminal", || {
            let live = self
                .h
                .state()?
                .get("sessions")
                .and_then(Value::as_array)
                .is_some_and(|all| {
                    all.iter()
                        .any(|s| id(s) == session && is(s, "/lifecycle", "running"))
                });
            Ok(live.then_some(()))
        })?;
        ensure!(
            self.flow(&["cancel", "long-1"]).is_err(),
            "An ended run took a second cancel"
        );
        Ok(())
    }
}
