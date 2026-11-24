//! `terminator-hook flow`: run a project's flow file
//! (`.terminator/flows/NAME.toml`, see `terminator_core::flow`).
//!
//! `flow run` is the only command that starts something, and only for a
//! project the user trusted in the GUI. It opens one new terminal per
//! command step and presses Enter on the step's command and prompt. A
//! gate step waits for `flow approve`. `flow cancel` starts no new step
//! and leaves running sessions alone. After a failure the runner waits:
//! `flow retry` starts the failed step again in a new terminal (the old one
//! stays), and `flow cancel` ends the run. `--set NAME=VALUE` gives the
//! flow's inputs.
//!
//! A gate with options takes `flow approve --option NAME`; the steps whose
//! `when` names another option are skipped, and a loop step runs its loop
//! again up to `max_rounds` (see `terminator_core::flow`).
//!
//! A step's own agent uses `flow context`, `flow report` and `flow ask`
//! (also MCP tools); the user answers with `flow answer`. The MCP server
//! has no tool for the user's side or for `flow run`.
mod agent;
mod cancel;
mod gates;
mod progress;
mod race;
mod retry;
mod runner;
mod wait;

use super::{cli_paths, lookup::Ctl};
use crate::output::Output;
use anyhow::{Result, bail, ensure};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use terminator_core::{
    flow::{
        Outcome, RUN_DIR, STATE_FILE,
        control::{self, ANSWERS, CANCEL, GateAnswer, REJECT},
        files::{self, plain_name},
    },
    *,
};

