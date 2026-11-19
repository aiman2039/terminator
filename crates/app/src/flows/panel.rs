//! The Flows tool window: the hub of the selected project's flows. "Needs
//! you" holds every gate and question that waits, "Runs" the runs with one
//! row per step (`runs_view`), and "Flows" the flow files with Edit and Run
//! (`picker`). It reads the run state files and the flow files on worker
//! threads while it shows. Its buttons write the user's answers through
//! `terminator_core::flow::control` on a worker thread, and Remove and
//! Clear ended runs delete ended run folders there; they never start a
//! step, never type into a terminal and never close a session. Run starts
//! a flow only on its explicit click, after the trust answer.
use super::Run;
use crate::*;
use std::path::PathBuf;
use std::sync::mpsc::{TryRecvError, channel};
use terminator_core::flow::control::{self, RunError};

const READ_EVERY: Duration = Duration::from_secs(1);
/// How often the tool window reads the flow files again.
const READ_FLOWS_EVERY: Duration = Duration::from_secs(3);
/// Most ended runs shown for one project, newest first. Runs that did
/// not end always show.
const SHOWN_RUNS: usize = 10;
/// Most run folders read each second, newest first, of all projects. Older
/// runs that did not end are read too.
const READ_RUNS: usize = 100;
pub(super) const ROW_HEIGHT: f32 = 26.0;

/// One click in the panel.
pub(super) enum Act {
    RunFlow,
    /// Open the window with New flow.
    NewFlow,
    /// Open this flow file of the tool window's project in the editor.
    Edit(String),
    /// Run this flow file of the tool window's project (explicit click).
    Run(String),
    /// The trust answer for the tool window's project.
    Trust(bool),
    Go(String),
    Gate(PathBuf, String, super::gates::GateChoice),
    Answer(PathBuf, String),
    /// Start this failed step of this run again (explicit click).
    Retry(PathBuf, String),
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

/// "running", "done", "stopped" or "stopped (runner gone)".
pub(super) fn run_status(run: &Run) -> &'static str {
    run.status().label()
}

