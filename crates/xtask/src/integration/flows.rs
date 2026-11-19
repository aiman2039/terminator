//! `terminator-hook flow` against a real daemon and real PTYs: the trust
//! gate, a shell step that ends by its own report, a human gate, the
//! handoff note, a question with its answer, and a fixture agent whose
//! prompt waits for the first hook report. The agent is `terminator-hook
//! emit` with normalized hook events; no real agent runs.
use super::super::harness::{Harness, Process, bin, git, id, output, wait_child};
use anyhow::{Context, Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::{Duration, Instant},
};
use terminator_core::{
    flow::{FLOW_DIR, RUN_DIR, STATE_FILE, control},
    quote,
    signals::ProcSignal,
    ui_preferences,
};

pub fn flows() -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("flow-project")?;
    let root = PathBuf::from(
        project
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing project path"))?,
    );
    write_flow(&h.root, &root)?;
    let case = Case {
        h: &h,
        project,
        root,
    };
    case.refused_without_trust()?;
    let run = "demo-1";
    let (mut runner, dir) = case.spawn_run("demo", run, |_| true)?;
    case.gate_and_question(&dir, run)?;
    let status = wait_child(&mut runner.0, Duration::from_mins(1))?;
    ensure!(status.success(), "The flow did not end as done: {status}");
    case.results(&dir)?;
    ensure!(
        case.flow(&["cancel", run]).is_err(),
        "An ended run took a cancel"
    );
    ensure!(
        case.flow(&["run", id(&case.project), "demo", "--run", run])
            .is_err(),
        "A second run took the folder of an earlier run"
    );
    case.hangup_ends_the_run()?;
    case.killed_runner_is_gone()?;
    case.worktree_step_runs_twice()?;
    println!(
        "{}",
        json!({"flow_trust_gate":true,"flow_shell_report":true,"flow_gate":true,"flow_handoff_note":true,"flow_ask_answer":true,"flow_fixture_agent_prompt":true,"flow_sighup_ends_run":true,"flow_killed_runner_gone":true,"flow_worktree_second_run":true})
    );
    Ok(())
}

/// A flow whose only step is a gate: its run waits until a signal.
const HOLD: &str =
    "version = 1\nname = \"Hold\"\n\n[[step]]\nid = \"wait\"\ngate = \"Continue?\"\n";

/// One fixture daemon with the flow's project.
struct Case<'a> {
    h: &'a Harness,
    project: Value,
    root: PathBuf,
}

