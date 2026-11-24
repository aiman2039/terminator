//! Best-of-N race execution (`count = N` or `agents = [...]`) and picked
//! worktree propagation for downstream steps.
use super::{
    super::{
        agents::latest_mark,
        lookup::{Ctl, NewSession},
    },
    agent,
    cancel::Cancelled,
    progress::{ENDED, Phase, Progress, Wait, progress, with_results},
    runner::{End, Runner, SETTLE_SECS, START_LIMIT, WAITS_FOR_HOOK},
};
use crate::output::Output;
use anyhow::{Result, bail};
use std::{
    collections::VecDeque,
    path::Path,
    process::Command,
    time::{Duration, Instant},
};
use terminator_core::{
    Project, State,
    flow::{
        Race, RacerRun, Step, StepKind, StepStatus,
        race::{fill_runtime, worktree_port},
    },
    now, ui_control,
};

/// Outcome of polling one active racer.
enum RacerEnd {
    Done {
        note: Option<String>,
        outputs: Vec<String>,
    },
    Failed {
        note: String,
        exited: bool,
    },
}

/// One active racer tracked inside [`Phase::Race`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RacerPhase {
    pub(super) index: u32,
    pub(super) wait: Wait,
    pub(super) started: Instant,
}

impl Runner {
    /// Launches all racers of a race step, each in its own managed worktree
    /// `NAME-i` on `terminator/NAME-i` and its own new terminal.
    pub(super) fn start_race(&mut self, step: &Step, race: &Race) -> Result<Phase> {
        let StepKind::Command {
            prompt,
            worktree: Some(_),
            ..
        } = &step.kind
        else {
            bail!("Race step requires worktree = NAME");
        };
        let prompt_template = prompt.as_deref().map(|p| self.handoff_prompt(step, p));
        let ctl = Ctl::connect(&self.paths)?;
        let entries = race.entries();
        let mut racers = Vec::with_capacity(entries.len());
        let mut active = Vec::with_capacity(entries.len());
        for (idx0, agent) in entries.into_iter().enumerate() {
            let index = u32::try_from(idx0.saturating_add(1)).unwrap_or(1);
            let (run_entry, phase_entry) = self.launch_one_racer(
                &ctl,
                (step, index, agent, prompt_template.as_deref()),
                &mut racers,
            )?;
            racers.push(run_entry);
            active.push(phase_entry);
        }
        Ok(Phase::Race(active))
    }

    fn launch_one_racer(
        &mut self,
        ctl: &Ctl,
        (step, index, agent, prompt_template): (&Step, u32, Option<String>, Option<&str>),
        existing: &mut Vec<RacerRun>,
    ) -> Result<(RacerRun, RacerPhase)> {
        let StepKind::Command {
            command,
            cwd,
            worktree: Some(base),
            ..
        } = &step.kind
        else {
            bail!("Race step requires worktree = NAME");
        };
        if self.cancelled() {
            return Err(Cancelled.into());
        }
        let wt_name = Race::worktree_name(base, index);
        let wt_project = ctl.step_worktree(&self.project, &wt_name)?;
        if self.cancelled() {
            return Err(Cancelled.into());
        }
        let port = worktree_port(&self.project.path, &wt_name);
        let cmd = fill_runtime(command, agent.as_deref(), Some(port), true)
            .map_err(|e| anyhow::anyhow!(e))?;
        let session = ctl.create_session(NewSession {
            project: &wt_project,
            cwd: cwd.as_ref().map(|sub| wt_project.path.join(sub)),
            file: None,
        })?;
        let (wait, waits_for) = match prompt_template {
            Some(tpl) => {
                let filled = fill_runtime(tpl, agent.as_deref(), Some(port), false)
                    .map_err(|e| anyhow::anyhow!(e))?;
                (
                    Wait::Starting { prompt: filled },
                    Some(WAITS_FOR_HOOK.to_owned()),
                )
            }
            None => (Wait::Working { since: None }, None),
        };
        let run_entry = RacerRun {
            index,
            agent,
            worktree: wt_name,
            worktree_path: Some(wt_project.path),
            status: StepStatus::Running,
            session: Some(session.id.clone()),
            waits_for,
            ..RacerRun::default()
        };
        if let Some(run) = self.state.steps.get_mut(&step.id) {
            run.racers.clone_from(existing);
            run.racers.push(run_entry.clone());
            run.started.get_or_insert_with(now);
        }
        self.state.set(&step.id, StepStatus::Running);
        self.changed = true;
        self.save()?;
        self.type_line(&session.id, &cmd)?;
        let _ = ui_control::rpc(
            &self.paths,
            ui_control::Request::ShowSession {
                session: session.id,
                anchor: None,
                split: None,
                background: true,
            },
        );
        Ok((
            run_entry,
            RacerPhase {
                index,
                wait,
                started: Instant::now(),
            },
        ))
    }

