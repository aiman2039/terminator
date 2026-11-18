//! The Flows tool window: the runs of the selected project, one row per
//! step (`runs_view`). It reads the run state files on a worker thread
//! once per second while it shows. Its buttons write the user's answers
//! through `terminator_core::flow::control` on a worker thread, and Remove
//! and Clear ended runs delete ended run folders there; they never start a
//! step, never type into a terminal and never close a session.
use super::Run;
use crate::*;
use std::path::PathBuf;
use std::sync::mpsc::{TryRecvError, channel};
use terminator_core::flow::{
    RunState, StepRun, StepStatus,
    control::{self, RunError},
};

const READ_EVERY: Duration = Duration::from_secs(1);
/// Most runs shown for one project, newest first.
const SHOWN_RUNS: usize = 10;
/// Most run folders read each second, newest first, of all projects.
const READ_RUNS: usize = 100;
pub(super) const ROW_HEIGHT: f32 = 26.0;

/// One click in the panel.
pub(super) enum Act {
    RunFlow,
    Go(String),
    Gate(PathBuf, String, bool),
    Answer(PathBuf, String),
    AskCancel(PathBuf),
    KeepRun,
    Cancel(PathBuf),
    /// Show (true) or hide the steps of a run.
    Toggle(PathBuf, bool),
    /// Delete the folder of one ended run.
    Remove(PathBuf),
    AskClear,
    KeepRuns,
    /// Delete the folders of these ended runs.
    Clear(Vec<PathBuf>),
}

/// "running", "done" or "stopped".
pub(super) fn run_status(state: &RunState) -> &'static str {
    let done = state
        .steps
        .values()
        .all(|step| step.status == StepStatus::Done);
    match (state.ended, done) {
        (false, _) => "running",
        (true, true) => "done",
        (true, false) => "stopped",
    }
}

/// The icon and the hover text of a step row.
pub(super) fn step_icon(step: &StepRun) -> (&'static str, &'static str) {
    match step.status {
        _ if step.question.is_some() => ("MessageCircleQuestion", "Waits for your answer"),
        StepStatus::Running if step.gate.is_some() => ("ShieldQuestion", "Waits for your approval"),
        StepStatus::Waiting => ("Circle", "Waiting"),
        StepStatus::Running => ("LoaderCircle", "Running"),
        StepStatus::Done => ("CircleCheck", "Done"),
        StepStatus::Failed => ("CircleAlert", "Failed"),
        StepStatus::Cancelled => ("CircleStop", "Cancelled"),
    }
}

/// Delete each run folder; keeps going after a refusal and gives the first.
fn remove_all(data: &std::path::Path, dirs: &[PathBuf]) -> Result<(), RunError> {
    let mut first = Ok(());
    for dir in dirs {
        if let Err(error) = control::remove(data, dir)
            && first.is_ok()
        {
            first = Err(error);
        }
    }
    first
}

impl App {
    /// Takes finished reads and answers, and starts the next read.
    pub(super) fn poll_flow_runs(&mut self, ctx: &egui::Context) {
        if let Some(receiver) = &self.flows.reading
            && let Ok(runs) = receiver.try_recv()
        {
            self.flows.runs = runs;
            self.flows.reading = None;
        }
        let mut failed = None;
        self.flows
            .acting
            .retain(|receiver| match receiver.try_recv() {
                Ok(Ok(())) | Err(TryRecvError::Disconnected) => false,
                Ok(Err(error)) => {
                    failed = Some(error);
                    false
                }
                Err(TryRecvError::Empty) => true,
            });
        if let Some(error) = failed {
            self.error.set(format!("Flow: {error}"));
        }
        let due = self
            .flows
            .read_at
            .is_none_or(|at| at.elapsed() >= READ_EVERY);
        if self.flows.reading.is_none() && due {
            let data = self.paths.data.clone();
            let (sender, receiver) = channel();
            let spawned = std::thread::Builder::new()
                .name("flow-runs".into())
                .spawn(move || {
                    let runs = control::recent_runs(&data, READ_RUNS)
                        .into_iter()
                        .map(|(dir, state)| Run { dir, state })
                        .collect();
                    let _ = sender.send(runs);
                });
            if spawned.is_ok() {
                self.flows.reading = Some(receiver);
            }
            self.flows.read_at = Some(Instant::now());
        }
        ctx.request_repaint_after(READ_EVERY);
    }

    /// Writes one answer off the GUI thread and reads the runs again soon.
    pub(super) fn flow_act(
        &mut self,
        work: impl FnOnce() -> Result<(), RunError> + Send + 'static,
    ) {
        let (sender, receiver) = channel();
        let spawned = std::thread::Builder::new()
            .name("flow-answer".into())
            .spawn(move || {
                let _ = sender.send(work().map_err(|error| error.to_string()));
            });
        if spawned.is_ok() {
            self.flows.acting.push(receiver);
            self.flows.read_at = None;
        } else {
            self.error.set("Flow: could not write the answer");
        }
    }

