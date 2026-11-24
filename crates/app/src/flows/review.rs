//! Guided review checklist for a finished flow run (#350): ordered steps with
//! GUI-memory check-off toggles and a computed risk badge. Display only.
use super::Run;
use super::panel::Act;
use crate::*;
use terminator_core::flow::{StepRun, StepStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    pub(crate) fn of(step: &StepRun) -> Self {
        let large_diff = step
            .racers
            .iter()
            .any(|r| r.added.unwrap_or(0).saturating_add(r.deleted.unwrap_or(0)) > 200);
        if step.status == StepStatus::Failed || step.exited || large_diff {
            Self::High
        } else if step.attempt() > 1
            || step.round() > 1
            || step.worktree_path.is_some()
            || !step.racers.is_empty()
        {
            Self::Medium
        } else {
            Self::Low
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Low => "low risk",
            Self::Medium => "medium risk",
            Self::High => "high risk",
        }
    }

    fn color(self, theme: &terminator_core::appearance::AppearanceConfig) -> egui::Color32 {
        match self {
            Self::Low => appearance::color(&theme.secondary),
            Self::Medium => appearance::color(&theme.status_waiting),
            Self::High => appearance::color(&theme.status_failed),
        }
    }
}

impl App {
    /// Collapsible ordered review checklist for an ended run.
    pub(super) fn flow_review_checklist(
        &mut self,
        ui: &mut egui::Ui,
        run: &Run,
        act: &mut Option<Act>,
    ) {
        if !run.ended() || run.state.order.is_empty() {
            return;
        }
        let total = run.state.order.len();
        let checked_count = run
            .state
            .order
            .iter()
            .filter(|id| {
                self.flows
                    .review_checked
                    .contains(&(run.dir.clone(), (*id).clone()))
            })
            .count();
        let title = format!("Review checklist ({checked_count}/{total})");
        egui::CollapsingHeader::new(
            egui::RichText::new(title)
                .size(appearance::FONT_CAPTION)
                .weak(),
        )
        .id_salt(("flow-review-checklist", &run.dir))
        .default_open(false)
        .show(ui, |ui| {
            for id in &run.state.order {
                let Some(step) = run.state.steps.get(id) else {
                    continue;
                };
                let key = (run.dir.clone(), id.clone());
                let mut done = self.flows.review_checked.contains(&key);
                let risk = Risk::of(step);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut done, id).changed() {
                        if done {
                            self.flows.review_checked.insert(key);
                        } else {
                            self.flows.review_checked.remove(&key);
                        }
                    }
                    ui.colored_label(
                        risk.color(&self.theme),
                        egui::RichText::new(risk.label()).size(appearance::FONT_CAPTION),
                    );
                    if let Some(path) = &step.worktree_path
                        && appearance::link(
                            ui,
                            egui::RichText::new("Diff").size(appearance::FONT_CAPTION),
                        )
                        .clicked()
                    {
                        *act = Some(Act::OpenDiff(path.clone()));
                    }
                    if let Some(session) = &step.session
                        && appearance::link(
                            ui,
                            egui::RichText::new("Peek").size(appearance::FONT_CAPTION),
                        )
                        .clicked()
                    {
                        *act = Some(Act::Peek(session.clone()));
                    }
                });
            }
        });
    }
}

/// The title, Run flow… and the menu with New flow and Clear ended runs.
pub(super) fn flows_header(ui: &mut egui::Ui, ended: usize, act: &mut Option<Act>) {
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
