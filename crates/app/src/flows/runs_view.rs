//! The rows of one run in the Flows tool window. Only the newest run and
//! the runs that did not end show their steps; an older ended run is one
//! line that a click opens. The open state is GUI memory only.
use super::Run;
use super::panel::{Act, ROW_HEIGHT, run_status, step_icon};
use crate::*;
use std::sync::Arc;
use terminator_core::flow::{StepRun, StepStatus};

/// Most text rows of a note under a step; the full text is on hover.
const NOTE_ROWS: usize = 2;

/// "stopped · 1 of 4 steps done".
pub(super) fn run_summary(run: &Run) -> String {
    let total = run.state.steps.len();
    let done = run
        .state
        .steps
        .values()
        .filter(|step| step.status == StepStatus::Done)
        .count();
    format!("{} · {done} of {total} steps done", run_status(&run.state))
}

/// A note laid out in at most [`NOTE_ROWS`] rows of `width`.
pub(super) fn note_galley(ui: &egui::Ui, text: &str, width: f32) -> Arc<egui::Galley> {
    let font = egui::TextStyle::Small.resolve(ui.style());
    let mut job =
        egui::text::LayoutJob::simple(text.to_owned(), font, ui.visuals().weak_text_color(), width);
    job.wrap.max_rows = NOTE_ROWS;
    ui.painter().layout_job(job)
}

/// A short note: clipped, with the full text on hover when clipped.
fn clipped_note(ui: &mut egui::Ui, text: &str) {
    let galley = note_galley(ui, text, ui.available_width().max(0.0));
    let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    let elided = galley.elided;
    let color = ui.visuals().weak_text_color();
    ui.painter().galley(rect.min, galley, color);
    if elided {
        response.on_hover_text(text);
    }
}

impl App {
    /// The newest run (`index` 0) and a run that did not end are open
    /// unless the user closed them; other runs are closed unless opened.
    pub(super) fn run_expanded(&self, index: usize, run: &Run) -> bool {
        self.flows
            .expanded
            .get(&run.dir)
            .copied()
            .unwrap_or(index == 0 || !run.state.ended)
    }

    pub(super) fn flow_run_rows(
        &mut self,
        ui: &mut egui::Ui,
        (index, run): (usize, &Run),
        act: &mut Option<Act>,
    ) {
        let open = self.run_expanded(index, run);
        self.flow_run_header(ui, (run, open), act);
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
        if !open {
            return;
        }
        for id in &run.state.order {
            if let Some(step) = run.state.steps.get(id) {
                self.flow_step_rows(ui, (run, id, step), act);
            }
        }
    }

    /// One line: open/close, flow name, state and summary, and Remove for
    /// an ended run or Cancel for a running one.
    fn flow_run_header(
        &mut self,
        ui: &mut egui::Ui,
        (run, open): (&Run, bool),
        act: &mut Option<Act>,
    ) {
        let name = run
            .dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let summary = run_summary(run);
        let tint = appearance::color(&self.theme.text);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if run.state.ended {
                    if appearance::sidebar_action(
                        ui,
                        "Trash",
                        "Remove this run from the list. Its terminals stay open.",
                    )
                    .clicked()
                    {
                        *act = Some(Act::Remove(run.dir.clone()));
                    }
                } else if appearance::sidebar_action(ui, "CircleStop", "Cancel run…").clicked() {
                    *act = Some(Act::AskCancel(run.dir.clone()));
                }
                let hover = if open {
                    format!("Run {name}. Click to hide its steps.")
                } else {
                    format!("Run {name}. Click to show its steps.")
                };
                let row = appearance::row(appearance::RowInput {
                    ui,
                    label: &run.state.flow,
                    icon: if open { "ChevronDown" } else { "ChevronRight" },
                    selected: false,
                    height: ROW_HEIGHT,
                    trailing: &summary,
                    tint,
                })
                .on_hover_text(hover);
                if row.clicked() {
                    *act = Some(Act::Toggle(run.dir.clone(), !open));
                }
            });
        });
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
                // Send sits after the field; Enter in the field also sends.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let clicked = ui.button("Send").clicked();
                    let field = ui.add(
                        egui::TextEdit::singleline(draft)
                            .hint_text("Answer")
                            .desired_width(ui.available_width()),
                    );
                    let entered =
                        field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (clicked || entered) && !draft.trim().is_empty() {
                        *act = Some(Act::Answer(run.dir.clone(), id.clone()));
                    }
                });
            });
        }
        // What a running step waits for, and the note of a failed one.
        if matches!(step.status, StepStatus::Running | StepStatus::Failed) {
            for text in [&step.waits_for, &step.note].into_iter().flatten() {
                clipped_note(ui, text);
            }
        }
    }
}
