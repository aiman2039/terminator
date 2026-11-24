//! Branches and loops of a flow.
//!
//! A gate step can have `options` (2 to 8 names). The user approves it
//! with one option. A step after the gate can set `when = "GATE.OPTION"`:
//! it runs only when the gate's choice is that option. Else it is
//! skipped. A step whose needed steps were all skipped is skipped too. A
//! step that needs some done and some skipped steps runs.
//!
//! A step can set `loop_to = "STEP"` and `max_rounds = N` (2 to 10). STEP
//! must be a step that this step needs, directly or through other steps.
//! The loop's steps are STEP, this step, and each step between them. When
//! this step is done in round R and R < N, each loop step waits again as
//! round R + 1 (a new terminal; nothing closes or reuses a session). The
//! steps after the loop's steps that are not running wait again too. After
//! round N the step stays done and the run goes on. Two loops never share
//! a step. The file stays a graph without a cycle: `loop_to` is not a
//! `needs` edge.
use super::{Flow, FlowError, RunState, Step, StepKind, StepStatus, id_ok};
use std::collections::{BTreeMap, BTreeSet};

/// Fewest and most options of a gate.
pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 8;
/// Fewest and most rounds of a loop.
pub const MIN_ROUNDS: u32 = 2;
pub const MAX_ROUNDS: u32 = 10;

/// `when = "GATE.OPTION"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct When {
    pub gate: String,
    pub option: String,
}

impl std::fmt::Display for When {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.gate, self.option)
    }
}

/// `loop_to = "STEP"` with `max_rounds = N`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoopBack {
    pub to: String,
    pub max_rounds: u32,
}

/// The checked options of gate `id`: none, or 2 to 8 different IDs.
pub(super) fn options(id: &str, options: &[String]) -> Result<Vec<String>, FlowError> {
    if options.is_empty() {
        return Ok(Vec::new());
    }
    let unique: BTreeSet<&String> = options.iter().collect();
    let good = (MIN_OPTIONS..=MAX_OPTIONS).contains(&options.len())
        && unique.len() == options.len()
        && options.iter().all(|option| id_ok(option));
    if good {
        Ok(options.to_vec())
    } else {
        Err(FlowError::Options(id.to_owned()))
    }
}

/// `GATE.OPTION`, each part a good ID. The graph check finds the gate.
pub(super) fn when(id: &str, text: &str) -> Result<When, FlowError> {
    match text.split_once('.') {
        Some((gate, option)) if id_ok(gate) && id_ok(option) => Ok(When {
            gate: gate.to_owned(),
            option: option.to_owned(),
        }),
        _ => Err(FlowError::When {
            step: id.to_owned(),
            problem: format!("must be GATE.OPTION, not \"{text}\""),
        }),
    }
}

/// `loop_to` and `max_rounds` come together, with 2 to 10 rounds.
pub(super) fn loop_back(
    id: &str,
    to: Option<&str>,
    max_rounds: Option<u32>,
) -> Result<Option<LoopBack>, FlowError> {
    let problem = |problem: String| FlowError::Loop {
        step: id.to_owned(),
        problem,
    };
    match (to, max_rounds) {
        (None, None) => Ok(None),
        (Some(to), Some(max_rounds)) if (MIN_ROUNDS..=MAX_ROUNDS).contains(&max_rounds) => {
            Ok(Some(LoopBack {
                to: to.to_owned(),
                max_rounds,
            }))
        }
        (Some(_), Some(_)) => Err(problem(format!(
            "needs max_rounds from {MIN_ROUNDS} to {MAX_ROUNDS}"
        ))),
        _ => Err(problem("and max_rounds must be set together".to_owned())),
    }
}

impl Flow {
    /// `when` names a needed gate and one of its options; `loop_to` names
    /// an earlier step; two loops share no step. After the graph check.
    pub(super) fn check_branches(&self) -> Result<(), FlowError> {
        let mut owner: BTreeMap<&str, &str> = BTreeMap::new();
        for step in &self.steps {
            if let Some(when) = &step.when {
                self.check_when(step, when)?;
            }
            let Some(back) = &step.loop_back else {
                continue;
            };
            let problem = |problem: String| FlowError::Loop {
                step: step.id.clone(),
                problem,
            };
            if back.to == step.id || !self.before(&step.id).contains(back.to.as_str()) {
                return Err(problem(format!(
                    "\"{}\" must be a step that this step needs, directly or through other steps",
                    back.to
                )));
            }
            for member in self.loop_steps(step) {
                if let Some(other) = owner.insert(member, step.id.as_str()) {
                    return Err(problem(format!(
                        "shares step \"{member}\" with the loop of step \"{other}\""
                    )));
                }
            }
        }
        Ok(())
    }