impl Case<'_> {
    /// Runs `terminator-hook flow ARGS`.
    fn flow(&self, args: &[&str]) -> Result<Vec<u8>> {
        let mut command = self.h.command("terminator-hook");
        command.arg("flow").args(args);
        output(command)
    }

    /// No trust answer: nothing starts. Then the answer is saved as the
    /// GUI saves it.
    fn refused_without_trust(&self) -> Result<()> {
        let refused = self.flow(&["run", id(&self.project), "demo"]);
        ensure!(refused.is_err(), "A flow ran without the trust answer");
        ensure!(
            self.h
                .state()?
                .get("sessions")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty),
            "A refused flow made a terminal"
        );
        let preferences = json!({
            "version": 2, "ide_mode": false,
            (ui_preferences::TRUST_KEY): {(self.root.to_string_lossy()): true},
        });
        fs::write(
            self.h.root.join(ui_preferences::FILE),
            serde_json::to_vec(&preferences)?,
        )?;
        Ok(())
    }

    /// Starts `flow run` of flow file `file` as run `run`, and gives its
    /// process and its run folder once its state passes `ready`.
    fn spawn_run(
        &self,
        file: &str,
        run: &str,
        ready: impl Fn(&Value) -> bool,
    ) -> Result<(Process, PathBuf)> {
        let mut command = self.h.command("terminator-hook");
        command
            .args(["flow", "run", id(&self.project), file, "--run", run])
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        let runner = Process(command.spawn().context("flow run did not start")?);
        let dir = self.h.root.join(RUN_DIR).join(run);
        // The state file is replaced in one step: a read error means it
        // is not there yet.
        until(20, &format!("run {run}"), || {
            Ok(read_state(&dir).ok().filter(|state| ready(state)).map(drop))
        })?;
        Ok((runner, dir))
    }

    /// The shell step ends by its report, the gate waits for the approve,
    /// and the next step's question gets its answer.
    fn gate_and_question(&self, dir: &Path, run: &str) -> Result<()> {
        ensure!(
            self.flow(&["approve", run, "build"]).is_err(),
            "A command step took a gate answer"
        );
        let step_is = |pointer: &str, want: &str| -> Result<bool> {
            Ok(is(&read_state(dir)?, &format!("/steps/{pointer}"), want))
        };
        until(30, "the gate", || {
            Ok(step_is("ok/status", "running")?.then_some(()))
        })?;
        ensure!(step_is("build/status", "done")?, "build is not done");
        ensure!(step_is("build/note", "built out.txt")?, "no result note");
        ensure!(step_is("ask/status", "waiting")?, "A step passed the gate");
        self.flow(&["approve", run, "ok"])?;
        until(30, "the question", || {
            Ok(step_is("ask/question", "A or B?")?.then_some(()))
        })
        .with_context(|| {
            // The terminal text of the step shows why it did not ask.
            let state = read_state(dir).unwrap_or_default();
            let session = state.pointer("/steps/ask/session").and_then(Value::as_str);
            let text = session.and_then(|sid| self.h.history(sid).ok());
            format!("state: {state}\nterminal: {}", text.unwrap_or_default())
        })?;
        self.flow(&["answer", run, "ask", "Use A"])?;
        Ok(())
    }

    /// Starts `flow run` of a new flow file `file` whose only step is a
    /// gate, as run `FILE-1`, and gives its process and its run folder
    /// once the gate waits.
    fn start_hold(&self, file: &str) -> Result<(Process, PathBuf)> {
        let flows = self.root.join(FLOW_DIR);
        fs::write(flows.join(format!("{file}.toml")), HOLD)?;
        let (runner, dir) = self.spawn_run(file, &format!("{file}-1"), |state| {
            is(state, "/steps/wait/status", "running")
        })?;
        ensure!(control::alive(&dir), "A live runner holds no lock");
        Ok((runner, dir))
    }

    /// SIGHUP (its terminal tab closed) ends the run as cancelled.
    fn hangup_ends_the_run(&self) -> Result<()> {
        let (mut runner, dir) = self.start_hold("hangup")?;
        terminator_core::signals::signal_process(runner.0.id(), ProcSignal::Hangup)?;
        wait_child(&mut runner.0, Duration::from_secs(10))?;
        let state = read_state(&dir)?;
        ensure!(
            state.get("ended") == Some(&Value::Bool(true)),
            "SIGHUP left the run open: {state}"
        );
        ensure!(
            is(&state, "/steps/wait/status", "cancelled") && is(&state, "/file", "hangup"),
            "{state}"
        );
        Ok(())
    }

    /// SIGKILL writes nothing: the lock shows that the runner is gone.
    /// Such a run takes no cancel, and Remove deletes it.
    fn killed_runner_is_gone(&self) -> Result<()> {
        let (mut runner, dir) = self.start_hold("killed")?;
        terminator_core::signals::signal_process(runner.0.id(), ProcSignal::Kill)?;
        wait_child(&mut runner.0, Duration::from_secs(10))?;
        let state = read_state(&dir)?;
        ensure!(state.get("ended") != Some(&Value::Bool(true)), "{state}");
        ensure!(
            !control::alive(&dir),
            "A killed runner still holds its lock"
        );
        let run = control::run_name(&dir);
        ensure!(
            self.flow(&["cancel", &run]).is_err() && self.flow(&["approve", &run, "wait"]).is_err(),
            "A run without a runner took an answer"
        );
        let status: Value = serde_json::from_slice(&self.flow(&["status", &run])?)?;
        ensure!(
            is(&status, "/status", "stopped (runner gone)")
                && is(&status, "/steps/wait/status", "cancelled"),
            "flow status shows a killed runner's run as running: {status}"
        );
        control::remove(&self.h.root, &dir)?;
        ensure!(!dir.exists(), "The run of a killed runner stayed");
        Ok(())
    }

    /// A step with `worktree` makes the worktree on the first run and runs
    /// in that same checkout on the second run.
    fn worktree_step_runs_twice(&self) -> Result<()> {
        let author = [
            "-c",
            "user.name=Flow",
            "-c",
            "user.email=flow@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ];
        git(&self.root, &["init", "-q"])?;
        git(&self.root, &[&author[..], &["add", "-A"]].concat())?;
        git(
            &self.root,
            &[&author[..], &["commit", "-q", "-m", "init"]].concat(),
        )?;
        let hook = quote(&bin().join("terminator-hook").to_string_lossy());
        let text = format!(
            "version = 1\nname = \"Tree\"\n\n[[step]]\nid = \"work\"\nworktree = \"flowtree\"\ncommand = \"{hook} flow report done\"\n"
        );
        fs::write(self.root.join(FLOW_DIR).join("tree.toml"), text)?;
        // The first run has the default name, the second its own.
        for (round, run) in [("first", None), ("second", Some("tree-again"))] {
            let mut args = vec!["run", id(&self.project), "tree"];
            args.extend(run.map(|run| ["--run", run]).into_iter().flatten());
            self.flow(&args)
                .with_context(|| format!("The {round} run of a worktree step failed"))?;
        }
        let checkout = self
            .root
            .parent()
            .context("project parent")?
            .join("terminator-flowtree");
        let branch = git(&checkout, &["branch", "--show-current"])?;
        ensure!(
            String::from_utf8_lossy(&branch).trim() == "terminator/flowtree",
            "The step's worktree is not on its branch"
        );
        Ok(())
    }

    /// Every step is done; the handoff note and the answer arrived.
    fn results(&self, dir: &Path) -> Result<()> {
        let state = read_state(dir)?;
        ensure!(
            state.get("ended") == Some(&Value::Bool(true)),
            "The run did not end"
        );
        for name in ["build", "ok", "ask", "agent"] {
            ensure!(
                is(&state, &format!("/steps/{name}/status"), "done"),
                "{name}: {state}"
            );
        }
        let context: Value = serde_json::from_slice(&fs::read(self.root.join("ctx.json"))?)?;
        ensure!(
            is(&context, "/step", "ask") && is(&context, "/needs/0/note", "built out.txt"),
            "flow context has no handoff note: {context}"
        );
        let answer: Value = serde_json::from_slice(&fs::read(self.root.join("answer.json"))?)?;
        ensure!(
            is(&answer, "/answer", "Use A"),
            "The answer did not arrive: {answer}"
        );
        Ok(())
    }
}