/// The runs of `project` that the tool window shows: the newest runs, and
/// every run that did not end (a gate or a question there waits for the
/// user, however old the run is).
pub(super) fn shown_runs<'a>(runs: &'a [Run], project: Option<&str>) -> Vec<&'a Run> {
    runs.iter()
        .filter(|run| project == Some(run.state.project.as_str()))
        .enumerate()
        .filter(|(index, run)| *index < SHOWN_RUNS || !run.ended())
        .map(|(_, run)| run)
        .collect()
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
            let ended = std::sync::Arc::clone(&self.flows.ended_runs);
            let (sender, receiver) = channel();
            let spawned = std::thread::Builder::new()
                .name("flow-runs".into())
                .spawn(move || {
                    let mut ended = ended.lock().unwrap_or_else(|poison| poison.into_inner());
                    let runs = control::recent_runs(&data, READ_RUNS, &mut ended)
                        .into_iter()
                        .map(|(dir, state)| Run::read(dir, state))
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

    /// Takes the finished read of the flow files and starts the next one:
    /// at once for another project, else every few seconds.
    fn poll_tool_flows(&mut self) {
        if let Some(receiver) = &self.flows.tool_loading
            && let Ok(loaded) = receiver.try_recv()
        {
            self.flows.tool_flows = Some(loaded);
            self.flows.tool_loading = None;
        }
        let Some(project) = self.selected_project() else {
            return;
        };
        let other = self
            .flows
            .tool_flows
            .as_ref()
            .is_none_or(|loaded| loaded.project != project.id);
        let due = self
            .flows
            .tool_read_at
            .is_none_or(|at| at.elapsed() >= READ_FLOWS_EVERY);
        if self.flows.tool_loading.is_some() || !(other || due) {
            return;
        }
        let (id, root) = (project.id.clone(), project.path.clone());
        let (sender, receiver) = channel();
        let spawned = std::thread::Builder::new()
            .name("flow-files".into())
            .spawn(move || {
                let _ = sender.send(super::load_flows(id, root));
            });
        if spawned.is_ok() {
            self.flows.tool_loading = Some(receiver);
        }
        self.flows.tool_read_at = Some(Instant::now());
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
            .filter(|run| run.ended())
            .filter(|run| self.selected.as_deref() == Some(run.state.project.as_str()))
            .map(|run| run.dir.clone())
            .collect()
    }

    /// The flow files of the selected project, once read.
    pub(super) fn tool_flows(&self) -> Option<&super::LoadedFlows> {
        self.flows
            .tool_flows
            .as_ref()
            .filter(|loaded| self.selected.as_deref() == Some(loaded.project.as_str()))
    }

    pub(crate) fn flows_panel(&mut self, ui: &mut egui::Ui) {
        self.poll_flow_runs(ui.ctx());
        self.poll_tool_flows();
        let mut act = None;
        let ended = self.ended_runs();
        flows_header(ui, ended.len(), &mut act);
        self.flow_confirmations(ui.ctx(), ended, &mut act);
        ui.add_space(4.0);
        let runs = std::mem::take(&mut self.flows.runs);
        let shown = shown_runs(&runs, self.selected.as_deref());
        let no_flows = self
            .tool_flows()
            .is_some_and(|loaded| loaded.files.is_empty());
        if self.selected_project().is_none() {
            ui.weak("Select a local project to see its flows.");
        } else if shown.is_empty() && no_flows {
            empty_state(ui, &mut act);
        } else {
            appearance::sidebar_scroll("flows-tool").show(ui, |ui| {
                self.needs_you_section(ui, &shown, &mut act);
                self.runs_section(ui, &shown, &mut act);
                self.flow_files_section(ui, &mut act);
            });
        }
        // Taken only to draw without a clone; nothing changes them meanwhile.
        self.flows.runs = runs;
        if let Some(act) = act {
            self.apply_flow_act(act);
        }
    }

    /// Cancel run and Clear ended runs ask first, in a confirmation.
    fn flow_confirmations(
        &mut self,
        ctx: &egui::Context,
        ended: Vec<PathBuf>,
        act: &mut Option<Act>,
    ) {
        if let Some(dir) = self.flows.confirm_cancel.clone() {
            let window = self.popups.window(ctx, "Cancel flow run?");
            let dialog = appearance::Confirm::new(
                "Cancel this run?",
                "No new step starts. Running terminals continue.",
            )
            .cancel(appearance::ConfirmButton::new("Keep"))
            .destructive(appearance::ConfirmButton::new("Cancel run"));
            match appearance::confirm(ctx, window, dialog, |_| {}) {
                Some(appearance::ConfirmAnswer::Destructive) => *act = Some(Act::Cancel(dir)),
                Some(_) => *act = Some(Act::KeepRun),
                None => {}
            }
        }
        if !self.flows.confirm_clear {
            return;
        }
        if ended.is_empty() {
            *act = Some(Act::KeepRuns);
            return;
        }
        let window = self.popups.window(ctx, "Clear ended runs?");
        let runs = appearance::count_noun(ended.len(), "ended run");
        let dialog = appearance::Confirm::new(
            format!("Remove {runs} from the list?"),
            "Their terminals stay open.",
        )
        .cancel(appearance::ConfirmButton::new("Keep"))
        .destructive(appearance::ConfirmButton::new("Clear"));
        match appearance::confirm(ctx, window, dialog, |_| {}) {
            Some(appearance::ConfirmAnswer::Destructive) => *act = Some(Act::Clear(ended)),
            Some(_) => *act = Some(Act::KeepRuns),
            None => {}
        }
    }

    pub(super) fn apply_flow_act(&mut self, act: Act) {
        match act {
            Act::RunFlow | Act::NewFlow => self.open_flow_picker(),
            Act::Edit(file) => {
                if let Some(loaded) = self.tool_flows().cloned() {
                    self.open_flow_tab(&loaded.project, &loaded.root, &file);
                }
            }
            Act::Run(file) => {
                if let Some(loaded) = self.tool_flows().cloned() {
                    let inputs = loaded
                        .files
                        .iter()
                        .find(|flow| flow.file == file)
                        .map(|flow| flow.inputs.clone())
                        .unwrap_or_default();
                    let origin = super::inputs_form::Origin::Window;
                    self.request_flow_run(&loaded, &file, (&inputs, origin));
                }
            }
            Act::Trust(trusted) => {
                if let Some(root) = self.tool_flows().map(|loaded| loaded.root.clone()) {
                    self.answer_task_trust(&root, trusted);
                }
            }
            Act::Go(session) => self.go_session(&session),
            Act::Gate(dir, step, choice) => {
                self.flow_act(move || control::answer_gate(&dir, &step, choice.answer()));
            }
            Act::Answer(dir, step) => {
                let text = self
                    .flows
                    .drafts
                    .remove(&(dir.clone(), step.clone()))
                    .unwrap_or_default();
                self.flow_act(move || control::answer_question(&dir, &step, &text));
            }
            Act::Retry(dir, step) => {
                self.flow_act(move || control::retry(&dir, &step));
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

/// The title, Run flow… and the menu with New flow and Clear ended runs.
fn flows_header(ui: &mut egui::Ui, ended: usize, act: &mut Option<Act>) {
    ui.horizontal(|ui| {
        ui.strong("Flows");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let more = appearance::icon_menu_button(ui, "EllipsisVertical", |ui| {
                if appearance::menu_action(ui, true, "New flow…", "Plus").clicked() {
                    *act = Some(Act::NewFlow);
                }
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

/// A section title: small secondary capitals, with a count at the right.
pub(super) fn section_title(ui: &mut egui::Ui, title: &str, count: usize) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let text = |text: String| {
            egui::RichText::new(text)
                .size(appearance::FONT_CAPTION)
                .weak()
        };
        ui.label(text(title.to_uppercase()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(text(count.to_string()));
        });
    });
    ui.add_space(2.0);
}

/// No flow and no run in this project: what a flow is, New flow, and
/// where the files are.
pub(super) fn empty_state(ui: &mut egui::Ui, act: &mut Option<Act>) {
    ui.add_space(8.0);
    egui::Frame::new()
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(appearance::RADIUS_LARGE)
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.strong("No flows in this project");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "A flow runs agent steps in order, with gates where you approve.",
                    )
                    .size(appearance::FONT_SMALL)
                    .weak(),
                );
                ui.add_space(8.0);
                if appearance::primary_button(ui, "New flow")
                    .on_hover_text("Name a new flow, then make it in the flow editor")
                    .clicked()
                {
                    *act = Some(Act::NewFlow);
                }
            });
        });
    ui.add_space(6.0);
    ui.label(
        egui::RichText::new("Flows are stored in .terminator/flows/.")
            .size(appearance::FONT_CAPTION)
            .weak(),
    );
}
