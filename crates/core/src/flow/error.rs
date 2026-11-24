//! Flow validation errors.
use super::{MAX_ID, MAX_OPTIONS, MAX_STEPS, MIN_OPTIONS, VERSION, inputs};

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
    #[error(
        "Step \"{0}\": options must be {MIN_OPTIONS} to {MAX_OPTIONS} different names of a-z, 0-9, - or _, on a gate step"
    )]
    Options(String),
    #[error("Step \"{step}\": when {problem}")]
    When { step: String, problem: String },
    #[error("Step \"{step}\": loop_to {problem}")]
    Loop { step: String, problem: String },
    #[error("Step \"{step}\": {problem}")]
    Race { step: String, problem: String },
    #[error("Step \"{step}\": {problem}")]
    Pick { step: String, problem: String },
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
