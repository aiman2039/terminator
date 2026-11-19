//! Run inputs of a flow: `[inputs.NAME]` tables with an optional
//! `description` and `default` (no default: the input is required). A
//! prompt, a command, a gate message and the rules can use
//! `{{inputs.NAME}}`. Values are plain text in prompts, gate messages and
//! rules. In a command, a value is one shell-quoted word, so a value can
//! never add shell syntax. A run gets its values from `flow run --set
//! NAME=VALUE` (or the GUI's form) before anything starts.
use super::{Flow, FlowError, MAX_TEXT, StepKind, id_ok, text_ok};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Most inputs in one flow.
pub const MAX_INPUTS: usize = 32;
const OPEN: &str = "{{";
const CLOSE: &str = "}}";
const PREFIX: &str = "inputs.";

/// One declared input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The value when the run sets none. No default: the run must set it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

impl Input {
    pub fn required(&self) -> bool {
        self.default.is_none()
    }
}

/// Why a run's values are refused. Nothing starts.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InputError {
    #[error("--set takes NAME=VALUE, not \"{0}\"")]
    Form(String),
    #[error("This flow has no input \"{0}\"")]
    Unknown(String),
    #[error("Input \"{0}\" is set two times")]
    Twice(String),
    #[error("Input \"{0}\" is required: set it with --set {0}=VALUE")]
    Missing(String),
    #[error("The value of input \"{0}\" is too long or has a control character")]
    Value(String),
}

/// A value is one line of at most 16 KiB with no control character: a
/// value is typed into a terminal, where a control character is a key.
pub fn value_ok(value: &str) -> bool {
    value.len() <= MAX_TEXT && !value.chars().any(char::is_control)
}

/// `NAME=VALUE` of `--set`.
pub fn parse_set(text: &str) -> Result<(String, String), InputError> {
    match text.split_once('=') {
        Some((name, value)) if !name.trim().is_empty() => {
            Ok((name.trim().to_owned(), value.to_owned()))
        }
        _ => Err(InputError::Form(text.to_owned())),
    }
}

/// Checked inputs of a file: names like step IDs, bounded text.
pub(super) fn checked(inputs: &BTreeMap<String, Input>) -> Result<(), FlowError> {
    if inputs.len() > MAX_INPUTS {
        return Err(FlowError::TooManyInputs);
    }
    for (name, input) in inputs {
        if !id_ok(name) {
            return Err(FlowError::BadInput(name.clone()));
        }
        if input
            .description
            .as_deref()
            .is_some_and(|text| !text_ok(text.trim(), false))
        {
            return Err(FlowError::InputText(name.clone(), "description"));
        }
        if input.default.as_deref().is_some_and(|text| !value_ok(text)) {
            return Err(FlowError::InputText(name.clone(), "default"));
        }
    }
    Ok(())
}

/// Where the quote state of a command is, at a place in its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Quote {
    /// Plain words, also inside `$( )` or `( )`.
    Plain,
    Single,
    Double,
    /// `$'…'`, `${…}` or backticks: no value may go there.
    Other,
}

/// The quote state at the end of `text`. A stack of open constructs:
/// `$(` and `(` reset to plain words until their `)`.
fn quote_at(text: &str) -> Quote {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Open {
        Paren,
        Single,
        Double,
        Ansi,
        Tick,
        Brace,
    }
    let mut stack: Vec<Open> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let top = stack.last().copied();
        match (top, c) {
            (Some(Open::Single | Open::Ansi), '\'') => {
                stack.pop();
            }
            (Some(Open::Single), _) => {}
            (None | Some(Open::Double | Open::Ansi | Open::Tick | Open::Paren), '\\') => {
                chars.next();
            }
            (Some(Open::Double), '"') | (Some(Open::Tick), '`') | (Some(Open::Brace), '}') => {
                stack.pop();
            }
            (Some(Open::Ansi | Open::Tick), _) => {}
            (Some(Open::Paren), ')') => {
                stack.pop();
            }
            (_, '$') => match chars.peek() {
                Some('(') => {
                    chars.next();
                    stack.push(Open::Paren);
                }
                Some('{') => {
                    chars.next();
                    stack.push(Open::Brace);
                }
                Some('\'') if top != Some(Open::Double) => {
                    chars.next();
                    stack.push(Open::Ansi);
                }
                _ => {}
            },
            (_, '`') => stack.push(Open::Tick),
            (Some(Open::Double), _) => {}
            (_, '"') => stack.push(Open::Double),
            (_, '\'') => stack.push(Open::Single),
            (_, '(') => stack.push(Open::Paren),
            _ => {}
        }
    }
    match stack.last() {
        None | Some(Open::Paren) => Quote::Plain,
        Some(Open::Single) => Quote::Single,
        Some(Open::Double) => Quote::Double,
        Some(Open::Ansi | Open::Tick | Open::Brace) => Quote::Other,
    }
}

