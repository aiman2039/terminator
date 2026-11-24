//! Best-of-N race steps (`count = N` or `agents = [...]`) with a `pick` gate,
//! plus deterministic per-worktree port allocation (`{{worktree.port}}`).
use super::{Flow, FlowError, StepKind, StepStatus, inputs};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Bounds on a race step's count or agent list.
pub const MIN_RACERS: u32 = 2;
pub const MAX_RACERS: u32 = 5;
/// Placeholder replaced with each racer's agent name in `agents = [...]`.
pub const AGENT_PLACEHOLDER: &str = "{{agent}}";
/// Placeholder replaced with the deterministic port of a step's managed worktree.
pub const PORT_PLACEHOLDER: &str = "{{worktree.port}}";
const PORT_BASE: u16 = 40_000;
const PORT_SPAN: u16 = 10_000;

/// Best-of-N race configuration on a command step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Race {
    /// Run `N` identical racers (`2..=5`) in worktrees `NAME-1..=NAME-N`.
    Count(u32),
    /// Run one racer per agent (`2..=5`), substituting `{{agent}}`.
    Agents(Vec<String>),
}

impl Race {
    pub fn len(&self) -> usize {
        match self {
            Self::Count(n) => *n as usize,
            Self::Agents(agents) => agents.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// One entry per racer (`None` for `Count`, `Some(agent)` for `Agents`).
    pub fn entries(&self) -> Vec<Option<String>> {
        match self {
            Self::Count(n) => vec![None; *n as usize],
            Self::Agents(agents) => agents.iter().cloned().map(Some).collect(),
        }
    }

    /// Worktree name for 1-based racer `index`.
    pub fn worktree_name(base: &str, index: u32) -> String {
        format!("{base}-{index}")
    }
}

/// Runtime state of one racer inside a race step.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RacerRun {
    /// 1-based racer index (`1..=N`).
    pub index: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub worktree: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<PathBuf>,
    pub status: StepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waits_for: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exited: bool,
}

/// Validate `count` and `agents` on a command step.
pub(super) fn checked_race(
    raw: &super::DraftStep,
    worktree: Option<&str>,
    (command, prompt): (&str, Option<&str>),
) -> Result<Option<Race>, FlowError> {
    let id = raw.id.as_str();
    let count = raw.count;
    let agents = raw.agents.as_slice();
    if count.is_some() && !agents.is_empty() {
        return Err(FlowError::Race {
            step: id.to_owned(),
            problem: "cannot set both count and agents".into(),
        });
    }
    let race = if let Some(n) = count {
        if !(MIN_RACERS..=MAX_RACERS).contains(&n) {
            return Err(FlowError::Race {
                step: id.to_owned(),
                problem: format!("count must be between {MIN_RACERS} and {MAX_RACERS}"),
            });
        }
        Race::Count(n)
    } else if !agents.is_empty() {
        let len = agents.len();
        if !(MIN_RACERS as usize..=MAX_RACERS as usize).contains(&len) {
            return Err(FlowError::Race {
                step: id.to_owned(),
                problem: format!("agents must have {MIN_RACERS} to {MAX_RACERS} entries"),
            });
        }
        let mut clean = Vec::with_capacity(len);
        for agent in agents {
            let trimmed = agent.trim();
            if trimmed.is_empty() || trimmed.len() > 64 || trimmed.chars().any(char::is_control) {
                return Err(FlowError::Race {
                    step: id.to_owned(),
                    problem: format!("invalid agent \"{agent}\""),
                });
            }
            clean.push(trimmed.to_owned());
        }
        let uses_placeholder = command.contains(AGENT_PLACEHOLDER)
            || prompt.is_some_and(|p| p.contains(AGENT_PLACEHOLDER));
        if !uses_placeholder {
            return Err(FlowError::Race {
                step: id.to_owned(),
                problem: format!("agents requires {AGENT_PLACEHOLDER} in command or prompt"),
            });
        }
        Race::Agents(clean)
    } else {
        return Ok(None);
    };
    let Some(base) = worktree else {
        return Err(FlowError::Race {
            step: id.to_owned(),
            problem: "race steps require worktree = NAME".into(),
        });
    };
    for i in 1..=u32::try_from(race.len()).unwrap_or(MAX_RACERS) {
        let wt = Race::worktree_name(base, i);
        if !crate::worktrees::name_ok(&wt) {
            return Err(FlowError::Worktree(id.to_owned()));
        }
    }
    Ok(Some(race))
}

/// Validate `pick` on a gate step (cannot combine with `options`).
pub(super) fn checked_pick(
    id: &str,
    pick: Option<&str>,
    options: &[String],
) -> Result<Option<String>, FlowError> {
    let Some(target) = pick.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if !options.is_empty() {
        return Err(FlowError::Pick {
            step: id.to_owned(),
            problem: "cannot set both options and pick".into(),
        });
    }
    if !super::id_ok(target) {
        return Err(FlowError::Pick {
            step: id.to_owned(),
            problem: format!("invalid step ID \"{target}\""),
        });
    }
    Ok(Some(target.to_owned()))
}

/// Deterministic, stable local port in `40000..=49999` for a managed worktree.
pub fn worktree_port(project_root: &Path, worktree: &str) -> u16 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in project_root
        .to_string_lossy()
        .as_bytes()
        .iter()
        .chain(b":")
        .chain(worktree.as_bytes())
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    PORT_BASE.saturating_add(u16::try_from(hash.rem_euclid(u64::from(PORT_SPAN))).unwrap_or(0))
}