/// The flow file and the hook events of the fixture agent.
fn write_flow(data: &Path, root: &Path) -> Result<()> {
    let hook = quote(&bin().join("terminator-hook").to_string_lossy());
    let event = |name: &str, state: &str| -> Result<String> {
        let file = data.join(format!("flow-{name}.json"));
        fs::write(
            &file,
            serde_json::to_vec(&json!({
                "protocol_version": 1, "event_id": format!("flow-{name}"),
                "terminal_session_id": "", "agent_invocation_id": "flow-agent",
                "agent_kind": "claude", "provider_session_id": null, "state": state,
                "request_id": null, "sequence": null, "summary": name,
            }))?,
        )?;
        Ok(quote(&file.to_string_lossy()))
    };
    let (start, done) = (event("start", "unknown")?, event("done", "completed")?);
    let text = format!(
        r#"version = 1
name = "Demo"

[[step]]
id = "build"
command = "echo hi > out.txt && {hook} flow report done --note 'built out.txt'"

[[step]]
id = "ok"
gate = "Is the build correct?"
needs = ["build"]

[[step]]
id = "ask"
needs = ["build", "ok"]
command = "{hook} flow context > ctx.json; {hook} flow ask 'A or B?' > answer.json && {hook} flow report done"

[[step]]
id = "agent"
needs = ["ok"]
command = "{hook} emit < {start}"
prompt = "sleep 3; {hook} emit < {done}"
"#
    );
    fs::create_dir_all(root.join(FLOW_DIR))?;
    Ok(fs::write(root.join(FLOW_DIR).join("demo.toml"), text)?)
}

fn read_state(dir: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(dir.join(STATE_FILE))?)?)
}

/// Whether the string at a JSON pointer is `want`.
fn is(value: &Value, pointer: &str, want: &str) -> bool {
    value.pointer(pointer).and_then(Value::as_str) == Some(want)
}

/// Polls `check` until it gives a value. The state file is replaced in
/// one step, so a read error is a real error.
fn until<T>(seconds: u64, what: &str, mut check: impl FnMut() -> Result<Option<T>>) -> Result<T> {
    let end = Instant::now()
        .checked_add(Duration::from_secs(seconds))
        .ok_or_else(|| anyhow!("deadline overflow"))?;
    loop {
        if let Some(value) = check()? {
            return Ok(value);
        }
        ensure!(Instant::now() < end, "Timed out waiting for {what}");
        thread::sleep(Duration::from_millis(50));
    }
}
