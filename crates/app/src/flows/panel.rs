//! The Flows tool window: the runs of the selected project, one row per
//! step. It reads the run state files on a worker thread once per second
//! while it shows. Its buttons write the user's answers through
//! `terminator_core::flow::control` on a worker thread; they never start a
//! step and never type into a terminal.
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
const ROW_HEIGHT: f32 = 26.0;

/// One click in the panel.
pub(super) enum Act {
    RunFlow,
    Go(String),
    Gate(PathBuf, String, bool),
    Answer(PathBuf, String),
    AskCancel(PathBuf),
    KeepRun,
    Cancel(PathBuf),
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
                    let runs = control::runs(&data)
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

    pub(crate) fn flows_panel(&mut self, ui: &mut egui::Ui) {
        self.poll_flow_runs(ui.ctx());
        let mut act = None;
        ui.horizontal(|ui| {
            ui.strong("Flows");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "Play", "Run flow…").clicked() {
                    act = Some(Act::RunFlow);
                }
            });
        });
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
                for run in shown {
                    self.flow_run_rows(ui, run, &mut act);
                    ui.add_space(8.0);
                }
            });
        }
        // Taken only to draw without a clone; nothing changes them meanwhile.
        self.flows.runs = runs;
        if let Some(act) = act {
            self.apply_flow_act(act);
        }
    }

    fn flow_run_rows(&mut self, ui: &mut egui::Ui, run: &Run, act: &mut Option<Act>) {
        let name = run
            .dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.strong(&run.state.flow).on_hover_text(name);
            ui.weak(run_status(&run.state));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !run.state.ended
                    && appearance::sidebar_action(ui, "CircleStop", "Cancel run…").clicked()
                {
                    *act = Some(Act::AskCancel(run.dir.clone()));
                }
            });
        });
        if self.flows.confirm_cancel.as_ref() == Some(&run.dir) {
            ui.label("Cancel this run? No new step starts. Running terminals continue.");
            ui.horizontal(|ui| {
                if appearance::danger_button(ui, "Cancel run").clicked() {
                    *act = Some(Act::Cancel(run.dir.clone()));
                }
                if ui.button("Keep").clicked() {
                    *act = Some(Act::KeepRun);
                }
            });
        }
        for id in &run.state.order {
            if let Some(step) = run.state.steps.get(id) {
                self.flow_step_rows(ui, (run, id, step), act);
            }
        }
    }

    fn flow_step_rows(
        &mut self,
        ui: &mut egui::Ui,
        (run, id, step): (&Run, &String, &StepRun),
        act: &mut Option<Act>,
    ) {
        let (icon, tip) = step_icon(step);
        // A step that waits shows what it waits for; the others their state.
        let state = match step.status {
            StepStatus::Waiting if !step.needs.is_empty() => {
                format!("after {}", step.needs.join(", "))
            }
            _ => tip.to_lowercase(),
        };
        let hover = if step.session.is_some() {
            format!("{tip}. Click to show the terminal of this step.")
        } else {
            tip.to_owned()
        };
        let row = appearance::row(appearance::RowInput {
            ui,
            label: id,
            icon,
            selected: false,
            height: ROW_HEIGHT,
            trailing: &state,
            tint: appearance::color(&self.theme.text),
        })
        .on_hover_text(hover);
        if row.clicked()
            && let Some(session) = &step.session
        {
            *act = Some(Act::Go(session.clone()));
        }
        if step.status == StepStatus::Running
            && let Some(message) = &step.gate
        {
            ui.label(message);
            ui.horizontal(|ui| {
                if ui.button("Approve").clicked() {
                    *act = Some(Act::Gate(run.dir.clone(), id.clone(), true));
                }
                if ui.button("Reject").clicked() {
                    *act = Some(Act::Gate(run.dir.clone(), id.clone(), false));
                }
            });
        }
        if let Some(question) = &step.question {
            ui.label(question);
            let draft = self
                .flows
                .drafts
                .entry((run.dir.clone(), id.clone()))
                .or_default();
            ui.horizontal(|ui| {
                let sent = ui.button("Send").clicked();
                ui.add(egui::TextEdit::singleline(draft).hint_text("Answer"));
                if sent && !draft.trim().is_empty() {
                    *act = Some(Act::Answer(run.dir.clone(), id.clone()));
                }
            });
        }
        if let Some(note) = &step.note {
            ui.weak(note);
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
        }
    }
}