    fn check_when(&self, step: &Step, when: &When) -> Result<(), FlowError> {
        let problem = |problem: String| FlowError::When {
            step: step.id.clone(),
            problem,
        };
        if !step.needs.contains(&when.gate) {
            return Err(problem(format!("gate \"{}\" must be in needs", when.gate)));
        }
        match self.step(&when.gate).map(|gate| &gate.kind) {
            Some(StepKind::Gate { options, .. }) if options.contains(&when.option) => Ok(()),
            Some(StepKind::Gate { options, .. }) if !options.is_empty() => Err(problem(format!(
                "option \"{}\" is not one of {}",
                when.option,
                options.join(", ")
            ))),
            _ => Err(problem(format!(
                "\"{}\" must be a gate step with options",
                when.gate
            ))),
        }
    }

    /// The steps that `id` needs, directly or through other steps.
    fn before(&self, id: &str) -> BTreeSet<&str> {
        let mut found = BTreeSet::new();
        let mut next = vec![id];
        while let Some(at) = next.pop() {
            for needs in self
                .step(at)
                .map(|step| step.needs.as_slice())
                .unwrap_or(&[])
            {
                if found.insert(needs.as_str()) {
                    next.push(needs);
                }
            }
        }
        found
    }

    /// The steps of the loop of `step`: its `loop_to` step, `step`, and
    /// each step between them. Empty for a step with no loop.
    fn loop_steps<'a>(&'a self, step: &'a Step) -> BTreeSet<&'a str> {
        let Some(back) = &step.loop_back else {
            return BTreeSet::new();
        };
        let mut steps = self.before(&step.id);
        steps.retain(|id| *id == back.to || self.before(id).contains(back.to.as_str()));
        steps.insert(step.id.as_str());
        steps
    }

    /// The most rounds of each step in a loop.
    pub fn loop_rounds(&self) -> BTreeMap<String, u32> {
        let mut rounds = BTreeMap::new();
        for step in &self.steps {
            let Some(back) = &step.loop_back else {
                continue;
            };
            for id in self.loop_steps(step) {
                rounds.insert(id.to_owned(), back.max_rounds);
            }
        }
        rounds
    }

    /// For a waiting step whose needed steps are all done or skipped:
    /// `Some(true)` to skip it, `Some(false)` to start it. `None` while it
    /// cannot be decided.
    pub(super) fn decide(&self, step: &Step, state: &RunState) -> Option<bool> {
        let status = |id: &str| state.status(id);
        if status(&step.id) != StepStatus::Waiting
            || !step.needs.iter().all(|id| status(id).passed())
        {
            return None;
        }
        let not_chosen = step.when.as_ref().is_some_and(|when| {
            let choice = state
                .steps
                .get(&when.gate)
                .and_then(|run| run.choice.as_deref());
            choice != Some(when.option.as_str())
        });
        let all_skipped = !step.needs.is_empty()
            && step
                .needs
                .iter()
                .all(|id| status(id) == StepStatus::Skipped);
        Some(not_chosen || all_skipped)
    }

    /// Marks the steps to skip now as skipped, also the steps after them
    /// that it decides. Gives their IDs in the order they were skipped.
    pub fn skip(&self, state: &mut RunState) -> Vec<String> {
        let mut skipped = Vec::new();
        loop {
            let next: Vec<String> = self
                .steps
                .iter()
                .filter(|step| self.decide(step, state) == Some(true))
                .map(|step| step.id.clone())
                .collect();
            if next.is_empty() {
                return skipped;
            }
            for id in next {
                state.set(&id, StepStatus::Skipped);
                skipped.push(id);
            }
        }
    }

    /// Step `id` is done. When it is a loop step in a round before its
    /// last, each loop step waits again as the next round, and the steps
    /// after them that do not run wait again. Gives the IDs that wait again
    /// (empty: no new round).
    pub fn loop_back(&self, state: &mut RunState, id: &str) -> Vec<String> {
        let Some(step) = self.step(id) else {
            return Vec::new();
        };
        let Some(back) = &step.loop_back else {
            return Vec::new();
        };
        let Some(run) = state.steps.get(id) else {
            return Vec::new();
        };
        let round = run.round();
        if run.status != StepStatus::Done || round >= back.max_rounds {
            return Vec::new();
        }
        let note = run.note.clone();
        let members = self.loop_steps(step);
        let mut again: Vec<String> = members.iter().map(|id| (*id).to_owned()).collect();
        for member in &members {
            for after in state.after(member) {
                let running = state.status(&after) == StepStatus::Running;
                if !running && !again.contains(&after) {
                    again.push(after);
                }
            }
        }
        for id in &again {
            let Some(run) = state.steps.get_mut(id) else {
                continue;
            };
            if members.contains(id.as_str()) {
                run.round = round.saturating_add(1);
                run.loop_note = None;
            } else {
                run.round = 1;
                run.loop_note = None;
            }
            run.again(1);
        }
        if let Some(target) = state.steps.get_mut(&back.to) {
            target.loop_note = note;
        }
        again
    }
}
