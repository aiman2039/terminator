//! The answer buttons of a gate that waits, the same in the Flows tool
//! window, on an editor card and in the run strip: Approve and Reject, or
//! one Approve button per option and Reject. Each is an explicit click; it
//! only writes the gate's answer file. Also the words for a gate's choice,
//! a branch condition and a loop round.
use crate::appearance;
use std::path::PathBuf;
use terminator_core::flow::{RacerRun, StepRun, StepStatus, control::GateAnswer};

/// The user's answer to a gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GateChoice {
    /// Approve a gate with no options.
    Approve,
    /// Approve a gate with options, with this one.
    Option(String),
    /// Approve a pick gate with this 1-based racer index.
    Pick(u32),
    Reject,
}

impl GateChoice {
    pub(crate) fn answer(&self) -> GateAnswer<'_> {
        match self {
            Self::Approve => GateAnswer::Approve,
            Self::Option(option) => GateAnswer::Choose(option),
            Self::Pick(index) => GateAnswer::Pick(*index),
            Self::Reject => GateAnswer::Reject,
        }
    }
}

/// One option button, with its hover text.
fn option_button(ui: &mut egui::Ui, option: &str) -> bool {
    appearance::primary_button(ui, option)
        .on_hover_text(format!("Approve with \"{option}\": the steps for it run"))
        .clicked()
}

/// The buttons of a gate in a card or a list: a right-aligned row with
/// Approve and Reject, or for a gate with options a row that wraps, with
/// one button per option and then Reject.
pub(crate) fn gate_buttons(ui: &mut egui::Ui, options: &[String]) -> Option<GateChoice> {
    let mut choice = None;
    if options.is_empty() {
        appearance::footer_row(ui, |ui| {
            if appearance::primary_button(ui, "Approve").clicked() {
                choice = Some(GateChoice::Approve);
            }
            if appearance::danger_button(ui, "Reject").clicked() {
                choice = Some(GateChoice::Reject);
            }
        });
        return choice;
    }
    ui.horizontal_wrapped(|ui| {
        for option in options {
            if option_button(ui, option) {
                choice = Some(GateChoice::Option(option.clone()));
            }
        }
        if appearance::danger_button(ui, "Reject").clicked() {
            choice = Some(GateChoice::Reject);
        }
    });
    choice
}

/// The same buttons in a right-to-left row (the run strip): from the left,
/// Reject, then Approve or the options in their order.
pub(crate) fn gate_buttons_right_to_left(
    ui: &mut egui::Ui,
    options: &[String],
) -> Option<GateChoice> {
    let mut choice = None;
    if options.is_empty() {
        if appearance::primary_button(ui, "Approve").clicked() {
            choice = Some(GateChoice::Approve);
        }
    } else {
        for option in options.iter().rev() {
            if option_button(ui, option) {
                choice = Some(GateChoice::Option(option.clone()));
            }
        }
    }
    if appearance::danger_button(ui, "Reject").clicked() {
        choice = Some(GateChoice::Reject);
    }
    choice
}

/// Buttons and diffstat rows for a `pick = "STEP"` gate: shows each racer's
/// note, diffstat (`+A −D`), an explicit `Open diff` link, and `Pick #N`.
pub(crate) fn pick_buttons(
    ui: &mut egui::Ui,
    racers: &[RacerRun],
    open_diff: &mut Option<PathBuf>,
) -> Option<GateChoice> {
    let mut choice = None;
    for racer in racers {
        ui.horizontal_wrapped(|ui| {
            let label = match &racer.agent {
                Some(agent) => format!("#{} ({agent})", racer.index),
                None => format!("#{}", racer.index),
            };
            if appearance::primary_button(ui, &format!("Pick {label}"))
                .on_hover_text(format!("Choose worktree {}", racer.worktree))
                .clicked()
            {
                choice = Some(GateChoice::Pick(racer.index));
            }
            if let (Some(a), Some(d)) = (racer.added, racer.deleted) {
                ui.weak(format!("+{a} −{d}"));
            }
            if let Some(path) = &racer.worktree_path
                && appearance::link(
                    ui,
                    egui::RichText::new("Open diff").size(appearance::FONT_CAPTION),
                )
                .clicked()
            {
                *open_diff = Some(path.clone());
            }
        });
        if let Some(note) = &racer.note {
            ui.label(
                egui::RichText::new(note)
                    .size(appearance::FONT_CAPTION)
                    .weak(),
            );
        }
    }
    if appearance::danger_button(ui, "Reject").clicked() {
        choice = Some(GateChoice::Reject);
    }
    choice
}

/// Pick-gate buttons in a right-to-left row (the run strip).
pub(crate) fn pick_buttons_right_to_left(
    ui: &mut egui::Ui,
    racer_count: usize,
) -> Option<GateChoice> {
    let mut choice = None;
    for idx in (1..=racer_count).rev() {
        let index = u32::try_from(idx).unwrap_or(1);
        if appearance::primary_button(ui, &format!("Pick #{index}")).clicked() {
            choice = Some(GateChoice::Pick(index));
        }
    }
    if appearance::danger_button(ui, "Reject").clicked() {
        choice = Some(GateChoice::Reject);
    }
    choice
}

/// The branch and loop words of a step: "chose ship" for a gate that was
/// answered with an option, "when review.ship" for a step with a condition
/// that has not run, and "round 2 of 3" for a step in a loop.
pub(crate) fn branch_labels(step: &StepRun) -> Vec<String> {
    let mut labels = Vec::new();
    if let Some(choice) = &step.choice {
        labels.push(format!("chose {choice}"));
    }
    if let Some(when) = &step.when
        && matches!(step.status, StepStatus::Waiting | StepStatus::Skipped)
    {
        labels.push(format!("when {when}"));
    }
    labels.extend(step.round_label());
    labels
}