#[derive(Parser)]
#[command(about = "Run flows: steps for agents, with an order and human gates")]
struct Cli {
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    runtime_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the flow files of a project.
    List { project: String },
    /// Validate a flow file. Starts nothing.
    Check { project: String, name: String },
    /// Run a flow: one new terminal per step, Enter on each command and
    /// prompt. The project must be trusted in Terminator.
    Run {
        project: String,
        name: String,
        /// The run's name (its folder): letters, digits, '-' or '_'.
        /// Default: NAME-SECONDS. A run with that name must not exist.
        #[arg(long = "run", value_name = "ID")]
        run_id: Option<String>,
        /// The value of one input of the flow (repeat for more). A required
        /// input with no value refuses the run before anything starts.
        #[arg(long = "set", value_name = "NAME=VALUE")]
        set: Vec<String>,
        /// End the run when a step fails and nothing else can start. By
        /// default the runner waits for Retry or Cancel.
        #[arg(long)]
        end_on_failure: bool,
    },
    /// Start a failed step again as its next attempt, in a new terminal.
    /// The run's runner must live; the old terminal stays.
    Retry { run: String, step: String },
    /// Print the state of a run, with its status ("stopped (runner gone)"
    /// when its runner is gone).
    Status { run: String },
    /// Wait until a flow run reaches a target state (`ended`, `done`, `waiting`, `stopped`).
    Wait {
        run: String,
        #[arg(long, default_value = "ended", value_parser = ["ended", "done", "waiting", "stopped"])]
        until: String,
        /// Maximum seconds to wait.
        #[arg(long, default_value = "3600", value_parser = clap::value_parser!(u64).range(1..=86400))]
        timeout: u64,
    },
    /// Print structured JSONL lifecycle events for a flow run.
    Events {
        run: String,
        /// Keep streaming events until the run ends.
        #[arg(long)]
        follow: bool,
    },
    /// Write a Markdown summary (`report.md`) of an ended run.
    ExportReport { run: String },
    /// Approve a gate step that waits. A gate with options needs `--option`;
    /// a pick gate needs `--pick N`.
    Approve {
        run: String,
        step: String,
        /// The chosen option of a gate with options.
        #[arg(long, value_name = "NAME", conflicts_with = "pick")]
        option: Option<String>,
        /// The 1-based racer number chosen for a `pick` gate.
        #[arg(long, value_name = "N", conflicts_with = "option")]
        pick: Option<u32>,
    },
    /// Reject a gate step that waits; the steps after it do not start.
    Reject { run: String, step: String },
    /// Stop a run: no new step starts; running sessions continue.
    Cancel { run: String },
    /// Answer the open question of a step (see `ask`).
    Answer {
        run: String,
        step: String,
        text: String,
    },
    /// Store a message in a step's mailbox (never writes to a terminal).
    Send {
        run: String,
        step: String,
        text: String,
    },
    /// For a step's agent: read messages stored in its mailbox (`flow send`).
    Inbox {
        /// The step's terminal (default: this terminal).
        #[arg(long)]
        session: Option<String>,
    },
    /// For a step's agent: its run, its step and the steps it needs.
    Context {
        /// The step's terminal (default: this terminal).
        #[arg(long)]
        session: Option<String>,
    },
    /// For a step's agent or command: end this step as done or failed.
    Report {
        #[arg(value_parser = ["done", "failed"])]
        result: String,
        #[arg(long)]
        note: Option<String>,
        /// Output file path produced by this step (repeat for multiple).
        #[arg(long = "output", value_name = "PATH")]
        outputs: Vec<String>,
        #[arg(long)]
        session: Option<String>,
    },
    /// For a step's agent: ask the user and wait for `flow answer`.
    Ask {
        question: String,
        /// Seconds to wait for the answer.
        #[arg(long, default_value = "1800", value_parser = clap::value_parser!(u64).range(1..=86400))]
        timeout: u64,
        #[arg(long)]
        session: Option<String>,
    },
    /// For a step's agent: propose a new step to run inside this flow.
    Propose {
        id: String,
        command: String,
        #[arg(long)]
        prompt: Option<String>,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        worktree: Option<String>,
        /// Step ID this proposed step waits for (repeat for multiple; defaults to proposing step).
        #[arg(long = "needs", value_name = "STEP")]
        needs: Vec<String>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Print the skill text that tells an agent how to use these commands.
    Skill,
}

/// The skill file for agents. Nothing installs it automatically.
const SKILL: &str = include_str!("../../../assets/flow-skill.md");

fn program_args(args: &[String]) -> impl Iterator<Item = String> {
    std::iter::once("terminator-hook flow".to_owned()).chain(args.iter().cloned())
}

pub fn run(args: &[String]) -> Result<()> {
    let cli = Cli::parse_from(program_args(args));
    match cli.command {
        Command::Skill => Output::text(SKILL),
        Command::Events { run, follow } => {
            let paths = workspace(cli.data_dir, cli.runtime_dir)?;
            return wait::stream_events(&run_dir(&paths, &run)?, follow);
        }
        Command::Run {
            project,
            name,
            run_id,
            set,
            end_on_failure,
        } => {
            let paths = workspace(cli.data_dir, cli.runtime_dir)?;
            let start = Start {
                project,
                name,
                run: run_id,
                set,
                hold: !end_on_failure,
            };
            return run_flow(paths, start);
        }
        _ => Output::line(serde_json::to_string_pretty(&execute(cli)?)?),
    }
    Ok(())
}

/// Runs a flow command line for `terminator-hook mcp`: only what a step's
/// own agent may do. The user's side (run, approve, reject, cancel,
/// answer) is not available there.
pub(crate) fn execute_args(args: &[String]) -> Result<Value> {
    let cli = Cli::try_parse_from(program_args(args))?;
    ensure!(
        matches!(
            cli.command,
            Command::Context { .. }
                | Command::Inbox { .. }
                | Command::Report { .. }
                | Command::Ask { .. }
                | Command::Propose { .. }
                | Command::Status { .. }
        ),
        "This flow command is not available here"
    );
    execute(cli)
}

fn workspace(data_dir: Option<PathBuf>, runtime_dir: Option<PathBuf>) -> Result<Paths> {
    Ok(generations::workspace_paths(&cli_paths(
        data_dir,
        runtime_dir,
    )?)?)
}

fn execute(cli: Cli) -> Result<Value> {
    let paths = workspace(cli.data_dir, cli.runtime_dir)?;
    Ok(match cli.command {
        Command::List { project } => wait::list(&project_of(&paths, &project)?),
        Command::Check { project, name } => {
            let flow = files::load(&project_of(&paths, &project)?.path, &name)?;
            json!({"flow": flow.name, "steps": flow.steps.len(), "valid": true})
        }
        Command::Status { run } => wait::status(&run_dir(&paths, &run)?)?,
        Command::Wait {
            run,
            until,
            timeout,
        } => wait::wait_run(
            &run_dir(&paths, &run)?,
            &until,
            std::time::Duration::from_secs(timeout),
        )?,
        Command::ExportReport { run } => {
            let path = terminator_core::flow::report::write_run_report(&run_dir(&paths, &run)?)?;
            json!({"run": run, "report": path})
        }
        Command::Approve {
            run,
            step,
            option,
            pick,
        } => wait::execute_approve(&run_dir(&paths, &run)?, &step, (option, pick))?,
        Command::Reject { run, step } => {
            control::answer_gate(&run_dir(&paths, &run)?, &step, GateAnswer::Reject)?;
            json!({"step": step, "answer": REJECT})
        }
        Command::Cancel { run } => {
            control::cancel(&run_dir(&paths, &run)?)?;
            json!({"cancelled": true})
        }
        Command::Retry { run, step } => {
            control::retry(&run_dir(&paths, &run)?, &step)?;
            json!({"step": step, "retry": true})
        }
        Command::Answer { run, step, text } => {
            control::answer_question(&run_dir(&paths, &run)?, &step, &text)?;
            json!({"step": step, "answered": true})
        }
        Command::Send { run, step, text } => {
            let msg = control::send_message(&run_dir(&paths, &run)?, &step, &text)?;
            json!({"step": step, "sent": true, "seq": msg.seq})
        }
        cmd @ (Command::Inbox { .. }
        | Command::Context { .. }
        | Command::Report { .. }
        | Command::Ask { .. }
        | Command::Propose { .. }) => execute_agent(&paths, cmd)?,
        Command::Run { .. } | Command::Skill | Command::Events { .. } => {
            bail!("This command prints its own output")
        }
    })
}

fn execute_agent(paths: &Paths, cmd: Command) -> Result<Value> {
    match cmd {
        Command::Inbox { session } => Ok(agent::inbox(&agent::own_step(&paths.data, session)?)),
        Command::Context { session } => Ok(agent::context(&agent::own_step(&paths.data, session)?)),
        Command::Report {
            result,
            note,
            outputs,
            session,
        } => agent::report(
            &agent::own_step(&paths.data, session)?,
            result == "done",
            note.as_deref(),
            &outputs,
        ),
        Command::Ask {
            question,
            timeout,
            session,
        } => agent::ask(
            &agent::own_step(&paths.data, session)?,
            &question,
            std::time::Duration::from_secs(timeout),
        ),
        Command::Propose {
            id,
            command,
            prompt,
            cwd,
            worktree,
            needs,
            session,
        } => agent::propose(
            &agent::own_step(&paths.data, session)?,
            agent::ProposeInput {
                id,
                command,
                prompt,
                cwd,
                worktree,
                needs,
            },
        ),
        _ => bail!("Not an agent flow command"),
    }
}

/// A local project of the running daemon.
fn project_of(paths: &Paths, name: &str) -> Result<Project> {
    let project = Ctl::project(&Ctl::snapshot(paths)?, name)?;
    ensure!(
        !project.path.starts_with("/__terminator_remote__"),
        "Flows need a local project"
    );
    Ok(project)
}

/// What `flow run` starts.
struct Start {
    project: String,
    name: String,
    run: Option<String>,
    /// `--set NAME=VALUE`, as typed.
    set: Vec<String>,
    /// Wait for Retry or Cancel after a failure.
    hold: bool,
}

/// The flow `name` of `project` with the run's input values in its texts,
/// and those values. A missing or wrong value refuses the run here,
/// before any folder or terminal exists.
fn flow_with_inputs(
    project: &Project,
    name: &str,
    set: &[String],
) -> Result<(flow::Flow, std::collections::BTreeMap<String, String>)> {
    let flow = files::load(&project.path, name)?;
    let sets = set
        .iter()
        .map(|text| flow::inputs::parse_set(text))
        .collect::<Result<Vec<_>, _>>()?;
    let values = flow.input_values(&sets)?;
    Ok((flow.with_values(&values)?, values))
}

fn run_flow(paths: Paths, start: Start) -> Result<()> {
    let name = start.name.as_str();
    let project = project_of(&paths, &start.project)?;
    match ui_preferences::trust_answer(&paths.data, &project.path) {
        Some(true) => {}
        Some(false) => bail!("This project is not trusted; its flows stay off"),
        None => bail!(
            "This project has no trust answer. Answer the trust question in Terminator (Run task…), then run the flow again"
        ),
    }
    let (flow, values) = flow_with_inputs(&project, name, &start.set)?;
    // Before the folder exists: a signal from now on ends the run with a
    // state file, never an empty folder.
    let interrupted = runner::interrupt_flag()?;
    let (run, dir) = match start.run {
        Some(run) => {
            let dir = new_run_dir(&paths.data, &run)?;
            (run, dir)
        }
        None => new_auto_run_dir(&paths.data, name, now())?,
    };
    Output::line_or_drop(format_args!("Run {run}: {}", flow.name));
    let outcome = runner::Runner::new(paths, project, (flow, name), (dir, interrupted))
        .with_inputs(values, start.hold)
        .run()?;
    let outcome = match outcome {
        Outcome::Done => "done",
        Outcome::Active | Outcome::Stopped => "stopped",
    };
    // After SIGHUP the terminal is gone; the run ended all the same.
    Output::line_or_drop(format_args!("Run {run}: {outcome}"));
    if outcome != "done" {
        std::process::exit(1);
    }
    Ok(())
}

/// Makes the folder of an automatic run name (`NAME-SECONDS`, adding a
/// numeric suffix when two runs start in the same second).
fn new_auto_run_dir(data: &Path, name: &str, second: u64) -> Result<(String, PathBuf)> {
    let base = format!("{name}-{second}");
    let runs = data.join(RUN_DIR);
    std::fs::create_dir_all(&runs)?;
    for suffix in 0..100u32 {
        let candidate = if suffix == 0 {
            base.clone()
        } else {
            format!("{base}-{suffix}")
        };
        if !plain_name(&candidate) {
            bail!("A run name must be letters, digits, '-' or '_'");
        }
        let dir = runs.join(&candidate);
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok((candidate, dir)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    bail!("Run {base} exists; try again")
}

/// Makes the folder of the new run `run` in the data folder. A run with
/// that name is never used again: the folder must not exist.
fn new_run_dir(data: &Path, run: &str) -> Result<PathBuf> {
    ensure!(
        plain_name(run),
        "A run name must be letters, digits, '-' or '_'"
    );
    let runs = data.join(RUN_DIR);
    std::fs::create_dir_all(&runs)?;
    let dir = runs.join(run);
    match std::fs::create_dir(&dir) {
        Ok(()) => Ok(dir),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            bail!("Run {run} exists; try again")
        }
        Err(error) => Err(error.into()),
    }
}

fn run_dir(paths: &Paths, run: &str) -> Result<PathBuf> {
    ensure!(
        plain_name(run),
        "A run name must be letters, digits, '-' or '_'"
    );
    let dir = paths.data.join(RUN_DIR).join(run);
    ensure!(dir.join(STATE_FILE).is_file(), "Unknown run");
    Ok(dir)
}

#[cfg(test)]
mod cancel_tests;
#[cfg(test)]
mod gates_tests;
#[cfg(test)]
mod retry_tests;
#[cfg(test)]
mod tests;