/// Substitute runtime placeholders (`{{agent}}`, `{{worktree.port}}`) when
/// launching a command step or racer, shell-quoting when `shell` is true.
pub fn fill_runtime(
    text: &str,
    agent: Option<&str>,
    port: Option<u16>,
    shell: bool,
) -> Result<String, String> {
    let port_str = port.map(|p| p.to_string());
    let mut extras: Vec<(&str, Option<&str>)> = Vec::new();
    if let Some(agent) = agent {
        extras.push(("agent", Some(agent)));
    }
    if let Some(ref p) = port_str {
        extras.push(("worktree.port", Some(p.as_str())));
    }
    inputs::fill_runtime(text, &extras, shell)
}

impl Flow {
    /// One step per worktree name (including expanded `NAME-i` racer worktrees).
    pub(super) fn check_worktrees(&self) -> Result<(), FlowError> {
        let mut seen = BTreeMap::new();
        for step in &self.steps {
            let StepKind::Command {
                worktree: Some(name),
                race,
                ..
            } = &step.kind
            else {
                continue;
            };
            let names: Vec<String> = match race {
                Some(race) => (1..=u32::try_from(race.len()).unwrap_or(MAX_RACERS))
                    .map(|i| Race::worktree_name(name, i))
                    .collect(),
                None => vec![name.clone()],
            };
            for wt in names {
                if let Some(first) = seen.insert(wt.to_ascii_lowercase(), step.id.as_str()) {
                    return Err(FlowError::DuplicateWorktree {
                        step: step.id.clone(),
                        first: first.to_owned(),
                        name: wt,
                    });
                }
            }
        }
        Ok(())
    }

    /// Each `pick = "STEP"` gate must name a needed race step.
    pub(super) fn check_picks(&self) -> Result<(), FlowError> {
        for step in &self.steps {
            let StepKind::Gate {
                pick: Some(target), ..
            } = &step.kind
            else {
                continue;
            };
            if !step.needs.iter().any(|n| n == target) {
                return Err(FlowError::Pick {
                    step: step.id.clone(),
                    problem: format!("pick target \"{target}\" must be in needs"),
                });
            }
            let is_race = self
                .step(target)
                .is_some_and(|s| matches!(s.kind, StepKind::Command { race: Some(_), .. }));
            if !is_race {
                return Err(FlowError::Pick {
                    step: step.id.clone(),
                    problem: format!(
                        "pick target \"{target}\" must be a race step (count or agents)"
                    ),
                });
            }
        }
        Ok(())
    }
}
