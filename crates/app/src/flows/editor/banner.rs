//! The run banner of the flow editor: one line below the toolbar that says
//! where the newest run of the flow is, so a click on Run has a clear
//! result.
use super::viewer::Colors;
use crate::flows::Run;
use crate::flows::panel::{run_status, step_icon};
use crate::*;
use std::path::{Path, PathBuf};
use terminator_core::flow::StepStatus;

/// What the run does now: the step that runs and what it waits for, or
/// the step that stopped the run.
fn now_text(run: &Run) -> Option<String> {
    let steps = || {
        run.state
            .order
            .iter()
            .filter_map(|id| Some((id, run.state.steps.get(id)?)))
    };
    if let Some((id, step)) = steps().find(|(_, step)| step.status == StepStatus::Running) {
        let state = step.waits_for.as_deref().unwrap_or(step_icon(step).1);
        return Some(format!("Now: {id}. {state}"));
    }
    let (id, step) = steps().find(|(_, step)| step.status == StepStatus::Failed)?;
    let note = step.note.as_deref().unwrap_or("No reason given.");
    Some(format!("Failed: {id}. {note}"))
}

/// A Run click whose new run does not show yet.
pub(crate) struct Starting {
    path: PathBuf,
    /// The clock at the click, in seconds: a run folder's name ends with
    /// the second in which its runner started.
    at: u64,
    since: Instant,
}

/// The longest time that the banner says "Starting".
const STARTING_LIMIT: Duration = Duration::from_secs(30);

impl Starting {
    pub(super) fn now(path: &Path) -> Self {
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_secs());
        Self {
            path: path.into(),
            at,
            since: Instant::now(),
        }
    }

    /// True for a run that started at or after the click.
    fn is_new(&self, run: &Run) -> bool {
        run.dir
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.rsplit('-').next())
            .and_then(|second| second.parse::<u64>().ok())
            .is_some_and(|second| second.saturating_add(1) >= self.at)
    }
}

/// True while the editor of `path` waits for the run of its last Run
/// click: `run` is then an older run. Forgets the click when its run
/// shows or after a time limit.
pub(super) fn still_starting(
    starting: &mut Option<Starting>,
    path: &Path,
    run: Option<&Run>,
) -> bool {
    let Some(click) = starting.as_ref().filter(|click| click.path == path) else {
        return false;
    };
    let shows = run.is_some_and(|run| click.is_new(run));
    if shows || click.since.elapsed() > STARTING_LIMIT {
        *starting = None;
        return false;
    }
    true
}

/// The banner between a Run click and the first state of its run.
pub(super) fn starting_banner(ui: &mut egui::Ui, colors: Colors) {
    egui::Frame::new()
        .fill(colors.node)
        .stroke(egui::Stroke::new(1.0, colors.step))
        .corner_radius(appearance::RADIUS_MEDIUM)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .outer_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spinner();
                ui.label(
                    egui::RichText::new("Starting the run…")
                        .strong()
                        .color(colors.step),
                );
                ui.weak("The runner starts in its own terminal tab.");
            });
        });
    ui.add_space(2.0);
}

/// The banner of `run`. True when the user closed it (an ended run only).
pub(super) fn run_banner(ui: &mut egui::Ui, run: &Run, colors: Colors) -> bool {
    let mut closed = false;
    let status = run_status(run);
    let total = run.state.steps.len();
    let done = run
        .state
        .steps
        .values()
        .filter(|step| step.status == StepStatus::Done)
        .count();
    let color = match status {
        "running" | "done" => colors.step,
        _ => colors.failed,
    };
    egui::Frame::new()
        .fill(colors.node)
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(appearance::RADIUS_MEDIUM)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .outer_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("Run {status}"))
                        .strong()
                        .color(color),
                );
                ui.label(format!("{done} of {total} steps done"));
                if let Some(now) = now_text(run) {
                    ui.weak(now);
                }
                // An ended run is old news: its banner can go away. A
                // newer run has its own banner.
                if run.ended() {
                    closed = appearance::sidebar_action(ui, "X", "Hide this banner").clicked();
                }
            });
            let hint = if run.gone {
                Some(
                    "The runner's terminal closed or its process stopped, so nothing \
                     changes this run again. Step terminals stay open. Run starts a new run.",
                )
            } else if !run.state.ended {
                Some(
                    "Each step runs in its own terminal: use Show terminal on its card. \
                     Cancel run is in the Flows tool window.",
                )
            } else {
                None
            };
            if let Some(hint) = hint {
                ui.label(egui::RichText::new(hint).small().weak());
            }
        });
    ui.add_space(2.0);
    closed
}