    /// Poll all active racers of step `id`. Finishes the race step once every
    /// racer has ended (Done if at least one succeeded, Failed if all failed).
    pub(super) fn poll_race(&mut self, id: &str, snapshot: Option<&State>) -> End {
        let mut active = match self.phases.get(id) {
            Some((Phase::Race(racers), _)) => racers.clone(),
            _ => return None,
        };
        let mut still_running = Vec::with_capacity(active.len());
        for mut racer in active.drain(..) {
            match self.poll_one_racer(id, &mut racer, snapshot) {
                None => still_running.push(racer),
                Some(end) => self.finish_racer(id, racer.index, end),
            }
        }
        if let Some((Phase::Race(slot), _)) = self.phases.get_mut(id) {
            slot.clone_from(&still_running);
        }
        if !still_running.is_empty() {
            return None;
        }
        let Some(run) = self.state.steps.get_mut(id) else {
            return Some(Err("Missing race step".into()));
        };
        let total = run.racers.len();
        let done = run
            .racers
            .iter()
            .filter(|r| r.status == StepStatus::Done)
            .count();
        if done > 0 {
            run.note = Some(format!("{done} of {total} racers finished"));
            Some(Ok(()))
        } else {
            Some(Err(format!("All {total} racers failed")))
        }
    }

    fn poll_one_racer(
        &mut self,
        id: &str,
        racer: &mut RacerPhase,
        snapshot: Option<&State>,
    ) -> Option<RacerEnd> {
        if let Some(report) = agent::read_racer_report(&self.dir, id, racer.index) {
            return Some(if report.done {
                RacerEnd::Done {
                    note: report.note,
                    outputs: report.outputs,
                }
            } else {
                RacerEnd::Failed {
                    note: report
                        .note
                        .unwrap_or_else(|| "The racer reported a failure".into()),
                    exited: false,
                }
            });
        }
        let snapshot = snapshot?;
        let session = self
            .state
            .steps
            .get(id)?
            .racers
            .iter()
            .find(|r| r.index == racer.index)?
            .session
            .clone()
            .unwrap_or_default();
        match progress(snapshot, &session, &racer.wait) {
            Progress::Done => Some(RacerEnd::Done {
                note: None,
                outputs: Vec::new(),
            }),
            Progress::Failed(note) => Some(RacerEnd::Failed {
                note: note.to_owned(),
                exited: note == ENDED,
            }),
            Progress::SendPrompt => self.send_racer_prompt((id, &session), racer, snapshot),
            Progress::Pending
                if matches!(racer.wait, Wait::Starting { .. })
                    && racer.started.elapsed() > START_LIMIT =>
            {
                Some(RacerEnd::Failed {
                    note: "No agent hook report in 120 seconds.".into(),
                    exited: false,
                })
            }
            Progress::Pending => None,
        }
    }

    fn send_racer_prompt(
        &mut self,
        (id, session): (&str, &str),
        racer: &mut RacerPhase,
        snapshot: &State,
    ) -> Option<RacerEnd> {
        let Wait::Starting { prompt } = &racer.wait else {
            return None;
        };
        let since = latest_mark(snapshot, session);
        let reported = since.as_ref().map_or(0, |mark| mark.0);
        if now() < reported.saturating_add(SETTLE_SECS) {
            return None;
        }
        if let Err(error) = self.type_line(session, &terminator_core::one_line(prompt)) {
            return Some(RacerEnd::Failed {
                note: format!("{error:#}"),
                exited: false,
            });
        }
        racer.wait = Wait::Working { since };
        racer.started = Instant::now();
        if let Some(item) = self
            .state
            .steps
            .get_mut(id)
            .and_then(|s| s.racers.iter_mut().find(|r| r.index == racer.index))
        {
            item.waits_for = None;
            self.changed = true;
        }
        None
    }

