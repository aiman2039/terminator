//! Flow files: steps for agents, with an order and human gates.
//!
//! A flow is one TOML file in `.terminator/flows/` of a project:
//!
//! ```toml
//! version = 1
//! name = "Plan and build"
//!
//! [[step]]
//! id = "plan"
//! command = "claude"            # typed in a new terminal
//! prompt = "Write the plan."    # optional, sent when the agent waits
//! worktree = "plan"             # optional, new worktree terminator/plan
//!
//! [[step]]
//! id = "approve"
//! gate = "Is the plan correct?" # waits for Approve in the Agents inbox
//! needs = ["plan"]
//! at = [320, 40]                # optional, the step's place in the editor
//! ```
//!
//! [`Draft`] is the same file as plain data. The native flow editor makes
//! one, and [`Draft::to_toml`] writes the file after [`Draft::check`].
//!
//! `prompt` is sent after the agent's first hook report (Claude Code
//! reports at its start). An agent that reports only after its first
//! prompt (Codex) needs the prompt in `command`: `codex "Write the plan."`.
//!
//! The format is strict: an unknown key, a missing version, a bad ID, an
//! unknown `needs` name or a cycle is an error, and no step runs. This
//! module only reads files and computes which steps can start; it starts
//! nothing. The runner (`terminator-hook flow run`) starts a flow only on
//! an explicit Run.
use petgraph::{algo::toposort, graph::DiGraph};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Component, Path},
};

pub mod control;
pub mod files;
mod run;
pub use run::{Outcome, RUN_DIR, RunState, STATE_FILE, StepRun, StepStatus};

/// Flow files of a project, relative to the project folder.
pub const FLOW_DIR: &str = ".terminator/flows";
/// The only file version this build reads.
pub const VERSION: u32 = 1;
/// Most steps in one flow.
pub const MAX_STEPS: usize = 64;
/// Longest step ID.
const MAX_ID: usize = 64;
/// Longest command, prompt, gate message or flow name, in bytes.
const MAX_TEXT: usize = 16 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FlowError {
    #[error("{0}")]
    Syntax(String),
    #[error("Flow version {0} is not supported (this build reads version {VERSION})")]
    Version(u32),
    #[error("Flow name is empty or too long")]
    Name,
    #[error("A flow needs at least one step")]
    Empty,
    #[error("A flow has at most {MAX_STEPS} steps")]
    TooManySteps,
    #[error("Step ID \"{0}\" must be 1 to {MAX_ID} characters of a-z, 0-9, - or _")]
    BadId(String),
    #[error("Step ID \"{0}\" is used twice")]
    DuplicateId(String),
    #[error("Step \"{0}\" must have exactly one of command or gate")]
    Kind(String),
    #[error("Step \"{0}\": prompt, cwd and worktree apply only to a command step")]
    GateField(String),
    #[error("Step \"{0}\": {1} is empty, too long or has a control character")]
    Text(String, &'static str),
    #[error("Step \"{0}\": cwd must stay inside the project")]
    Cwd(String),
    #[error("Step \"{0}\": worktree must be letters, digits, '-', '_' or '.'")]
    Worktree(String),
    #[error("Step \"{step}\" needs unknown step \"{needs}\"")]
    UnknownNeed { step: String, needs: String },
    #[error("Step \"{0}\" is part of a cycle")]
    Cycle(String),
}

/// What a step does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepKind {
    /// Type `command` in a new terminal; send `prompt` when the agent waits.
    Command {
        /// One line.
        command: String,
        prompt: Option<String>,
        /// Folder relative to the project (or to the step's worktree).
        cwd: Option<String>,
        /// Name of a new worktree for this step (branch `terminator/NAME`).
        worktree: Option<String>,
    },
    /// Stop until the user approves `message`.
    Gate { message: String },
}

/// One validated step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub id: String,
    /// IDs of the steps that must be done first; each one exists.
    pub needs: Vec<String>,
    pub kind: StepKind,
    /// The step's place in the flow editor. Display only.
    pub at: Option<[i32; 2]>,
}

/// A validated flow: unique IDs, known `needs`, no cycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    pub name: String,
    /// In file order.
    pub steps: Vec<Step>,
}

/// A flow file as plain data, not checked: what the file says, and what
/// the flow editor makes. [`Draft::check`] gives the checked [`Flow`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub version: u32,
    pub name: String,
    #[serde(default, rename = "step")]
    pub steps: Vec<DraftStep>,
}

/// One step of a [`Draft`]. A step has `command` or `gate`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftStep {
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[i32; 2]>,
}

/// Read and validate one flow file's text.
pub fn parse(text: &str) -> Result<Flow, FlowError> {
    let draft: Draft =
        toml_edit::de::from_str(text).map_err(|error| FlowError::Syntax(error.to_string()))?;
    draft.check()
}

impl Draft {
    /// A draft of this file version.
    pub fn new(name: &str, steps: Vec<DraftStep>) -> Self {
        Self {
            version: VERSION,
            name: name.to_owned(),
            steps,
        }
    }

