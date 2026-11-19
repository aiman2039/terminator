//! The state of one flow run: a JSON file that the runner writes and the
//! GUI reads. It records what happened; it starts nothing.
use super::{Flow, Step, StepKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Run folders, relative to the data folder: `flows/RUN/state.json`.
pub const RUN_DIR: &str = "flows";
/// The state file in a run folder.
pub const STATE_FILE: &str = "state.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// Not started: a needed step is not done, or the runner did not reach it.
    #[default]
    Waiting,
    /// The terminal exists, or the gate waits for an answer.
    Running,
    Done,
    /// The agent failed or the gate was rejected.
    Failed,
    /// The user cancelled the run before this step ended.
    Cancelled,
}

/// One step of a run, with the display data of its flow step.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRun {
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    /// The question of a gate step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    /// The step's terminal session, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// A question of the step's agent that waits for the user's answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// Why the step failed, or the result note of a done step
    /// (`flow report done --note`), which the steps that need it get.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// What the runner waits for before it continues a running step, for
    /// example the agent's first hook report. Display only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits_for: Option<String>,
}

/// Where a run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A step runs or can start.
    Active,
    /// Every step is done.
    Done,
    /// No step runs and no step can start, but not all are done.
    Stopped,
}

/// One run of a flow.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunState {
    pub version: u32,
    /// The flow's name, for display.
    pub flow: String,
    /// The flow file name without `.toml`: what finds the runs of one flow
    /// file. A state file without it is not readable.
    pub file: String,
    /// The project's ID.
    pub project: String,
    /// The runner ended; nothing changes after this.
    #[serde(default)]
    pub ended: bool,
    /// Step IDs in file order.
    pub order: Vec<String>,
    pub steps: BTreeMap<String, StepRun>,
}

impl RunState {
    /// A run of flow file `file` (without `.toml`) of `project`, in which
    /// every step waits.
    pub fn new(flow: &Flow, file: &str, project: &str) -> Self {
        let step_run = |step: &Step| StepRun {
            needs: step.needs.clone(),
            gate: match &step.kind {
                StepKind::Gate { message } => Some(message.clone()),
                StepKind::Command { .. } => None,
            },
            ..StepRun::default()
        };
        Self {
            version: super::VERSION,
            flow: flow.name.clone(),
            file: file.to_owned(),
            project: project.to_owned(),
            ended: false,
            order: flow.steps.iter().map(|step| step.id.clone()).collect(),
            steps: flow
                .steps
                .iter()
                .map(|step| (step.id.clone(), step_run(step)))
                .collect(),
        }
    }

    /// A step the file does not name waits.
    pub fn status(&self, id: &str) -> StepStatus {
        self.steps
            .get(id)
            .map(|step| step.status)
            .unwrap_or_default()
    }

    /// Set a step's status and keep its other data.
    pub fn set(&mut self, id: &str, status: StepStatus) {
        self.steps.entry(id.to_owned()).or_default().status = status;
    }

    /// End a step as failed, with the reason.
    pub fn fail(&mut self, id: &str, note: impl Into<String>) {
        let step = self.steps.entry(id.to_owned()).or_default();
        step.status = StepStatus::Failed;
        step.note = Some(note.into());
    }

    /// Mark every step that did not end as cancelled. A cancelled step
    /// waits for nothing and has no open question.
    pub fn cancel(&mut self) {
        for step in self.steps.values_mut() {
            if matches!(step.status, StepStatus::Waiting | StepStatus::Running) {
                step.status = StepStatus::Cancelled;
                step.waits_for = None;
                step.question = None;
            }
        }
    }

    /// The runner stopped on an error: a running step fails with `note`
    /// (its terminal stays), and a step that did not start is cancelled.
    pub fn abort(&mut self, note: &str) {
        for step in self.steps.values_mut() {
            if step.status == StepStatus::Running {
                step.status = StepStatus::Failed;
                step.note = Some(note.to_owned());
                step.waits_for = None;
                step.question = None;
            }
        }
        self.cancel();
    }
}

impl Flow {
    /// Steps that can start now: waiting, with every needed step done.
    pub fn ready(&self, state: &RunState) -> Vec<&Step> {
        self.steps
            .iter()
            .filter(|step| state.status(&step.id) == StepStatus::Waiting)
            .filter(|step| {
                step.needs
                    .iter()
                    .all(|needs| state.status(needs) == StepStatus::Done)
            })
            .collect()
    }

    pub fn outcome(&self, state: &RunState) -> Outcome {
        let status = |step: &Step| state.status(&step.id);
        if self
            .steps
            .iter()
            .all(|step| status(step) == StepStatus::Done)
        {
            Outcome::Done
        } else if self
            .steps
            .iter()
            .any(|step| status(step) == StepStatus::Running)
            || !self.ready(state).is_empty()
        {
            Outcome::Active
        } else {
            Outcome::Stopped
        }
    }
}