    fn finish_racer(&mut self, id: &str, index: u32, end: RacerEnd) {
        let (status, note, outputs, exited) = match end {
            RacerEnd::Done { note, outputs } => (StepStatus::Done, note, outputs, false),
            RacerEnd::Failed { note, exited } => {
                (StepStatus::Failed, Some(note), Vec::new(), exited)
            }
        };
        let wt_path = self
            .state
            .steps
            .get(id)
            .and_then(|s| s.racers.iter().find(|r| r.index == index))
            .and_then(|r| r.worktree_path.clone());
        let diffstat = if status == StepStatus::Done {
            wt_path
                .as_deref()
                .and_then(|wt| racer_diffstat(&self.project.path, wt))
        } else {
            None
        };
        if let Some(item) = self
            .state
            .steps
            .get_mut(id)
            .and_then(|s| s.racers.iter_mut().find(|r| r.index == index))
        {
            item.status = status;
            item.note = note;
            item.outputs = outputs;
            item.waits_for = None;
            item.exited = exited;
            if let Some((added, deleted)) = diffstat {
                item.added = Some(added);
                item.deleted = Some(deleted);
            }
            self.changed = true;
        }
        Output::line_or_drop(format_args!("{id}#{index}: {status:?}"));
    }

    /// Builds `prompt` with the flow rules and needed-step handoff notes/outputs.
    pub(super) fn handoff_prompt(&self, step: &Step, prompt: &str) -> String {
        let need_notes: Vec<(String, String)> = step
            .needs
            .iter()
            .filter_map(|needs| {
                let run = self.state.steps.get(needs)?;
                let text = match (run.note.as_deref(), run.outputs.is_empty()) {
                    (Some(note), true) => note.to_owned(),
                    (Some(note), false) => format!("{note} (outputs: {})", run.outputs.join(", ")),
                    (None, false) => format!("outputs: {}", run.outputs.join(", ")),
                    (None, true) => return None,
                };
                Some((needs.clone(), text))
            })
            .collect();
        let results = need_notes
            .iter()
            .map(|(step, text)| (step.as_str(), text.as_str()))
            .chain(self.loop_note(&step.id));
        with_results(&self.flow.full_prompt(prompt), results)
    }

    /// If `step` depends on a completed `pick` gate, returns a Project rooted
    /// at the winning racer's worktree so downstream steps run there.
    pub(super) fn picked_worktree_for(&self, step: &Step) -> Option<Project> {
        let mut queue: VecDeque<&str> = step.needs.iter().map(String::as_str).collect();
        let mut seen = std::collections::HashSet::new();
        while let Some(need) = queue.pop_front() {
            if !seen.insert(need) {
                continue;
            }
            if let Some(run) = self.state.steps.get(need) {
                if let (Some(target), Some(winner)) = (&run.pick, run.picked)
                    && let Some(path) = self
                        .state
                        .steps
                        .get(target)
                        .and_then(|race| race.racers.iter().find(|r| r.index == winner))
                        .and_then(|r| r.worktree_path.clone())
                {
                    let mut project = self.project.clone();
                    project.path = path;
                    return Some(project);
                }
                for parent in &run.needs {
                    queue.push_back(parent.as_str());
                }
            }
        }
        None
    }
}

/// Read-only `git diff --numstat` of a racer worktree against the source
/// checkout's HEAD (falling back to working tree diff). Best-effort.
fn racer_diffstat(source: &Path, worktree: &Path) -> Option<(u32, u32)> {
    let base = Command::new("git")
        .args(["-C", &source.to_string_lossy(), "rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty());
    let mut cmd = Command::new("git");
    cmd.args(["-C", &worktree.to_string_lossy(), "diff", "--numstat"]);
    if let Some(rev) = &base {
        cmd.arg(rev);
    }
    let output = terminator_core::run_command(
        cmd,
        terminator_core::CommandOptions {
            timeout: Duration::from_secs(5),
            ..terminator_core::CommandOptions::default()
        },
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let (mut added, mut deleted) = (0u32, 0u32);
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.split('\t');
        let a = parts
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        let d = parts
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(0);
        added = added.saturating_add(a);
        deleted = deleted.saturating_add(d);
    }
    Some((added, deleted))
}