    /// The checked flow of this draft.
    pub fn check(self) -> Result<Flow, FlowError> {
        if self.version != VERSION {
            return Err(FlowError::Version(self.version));
        }
        let name = self.name.trim();
        if !text_ok(name, false) {
            return Err(FlowError::Name);
        }
        if self.steps.is_empty() {
            return Err(FlowError::Empty);
        }
        if self.steps.len() > MAX_STEPS {
            return Err(FlowError::TooManySteps);
        }
        let steps = self.steps.into_iter().map(step).collect::<Result<_, _>>()?;
        let flow = Flow {
            name: name.to_owned(),
            steps,
        };
        flow.check_graph()?;
        Ok(flow)
    }

    /// The file text of a correct draft. A draft that is not a correct
    /// flow gives its error and no text.
    pub fn to_toml(&self) -> Result<String, FlowError> {
        self.clone().check()?;
        toml_edit::ser::to_string_pretty(self).map_err(|error| FlowError::Syntax(error.to_string()))
    }
}

impl From<&Flow> for Draft {
    fn from(flow: &Flow) -> Self {
        let steps = flow.steps.iter().map(|step| {
            let mut draft = DraftStep {
                id: step.id.clone(),
                needs: step.needs.clone(),
                at: step.at,
                ..DraftStep::default()
            };
            match &step.kind {
                StepKind::Command {
                    command,
                    prompt,
                    cwd,
                    worktree,
                } => {
                    draft.command = Some(command.clone());
                    draft.prompt.clone_from(prompt);
                    draft.cwd.clone_from(cwd);
                    draft.worktree.clone_from(worktree);
                }
                StepKind::Gate { message } => draft.gate = Some(message.clone()),
            }
            draft
        });
        Self::new(&flow.name, steps.collect())
    }
}

fn step(raw: DraftStep) -> Result<Step, FlowError> {
    let id = raw.id;
    if !id_ok(&id) {
        return Err(FlowError::BadId(id));
    }
    let kind = match (raw.command, raw.gate) {
        (Some(command), None) => StepKind::Command {
            command: checked(&id, "command", &command, false)?,
            prompt: raw
                .prompt
                .map(|prompt| checked(&id, "prompt", &prompt, true))
                .transpose()?,
            cwd: raw.cwd.map(|cwd| checked_cwd(&id, &cwd)).transpose()?,
            worktree: raw
                .worktree
                .map(|name| checked_worktree(&id, &name))
                .transpose()?,
        },
        (None, Some(message)) => {
            if raw.prompt.is_some() || raw.cwd.is_some() || raw.worktree.is_some() {
                return Err(FlowError::GateField(id));
            }
            StepKind::Gate {
                message: checked(&id, "gate", &message, true)?,
            }
        }
        _ => return Err(FlowError::Kind(id)),
    };
    Ok(Step {
        id,
        needs: raw.needs,
        kind,
        at: raw.at,
    })
}

fn id_ok(id: &str) -> bool {
    (1..=MAX_ID).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
}

/// Not empty, bounded, and no control character (line breaks and tabs are
/// allowed only in `multiline` text).
fn text_ok(text: &str, multiline: bool) -> bool {
    !text.is_empty()
        && text.len() <= MAX_TEXT
        && text
            .chars()
            .all(|c| !c.is_control() || (multiline && matches!(c, '\n' | '\t')))
}

fn checked(
    id: &str,
    field: &'static str,
    text: &str,
    multiline: bool,
) -> Result<String, FlowError> {
    let text = text.trim();
    if text_ok(text, multiline) {
        Ok(text.to_owned())
    } else {
        Err(FlowError::Text(id.to_owned(), field))
    }
}

fn checked_cwd(id: &str, cwd: &str) -> Result<String, FlowError> {
    let cwd = checked(id, "cwd", cwd, false)?;
    let inside = Path::new(&cwd)
        .components()
        .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    if inside {
        Ok(cwd)
    } else {
        Err(FlowError::Cwd(id.to_owned()))
    }
}

/// The same names as `ctl spawn --worktree`: the branch is
/// `terminator/NAME` in the folder `terminator-NAME` beside the project.
fn checked_worktree(id: &str, name: &str) -> Result<String, FlowError> {
    let plain = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(['-', '.'])
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if plain {
        Ok(name.to_owned())
    } else {
        Err(FlowError::Worktree(id.to_owned()))
    }
}

impl Flow {
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|step| step.id == id)
    }

    /// Unique IDs, known `needs` and no cycle.
    fn check_graph(&self) -> Result<(), FlowError> {
        let mut graph = DiGraph::<&str, ()>::new();
        let mut nodes = BTreeMap::new();
        for step in &self.steps {
            let node = graph.add_node(step.id.as_str());
            if nodes.insert(step.id.as_str(), node).is_some() {
                return Err(FlowError::DuplicateId(step.id.clone()));
            }
        }
        for step in &self.steps {
            for needs in &step.needs {
                let (Some(from), Some(to)) =
                    (nodes.get(needs.as_str()), nodes.get(step.id.as_str()))
                else {
                    return Err(FlowError::UnknownNeed {
                        step: step.id.clone(),
                        needs: needs.clone(),
                    });
                };
                graph.add_edge(*from, *to, ());
            }
        }
        toposort(&graph, None).map(|_| ()).map_err(|cycle| {
            let id = graph.node_weight(cycle.node_id()).copied().unwrap_or("");
            FlowError::Cycle(id.to_owned())
        })
    }
}

#[cfg(test)]
mod tests;
