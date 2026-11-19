//! `terminator-hook flow`: run a project's flow file
//! (`.terminator/flows/NAME.toml`, see `terminator_core::flow`).
//!
//! `flow run` is the only command that starts something, and only for a
//! project the user trusted in the GUI. It opens one new terminal per
//! command step and presses Enter on the step's command and prompt. A
//! gate step waits for `flow approve`. `flow cancel` starts no new step
//! and leaves running sessions alone.
//!
//! A step's own agent uses `flow context`, `flow report` and `flow ask`
//! (also MCP tools); the user answers with `flow answer`. The MCP server
//! has no tool for the user's side or for `flow run`.
mod agent;
mod progress;
mod runner;

use super::{cli_paths, lookup::Ctl};
use crate::output::Output;
use anyhow::{Result, bail, ensure};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use terminator_core::{
    flow::{
        Flow, Outcome, RUN_DIR, RunState, STATE_FILE,
        control::{self, ANSWERS, APPROVE, CANCEL, REJECT},
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
    Run { project: String, name: String },
    /// Print the state of a run.
    Status { run: String },
    /// Approve a gate step that waits.
    Approve { run: String, step: String },
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
        Command::Run { project, name } => {
            let paths = workspace(cli.data_dir, cli.runtime_dir)?;
            return run_flow(paths, &project, &name);
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
                | Command::Report { .. }
                | Command::Ask { .. }
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
        Command::List { project } => list(&project_of(&paths, &project)?),
        Command::Check { project, name } => {
            let flow = load(&project_of(&paths, &project)?, &name)?;
            json!({"flow": flow.name, "steps": flow.steps.len(), "valid": true})
        }
        Command::Status { run } => serde_json::to_value(read_state(&run_dir(&paths, &run)?)?)?,
        Command::Approve { run, step } => {
            control::answer_gate(&run_dir(&paths, &run)?, &step, true)?;
            json!({"step": step, "answer": APPROVE})
        }
        Command::Reject { run, step } => {
            control::answer_gate(&run_dir(&paths, &run)?, &step, false)?;
            json!({"step": step, "answer": REJECT})
        }
        Command::Cancel { run } => {
            control::cancel(&run_dir(&paths, &run)?)?;
            json!({"cancelled": true})
        }
        Command::Answer { run, step, text } => {
            control::answer_question(&run_dir(&paths, &run)?, &step, &text)?;
            json!({"step": step, "answered": true})
        }
        Command::Context { session } => agent::context(&agent::own_step(&paths.data, session)?),
        Command::Report {
            result,
            note,
            session,
        } => agent::report(
            &agent::own_step(&paths.data, session)?,
            result == "done",
            note.as_deref(),
        )?,
        Command::Ask {
            question,
            timeout,
            session,
        } => agent::ask(
            &agent::own_step(&paths.data, session)?,
            &question,
            std::time::Duration::from_secs(timeout),
        )?,
        Command::Run { .. } | Command::Skill => bail!("This command prints its own output"),
    })
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

/// Flow and run names are plain file names, never a path.
fn plain_name(name: &str) -> Result<&str> {
    ensure!(
        flow::files::plain_name(name),
        "A name must be letters, digits, '-' or '_'"
    );
    Ok(name)
}

/// The flow `name` of the project.
fn load(project: &Project, name: &str) -> Result<Flow> {
    Ok(flow::files::load(&project.path, name)?)
}

/// Each flow file with its name, or the reason it is refused.
fn list(project: &Project) -> Value {
    let names = flow::files::names(&project.path);
    let rows = names.into_iter().map(|file| match load(project, &file) {
        Ok(flow) => json!({"file": file, "flow": flow.name, "steps": flow.steps.len()}),
        Err(error) => json!({"file": file, "error": format!("{error:#}")}),
    });
    Value::Array(rows.collect())
}

/// The trust answer the user gave in the GUI for this project folder
/// (`lsp_trust` in `ui-preferences.json`), shared with project tasks.
fn trust_answer(data: &Path, root: &Path) -> Option<bool> {
    let text = std::fs::read(data.join("ui-preferences.json")).ok()?;
    let preferences: Value = serde_json::from_slice(&text).ok()?;
    preferences
        .get("lsp_trust")?
        .get(root.to_string_lossy().as_ref())?
        .as_bool()
}

fn run_flow(paths: Paths, project: &str, name: &str) -> Result<()> {
    let project = project_of(&paths, project)?;
    match trust_answer(&paths.data, &project.path) {
        Some(true) => {}
        Some(false) => bail!("This project is not trusted; its flows stay off"),
        None => bail!(
            "This project has no trust answer. Answer the trust question in Terminator (Run task…), then run the flow again"
        ),
    }
    let flow = load(&project, name)?;
    let run = format!("{name}-{}", now());
    let dir = paths.data.join(RUN_DIR).join(&run);
    ensure!(!dir.exists(), "Run {run} exists; try again");
    Output::line(format_args!("Run {run}: {}", flow.name));
    let outcome_of = flow.clone();
    let state = runner::Runner::new(paths, project, (flow, name), dir)?.run()?;
    let outcome = match outcome_of.outcome(&state) {
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

fn run_dir(paths: &Paths, run: &str) -> Result<PathBuf> {
    let dir = paths.data.join(RUN_DIR).join(plain_name(run)?);
    ensure!(dir.join(STATE_FILE).is_file(), "Unknown run");
    Ok(dir)
}

fn read_state(dir: &Path) -> Result<RunState> {
    Ok(control::read_state(dir)?)
}

#[cfg(test)]
mod tests;
