//! The run banner of the flow editor: one line below the toolbar that says
//! where the newest run of the flow is, so a click on Run has a clear
//! result.
use super::viewer::Colors;
use crate::flows::Run;
use crate::flows::panel::{run_status, step_icon};
use crate::*;
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

/// The banner of `run`.
pub(super) fn run_banner(ui: &mut egui::Ui, run: &Run, colors: Colors) {
    let status = run_status(&run.state);
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
            });
            if !run.state.ended {
                ui.label(
                    egui::RichText::new(
                        "Each step runs in its own terminal: use Show terminal on its card. \
                         Cancel run is in the Flows tool window.",
                    )
                    .small()
                    .weak(),
                );
            }
        });
    ui.add_space(2.0);
}
