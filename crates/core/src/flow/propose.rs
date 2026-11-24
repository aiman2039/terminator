//! Dynamic steps proposed by a running step's agent (`flow propose` /
//! `flow_propose`). Each proposal is validated against the flow's strict
//! step rules before the runner adds it and starts it automatically once
//! its `needs` are satisfied.
use super::{
    Flow, FlowError, MAX_STEPS, RunState, Step, StepKind, StepRun, checked, checked_cwd,
    checked_worktree,
    control::{RunError, open, write},
    id_ok,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Folder of pending agent-proposed steps inside a run folder.
pub const PROPOSALS: &str = "proposals";

/// One step proposed by a running agent during a flow run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedStep {
    pub id: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    pub proposed_by: String,
}

impl ProposedStep {
    /// Validate fields into a checked [`Step`].
    pub fn into_step(&self) -> Result<Step, FlowError> {
        let id = self.id.trim();
        if !id_ok(id) {
            return Err(FlowError::BadId(id.to_owned()));
        }
        let command = checked(id, "command", &self.command, false)?;
        let prompt = self
            .prompt
            .as_deref()
            .map(|p| checked(id, "prompt", p, true))
            .transpose()?;
        let cwd = self
            .cwd
            .as_deref()
            .map(|c| checked_cwd(id, c))
            .transpose()?;
        let worktree = self
            .worktree
            .as_deref()
            .map(|w| checked_worktree(id, w))
            .transpose()?;
        Ok(Step {
            id: id.to_owned(),
            needs: self.needs.clone(),
            kind: StepKind::Command {
                command,
                prompt,
                cwd,
                worktree,
                race: None,
            },
            at: None,
            when: None,
            loop_back: None,
        })
    }
}

/// Write a validated proposal into `RUN/proposals/<id>` for the runner to
/// ingest on its next tick.
pub fn write_proposal(dir: &Path, proposal: &ProposedStep) -> Result<(), RunError> {
    let state = open(dir)?;
    let step = proposal
        .into_step()
        .map_err(|e| RunError::Proposal(e.to_string()))?;
    if state.steps.len() >= MAX_STEPS {
        return Err(RunError::Proposal(format!(
            "A flow has at most {MAX_STEPS} steps"
        )));
    }
    if state.steps.contains_key(&step.id) {
        return Err(RunError::Proposal(format!(
            "Step ID \"{}\" already exists",
            step.id
        )));
    }
    for need in &step.needs {
        if !state.steps.contains_key(need) {
            return Err(RunError::Proposal(format!(
                "Step \"{}\" needs unknown step \"{need}\"",
                step.id
            )));
        }
    }
    let bytes = serde_json::to_vec(&proposal).map_err(|_| RunError::State)?;
    write(dir, PROPOSALS, &step.id, &bytes)
}

/// Take all pending proposals from `RUN/proposals/` in sorted filename order.
pub fn take_proposals(dir: &Path) -> Vec<ProposedStep> {
    let mut files: Vec<_> = std::fs::read_dir(dir.join(PROPOSALS))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let parsed = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ProposedStep>(&bytes).ok());
        let _ = std::fs::remove_file(&path);
        if let Some(proposal) = parsed {
            out.push(proposal);
        }
    }
    out
}

impl Flow {
    /// Validate and append a dynamically proposed step to this running flow
    /// and its [`RunState`].
    pub fn add_proposed(
        &mut self,
        state: &mut RunState,
        proposal: ProposedStep,
    ) -> Result<String, FlowError> {
        if self.steps.len() >= MAX_STEPS {
            return Err(FlowError::TooManySteps);
        }
        let step = proposal.into_step()?;
        if self.step(&step.id).is_some() || state.steps.contains_key(&step.id) {
            return Err(FlowError::DuplicateId(step.id));
        }
        for need in &step.needs {
            if self.step(need).is_none() {
                return Err(FlowError::UnknownNeed {
                    step: step.id,
                    needs: need.clone(),
                });
            }
        }
        let id = step.id.clone();
        let needs = step.needs.clone();
        self.steps.push(step);
        if let Err(error) = self
            .check_graph()
            .and_then(|()| self.check_worktrees())
            .and_then(|()| self.check_placeholders())
        {
            self.steps.pop();
            return Err(error);
        }
        state.order.push(id.clone());
        state.steps.insert(
            id.clone(),
            StepRun {
                needs,
                proposed_by: Some(proposal.proposed_by),
                ..StepRun::default()
            },
        );
        Ok(id)
    }
}
