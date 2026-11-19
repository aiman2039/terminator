//! Flow files: steps for agents, with an order and human gates.
//!
//! A flow is one TOML file in `.terminator/flows/` of a project:
//!
//! ```toml
//! version = 1
//! name = "Plan and build"
//!
//! [inputs.issue]                # optional run inputs (see `inputs`)
//! description = "Issue number"
//! default = ""                  # optional; no default: required
//!
//! [[step]]
//! id = "plan"
//! command = "claude"            # typed in a new terminal
//! prompt = "Plan issue {{inputs.issue}}." # optional, sent when the agent waits
//! worktree = "plan"             # optional, worktree terminator/plan (made
//!                               # on the first run, used again later)
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
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Component, Path},
};

pub mod control;
pub mod files;
mod graph;
pub mod inputs;
mod run;
pub use graph::Ordered;
pub use inputs::{Input, InputError};
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
    #[error("rules is too long or has a control character")]
    Rules,
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
    #[error(
        "Step \"{step}\": worktree \"{name}\" is also the worktree of step \"{first}\"; give each step its own worktree name"
    )]
    DuplicateWorktree {
        step: String,
        first: String,
        name: String,
    },
    #[error("Step \"{step}\" needs unknown step \"{needs}\"")]
    UnknownNeed { step: String, needs: String },
    #[error("Step \"{0}\" is part of a cycle")]
    Cycle(String),
    #[error("Input name \"{0}\" must be 1 to {MAX_ID} characters of a-z, 0-9, - or _")]
    BadInput(String),
    #[error("Input \"{0}\": {1} is too long or has a control character")]
    InputText(String, &'static str),
    #[error("A flow has at most {} inputs", inputs::MAX_INPUTS)]
    TooManyInputs,
    /// A `{{…}}` in a text that is not a declared input at a good place.
    #[error("{}: {problem}", place_name(.step.as_deref(), .field))]
    Placeholder {
        step: Option<String>,
        field: &'static str,
        problem: String,
    },
}

/// "Step \"build\": command", or "rules" for the flow's rules.
fn place_name(step: Option<&str>, field: &str) -> String {
    match step {
        Some(step) => format!("Step \"{step}\": {field}"),
        None => field.to_owned(),
    }
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
        /// Name of this step's worktree (branch `terminator/NAME`): made on
        /// the first run, and that same checkout on later runs. Unique in
        /// a flow.
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
    /// The flow's rules: one text for the whole flow, which every step's
    /// agent gets before its prompt.
    pub rules: Option<String>,
    /// The run inputs by name: `{{inputs.NAME}}` in the texts.
    pub inputs: BTreeMap<String, Input>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, Input>,
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
            rules: None,
            inputs: BTreeMap::new(),
            steps,
        }
    }

    /// The checked flow of this draft.
    pub fn check(&self) -> Result<Flow, FlowError> {
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
        let rules = match self.rules.as_deref().map(str::trim) {
            Some("") | None => None,
            Some(text) if !text_ok(text, true) => return Err(FlowError::Rules),
            Some(text) => Some(text.to_owned()),
        };
        inputs::checked(&self.inputs)?;
        let steps = self.steps.iter().map(step).collect::<Result<_, _>>()?;
        let inputs = self
            .inputs
            .iter()
            .map(|(name, input)| {
                let description = input.description.as_deref().map(str::trim);
                let input = Input {
                    description: description.filter(|text| !text.is_empty()).map(Into::into),
                    default: input.default.clone(),
                };
                (name.clone(), input)
            })
            .collect();
        let flow = Flow {
            name: name.to_owned(),
            rules,
            inputs,
            steps,
        };
        flow.check_graph()?;
        flow.check_worktrees()?;
        flow.check_placeholders()?;
        Ok(flow)
    }

    /// The file text of a correct draft. A draft that is not a correct
    /// flow gives its error and no text.
    pub fn to_toml(&self) -> Result<String, FlowError> {
        self.check()?;
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
        Self {
            rules: flow.rules.clone(),
            inputs: flow.inputs.clone(),
            ..Self::new(&flow.name, steps.collect())
        }
    }
}

fn step(raw: &DraftStep) -> Result<Step, FlowError> {
    let id = raw.id.as_str();
    if !id_ok(id) {
        return Err(FlowError::BadId(id.to_owned()));
    }
    let kind = match (&raw.command, &raw.gate) {
        (Some(command), None) => StepKind::Command {
            command: checked(id, "command", command, false)?,
            prompt: raw
                .prompt
                .as_deref()
                .map(|prompt| checked(id, "prompt", prompt, true))
                .transpose()?,
            cwd: raw
                .cwd
                .as_deref()
                .map(|cwd| checked_cwd(id, cwd))
                .transpose()?,
            worktree: raw
                .worktree
                .as_deref()
                .map(|name| checked_worktree(id, name))
                .transpose()?,
        },
        (None, Some(message)) => {
            if raw.prompt.is_some() || raw.cwd.is_some() || raw.worktree.is_some() {
                return Err(FlowError::GateField(id.to_owned()));
            }
            StepKind::Gate {
                message: checked(id, "gate", message, true)?,
            }
        }
        _ => return Err(FlowError::Kind(id.to_owned())),
    };
    Ok(Step {
        id: id.to_owned(),
        needs: raw.needs.clone(),
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

/// A relative folder that stays inside the project: only plain names and
/// `.`, never `..`, a root or a prefix. Shared with project tasks.
pub fn inside_project(path: &Path) -> bool {
    path.components()
        .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

fn checked_cwd(id: &str, cwd: &str) -> Result<String, FlowError> {
    let cwd = checked(id, "cwd", cwd, false)?;
    if inside_project(Path::new(&cwd)) {
        Ok(cwd)
    } else {
        Err(FlowError::Cwd(id.to_owned()))
    }
}

/// The same names as `ctl spawn --worktree` ([`crate::worktrees::name_ok`]).
fn checked_worktree(id: &str, name: &str) -> Result<String, FlowError> {
    if crate::worktrees::name_ok(name) {
        Ok(name.to_owned())
    } else {
        Err(FlowError::Worktree(id.to_owned()))
    }
}

impl Flow {
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|step| step.id == id)
    }

    /// A step's prompt with the flow's rules before it.
    pub fn full_prompt(&self, prompt: &str) -> String {
        match &self.rules {
            Some(rules) => format!("{rules}\n\n{prompt}"),
            None => prompt.to_owned(),
        }
    }

    /// One step per worktree name. Names that differ only in case are the
    /// same folder on a file system that ignores case.
    fn check_worktrees(&self) -> Result<(), FlowError> {
        let mut seen = BTreeMap::new();
        for step in &self.steps {
            let StepKind::Command {
                worktree: Some(name),
                ..
            } = &step.kind
            else {
                continue;
            };
            if let Some(first) = seen.insert(name.to_ascii_lowercase(), step.id.as_str()) {
                return Err(FlowError::DuplicateWorktree {
                    step: step.id.clone(),
                    first: first.to_owned(),
                    name: name.clone(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod inputs_tests;
#[cfg(test)]
mod tests;