/// `value` as shell text at a place with quote state `quote`: one word
/// that the shell takes as plain text. `None` where no value may go.
fn shell_value(value: &str, quote: Quote) -> Option<String> {
    let single = crate::quote(value);
    match quote {
        Quote::Plain => Some(single),
        // Close the double quotes, add the single-quoted word, open again.
        Quote::Double => Some(format!("\"{single}\"")),
        // In single quotes only a quote ends the text.
        Quote::Single => Some(value.replace('\'', "'\\''")),
        Quote::Other => None,
    }
}

/// `text` with each `{{inputs.NAME}}` replaced by the value of NAME. In a
/// command (`shell`) each value is shell-quoted for its place. The error
/// is the problem, for [`FlowError::Placeholder`].
pub(super) fn fill(
    text: &str,
    values: &BTreeMap<String, String>,
    shell: bool,
) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after)) = rest.split_once(OPEN) {
        out.push_str(before);
        let (inner, next) = after
            .split_once(CLOSE)
            .ok_or_else(|| "a {{ has no }}".to_owned())?;
        let Some(name) = inner.strip_prefix(PREFIX) else {
            return Err(format!(
                "unknown placeholder {{{{{inner}}}}}; use {{{{inputs.NAME}}}}"
            ));
        };
        let value = values
            .get(name)
            .ok_or_else(|| format!("input \"{name}\" is not in [inputs]"))?;
        if shell {
            let quoted = shell_value(value, quote_at(&out)).ok_or_else(|| {
                format!("input \"{name}\" cannot be inside $'…', ${{…}} or backticks")
            })?;
            out.push_str(&quoted);
        } else {
            out.push_str(value);
        }
        rest = next;
    }
    out.push_str(rest);
    Ok(out)
}

impl Flow {
    /// Each text uses only declared inputs, at a place where a value can
    /// go. Checked with empty values.
    pub(super) fn check_placeholders(&self) -> Result<(), FlowError> {
        let empty: BTreeMap<String, String> = self
            .inputs
            .keys()
            .map(|name| (name.clone(), String::new()))
            .collect();
        self.with_values(&empty).map(|_| ())
    }

    /// The flow with `values` in its texts. `values` must have every
    /// declared input ([`Flow::input_values`]).
    pub fn with_values(&self, values: &BTreeMap<String, String>) -> Result<Self, FlowError> {
        fn place(step: Option<&str>, field: &'static str) -> impl FnOnce(String) -> FlowError {
            let step = step.map(str::to_owned);
            move |problem| FlowError::Placeholder {
                step,
                field,
                problem,
            }
        }
        let mut flow = self.clone();
        if let Some(rules) = &mut flow.rules {
            *rules = fill(rules, values, false).map_err(place(None, "rules"))?;
        }
        for step in &mut flow.steps {
            let id = Some(step.id.as_str());
            match &mut step.kind {
                StepKind::Command {
                    command, prompt, ..
                } => {
                    *command = fill(command, values, true).map_err(place(id, "command"))?;
                    if let Some(prompt) = prompt {
                        *prompt = fill(prompt, values, false).map_err(place(id, "prompt"))?;
                    }
                }
                StepKind::Gate { message } => {
                    *message = fill(message, values, false).map_err(place(id, "gate"))?;
                }
            }
        }
        Ok(flow)
    }

    /// The values of a run: each `--set` (a declared input, once, with a
    /// good value), then the defaults. A required input with no value
    /// refuses the run.
    pub fn input_values(
        &self,
        sets: &[(String, String)],
    ) -> Result<BTreeMap<String, String>, InputError> {
        let mut values = BTreeMap::new();
        for (name, value) in sets {
            if !self.inputs.contains_key(name) {
                return Err(InputError::Unknown(name.clone()));
            }
            if !value_ok(value) {
                return Err(InputError::Value(name.clone()));
            }
            if values.insert(name.clone(), value.clone()).is_some() {
                return Err(InputError::Twice(name.clone()));
            }
        }
        for (name, input) in &self.inputs {
            if values.contains_key(name) {
                continue;
            }
            let value = input
                .default
                .clone()
                .ok_or_else(|| InputError::Missing(name.clone()))?;
            values.insert(name.clone(), value);
        }
        Ok(values)
    }
}
