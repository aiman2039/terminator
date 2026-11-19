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
    /// Never started: its `when` option was not chosen, or every step it
    /// needs was skipped. The steps after it count it as ended.
    Skipped,
}

impl StepStatus {
    /// Done or skipped: the steps that need it can go on.
    pub fn passed(self) -> bool {
        matches!(self, Self::Done | Self::Skipped)
    }
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
    /// The attempt of this step: 1 (also 0, in older files) is the first,
    /// and each Retry adds one.
    #[serde(default, skip_serializing_if = "first_attempt")]
    pub attempt: u32,
    /// When this attempt started, in seconds since 1970. Display only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<u64>,
    /// The step failed because its terminal ended with no report (no hook
    /// state and no `flow report`). Display only.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exited: bool,
    /// The options of a gate step (empty: a plain Approve).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    /// The option the user chose at this gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    /// The step's `when` condition, "GATE.OPTION". Display only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// The step that this loop step runs again from. Display only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_to: Option<String>,
    /// The round of a step in a loop: 1 (also 0) is the first.
    #[serde(default, skip_serializing_if = "first_attempt")]
    pub round: u32,
    /// The most rounds of the loop this step is in; 0 when it is in none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_rounds: u32,
    /// The loop step's note at the end of the previous round, which the
    /// first step of the loop gets with its prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_note: Option<String>,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference."
)]
fn first_attempt(attempt: &u32) -> bool {
    *attempt <= 1
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if passes a reference."
)]
fn is_zero(value: &u32) -> bool {
    *value == 0
}

impl StepRun {
    /// The attempt number, 1 for the first.
    pub fn attempt(&self) -> u32 {
        self.attempt.max(1)
    }

    /// The round number, 1 for the first.
    pub fn round(&self) -> u32 {
        self.round.max(1)
    }

    /// "round 2 of 3" for a step in a loop.
    pub fn round_label(&self) -> Option<String> {
        (self.max_rounds > 0).then(|| format!("round {} of {}", self.round(), self.max_rounds))
    }

    /// This step as a new attempt that waits: the flow's data, the round
    /// and the loop note stay; the rest (session, notes, choice) goes.
    pub(super) fn again(&mut self, attempt: u32) {
        *self = Self {
            needs: std::mem::take(&mut self.needs),
            gate: self.gate.take(),
            options: std::mem::take(&mut self.options),
            when: self.when.take(),
            loop_to: self.loop_to.take(),
            round: self.round,
            max_rounds: self.max_rounds,
            loop_note: self.loop_note.take(),
            attempt,
            ..Self::default()
        };
    }

    /// A failed or cancelled step: a live runner can start it again.
    pub fn retryable(&self) -> bool {
        matches!(self.status, StepStatus::Failed | StepStatus::Cancelled)
    }
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
    /// The run's input values by name (`flow run --set`, and defaults).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, String>,
    /// Step IDs in file order.
    pub order: Vec<String>,
    pub steps: BTreeMap<String, StepRun>,
}

impl RunState {
    /// A run of flow file `file` (without `.toml`) of `project`, in which
    /// every step waits.
    pub fn new(flow: &Flow, file: &str, project: &str) -> Self {
        let rounds = flow.loop_rounds();
        let step_run = |step: &Step| {
            let (gate, options) = match &step.kind {
                StepKind::Gate { message, options } => (Some(message.clone()), options.clone()),
                StepKind::Command { .. } => (None, Vec::new()),
            };
            StepRun {
                needs: step.needs.clone(),
                gate,
                options,
                when: step.when.as_ref().map(ToString::to_string),
                loop_to: step.loop_back.as_ref().map(|back| back.to.clone()),
                max_rounds: rounds.get(&step.id).copied().unwrap_or(0),
                ..StepRun::default()
            }
        };
        Self {
            version: super::VERSION,
            flow: flow.name.clone(),
            file: file.to_owned(),
            project: project.to_owned(),
            ended: false,
            inputs: BTreeMap::new(),
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

    /// Retry a failed or cancelled step: it waits again as its next
    /// attempt, with no session (the old terminal stays, and nothing uses
    /// it again). The steps after it that did not start wait again. False
    /// for a step that is not failed or cancelled.
    pub fn retry(&mut self, id: &str) -> bool {
        let Some(step) = self.steps.get_mut(id).filter(|step| step.retryable()) else {
            return false;
        };
        let attempt = step.attempt().saturating_add(1);
        step.again(attempt);
        for after in self.after(id) {
            if let Some(step) = self.steps.get_mut(&after)
                && matches!(step.status, StepStatus::Waiting | StepStatus::Cancelled)
            {
                step.status = StepStatus::Waiting;
                step.waits_for = None;
                step.question = None;
            }
        }
        true
    }

    /// The steps that need `id`, also through other steps.
    pub(super) fn after(&self, id: &str) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        let mut next = vec![id.to_owned()];
        while let Some(from) = next.pop() {
            for (other, step) in &self.steps {
                if step.needs.contains(&from) && !found.contains(other) {
                    found.push(other.clone());
                    next.push(other.clone());
                }
            }
        }
        found
    }

    /// A step failed and the run's runner waits for Retry or Cancel: no
    /// step runs and no step can start.
    pub fn held(&self) -> bool {
        let status = |id: &String| self.status(id);
        let running = self
            .steps
            .values()
            .any(|step| step.status == StepStatus::Running);
        let ready = self.steps.values().any(|step| {
            step.status == StepStatus::Waiting && step.needs.iter().all(|id| status(id).passed())
        });
        !self.ended && !running && !ready && self.steps.values().any(StepRun::retryable)
    }
}

impl Flow {
    /// Steps that can start now: waiting, with every needed step done or
    /// skipped, and not a step to skip (see [`Flow::skip`]).
    pub fn ready(&self, state: &RunState) -> Vec<&Step> {
        self.steps
            .iter()
            .filter(|step| self.decide(step, state) == Some(false))
            .collect()
    }

    pub fn outcome(&self, state: &RunState) -> Outcome {
        let status = |step: &Step| state.status(&step.id);
        if self.steps.iter().all(|step| status(step).passed()) {
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