    /// The ended runs of the selected project, all of them.
    fn ended_runs(&self) -> Vec<PathBuf> {
        self.flows
            .runs
            .iter()
            .filter(|run| run.state.ended)
            .filter(|run| self.selected.as_deref() == Some(run.state.project.as_str()))
            .map(|run| run.dir.clone())
            .collect()
    }

    pub(crate) fn flows_panel(&mut self, ui: &mut egui::Ui) {
        self.poll_flow_runs(ui.ctx());
        let mut act = None;
        let ended = self.ended_runs();
        flows_header(ui, ended.len(), &mut act);
        if self.flows.confirm_clear {
            clear_confirmation(ui, ended, &mut act);
        }
        ui.add_space(4.0);
        let runs = std::mem::take(&mut self.flows.runs);
        let shown: Vec<&Run> = runs
            .iter()
            .filter(|run| self.selected.as_deref() == Some(run.state.project.as_str()))
            .take(SHOWN_RUNS)
            .collect();
        if shown.is_empty() {
            ui.weak("No flow runs in this project. Use Run flow… to start one.");
        } else {
            appearance::sidebar_scroll("flows-tool").show(ui, |ui| {
                for (index, run) in shown.into_iter().enumerate() {
                    self.flow_run_rows(ui, (index, run), &mut act);
                    ui.add_space(4.0);
                }
            });
        }
        // Taken only to draw without a clone; nothing changes them meanwhile.
        self.flows.runs = runs;
        if let Some(act) = act {
            self.apply_flow_act(act);
        }
    }

    pub(super) fn apply_flow_act(&mut self, act: Act) {
        match act {
            Act::RunFlow => self.open_flow_picker(),
            Act::Go(session) => self.go_session(&session),
            Act::Gate(dir, step, approve) => {
                self.flow_act(move || control::answer_gate(&dir, &step, approve));
            }
            Act::Answer(dir, step) => {
                let text = self
                    .flows
                    .drafts
                    .remove(&(dir.clone(), step.clone()))
                    .unwrap_or_default();
                self.flow_act(move || control::answer_question(&dir, &step, &text));
            }
            Act::AskCancel(dir) => self.flows.confirm_cancel = Some(dir),
            Act::KeepRun => self.flows.confirm_cancel = None,
            Act::Cancel(dir) => {
                self.flows.confirm_cancel = None;
                self.flow_act(move || control::cancel(&dir));
            }
            Act::Toggle(dir, open) => {
                self.flows.expanded.insert(dir, open);
            }
            Act::Remove(dir) => self.remove_flow_runs(vec![dir]),
            Act::AskClear => self.flows.confirm_clear = true,
            Act::KeepRuns => self.flows.confirm_clear = false,
            Act::Clear(dirs) => {
                self.flows.confirm_clear = false;
                self.remove_flow_runs(dirs);
            }
        }
    }

    /// Delete ended run folders off the GUI thread and forget their GUI
    /// state. `control::remove` refuses a run that did not end.
    fn remove_flow_runs(&mut self, dirs: Vec<PathBuf>) {
        for dir in &dirs {
            self.flows.expanded.remove(dir);
            self.flows.drafts.retain(|(run, _), _| run != dir);
        }
        let data = self.paths.data.clone();
        self.flow_act(move || remove_all(&data, &dirs));
    }
}

/// The title, Run flow… and the menu with Clear ended runs.
fn flows_header(ui: &mut egui::Ui, ended: usize, act: &mut Option<Act>) {
    ui.horizontal(|ui| {
        ui.strong("Flows");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let more = appearance::icon_menu_button(ui, "EllipsisVertical", |ui| {
                if appearance::menu_danger_action(ui, ended > 0, "Clear ended runs…", "Trash")
                    .on_hover_text("Remove every ended run of this project from the list")
                    .clicked()
                {
                    *act = Some(Act::AskClear);
                }
            });
            more.response.on_hover_text("More flow actions");
            if appearance::sidebar_action(ui, "Play", "Run flow…").clicked() {
                *act = Some(Act::RunFlow);
            }
        });
    });
}

/// The confirmation of Clear ended runs.
fn clear_confirmation(ui: &mut egui::Ui, ended: Vec<PathBuf>, act: &mut Option<Act>) {
    if ended.is_empty() {
        *act = Some(Act::KeepRuns);
        return;
    }
    let count = ended.len();
    let runs = if count == 1 { "run" } else { "runs" };
    ui.label(format!(
        "Remove {count} ended {runs} from the list? Their terminals stay open."
    ));
    ui.horizontal(|ui| {
        if appearance::danger_button(ui, "Clear").clicked() {
            *act = Some(Act::Clear(ended));
            return;
        }
        if ui.button("Keep").clicked() {
            *act = Some(Act::KeepRuns);
        }
    });
}
