//! One look for the state of a flow step and run: its icon, color, spin
//! and words. The Flows tool window, the editor's cards, pins, wires and
//! run strip use it, so a state reads the same everywhere. Display only: it
//! never changes a run.
use super::Run;
use crate::*;
use terminator_core::appearance::AppearanceConfig;
use terminator_core::flow::{StepRun, StepStatus, control::RunStatus};

/// The color role of a state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    /// Running: green, with a spin.
    Running,
    /// Done: a quieter green, between green and the secondary text color.
    Done,
    /// A gate or a question waits for the user: amber.
    Waits,
    /// Failed: red.
    Failed,
    /// Not reached, or cancelled: the secondary text color.
    Dim,
}

impl Tone {
    pub(crate) fn color(self, theme: &AppearanceConfig) -> egui::Color32 {
        match self {
            Self::Running => appearance::color(&theme.status_running),
            Self::Done => appearance::color(&theme.status_running)
                .lerp_to_gamma(appearance::color(&theme.secondary), DONE_QUIET),
            Self::Waits => appearance::color(&theme.status_waiting),
            Self::Failed => appearance::color(&theme.status_failed),
            Self::Dim => appearance::color(&theme.secondary),
        }
    }
}

/// How far the done green goes toward the secondary text color: a done
/// step is still green, but quieter than a running one.
const DONE_QUIET: f32 = 0.5;

#[cfg(test)]
impl Tone {
    pub(crate) const ALL: [Self; 5] = [
        Self::Running,
        Self::Done,
        Self::Waits,
        Self::Failed,
        Self::Dim,
    ];
}

/// How a step shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StepStyle {
    pub(crate) icon: &'static str,
    pub(crate) tone: Tone,
    /// The loader turns while the step runs.
    pub(crate) spin: bool,
    /// "Done", "Waits for your approval"...
    pub(crate) label: &'static str,
    /// The state in a word or two, for a chip: "done", "waits"...
    pub(crate) chip: &'static str,
    /// The run reached this step: not waiting to start and not cancelled.
    pub(crate) reached: bool,
}

impl StepStyle {
    pub(crate) fn color(self, theme: &AppearanceConfig) -> egui::Color32 {
        self.tone.color(theme)
    }
}

/// The one style of a step's state.
pub(crate) fn step_style(step: &StepRun) -> StepStyle {
    let style = |icon, tone, label, chip| StepStyle {
        icon,
        tone,
        spin: false,
        label,
        chip,
        reached: true,
    };
    match step.status {
        _ if step.question.is_some() => style(
            "MessageCircleQuestion",
            Tone::Waits,
            "Waits for your answer",
            "asks",
        ),
        StepStatus::Running if step.gate.is_some() => style(
            "ShieldQuestion",
            Tone::Waits,
            "Waits for your approval",
            "waits",
        ),
        StepStatus::Waiting => StepStyle {
            reached: false,
            ..style("Circle", Tone::Dim, "Not started", "not started")
        },
        StepStatus::Running => StepStyle {
            spin: true,
            ..style("LoaderCircle", Tone::Running, "Running", "running")
        },
        StepStatus::Done => style("CircleCheck", Tone::Done, "Done", "done"),
        StepStatus::Failed => style("CircleAlert", Tone::Failed, "Failed", "failed"),
        StepStatus::Cancelled => StepStyle {
            reached: false,
            ..style("CircleStop", Tone::Dim, "Cancelled", "cancelled")
        },
    }
}

/// A gate or a question of this step waits for the user.
pub(crate) fn waits_for_user(step: &StepRun) -> bool {
    step_style(step).tone == Tone::Waits
}

/// The run's chip: one word and its tone. A running run with a gate or a
/// question "waits"; a stopped run with a failed step "failed".
pub(crate) fn run_chip(run: &Run) -> (&'static str, Tone) {
    let steps = || run.state.steps.values();
    match run.status() {
        RunStatus::Running if steps().any(waits_for_user) => ("waits", Tone::Waits),
        RunStatus::Running => ("running", Tone::Running),
        RunStatus::Done => ("done", Tone::Done),
        RunStatus::Stopped if steps().any(|step| step.status == StepStatus::Failed) => {
            ("failed", Tone::Failed)
        }
        RunStatus::Stopped => ("stopped", Tone::Dim),
        RunStatus::RunnerGone => ("runner gone", Tone::Dim),
    }
}

/// The tone of each step of a run, in flow order: one progress segment
/// each.
pub(crate) fn segments(run: &Run) -> Vec<Tone> {
    run.state
        .order
        .iter()
        .filter_map(|id| run.state.steps.get(id))
        .map(|step| step_style(step).tone)
        .collect()
}

/// Paint the progress bar of a run: one segment per step, in its state's
/// color. A done step is a quieter green than a running one.
pub(crate) fn progress_bar(ui: &mut egui::Ui, run: &Run, theme: &AppearanceConfig) {
    const HEIGHT: f32 = 4.0;
    const GAP: f32 = 2.0;
    const LEFT: f32 = 26.0;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, HEIGHT), egui::Sense::hover());
    let bar = egui::Rect::from_min_max(
        egui::pos2(rect.left() + LEFT, rect.top()),
        egui::pos2(rect.right() - 6.0, rect.bottom()),
    );
    let tones = segments(run);
    if tones.is_empty() || bar.width() <= 0.0 || !ui.is_rect_visible(bar) {
        return;
    }
    let count = tones.len() as f32;
    let each = (bar.width() - GAP * (count - 1.0)) / count;
    let border = appearance::color(&theme.border);
    for (index, tone) in tones.into_iter().enumerate() {
        let left = bar.left() + index as f32 * (each + GAP);
        let segment =
            egui::Rect::from_min_size(egui::pos2(left, bar.top()), egui::vec2(each, HEIGHT));
        let fill = match tone {
            Tone::Dim => border,
            _ => tone.color(theme),
        };
        ui.painter()
            .rect_filled(segment, appearance::RADIUS_XSMALL, fill);
    }
}

/// A small status chip: `text` in the tone's color, with a light fill and
/// outline of the same color.
pub(crate) fn chip(ui: &mut egui::Ui, text: &str, color: egui::Color32) -> egui::Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .stroke(egui::Stroke::new(1.0, color.gamma_multiply(0.6)))
        .corner_radius(appearance::RADIUS_SMALL)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .size(appearance::FONT_CAPTION)
                    .color(color),
            )
        })
        .response
}

/// A status icon of the toolbar size in `tint`, with its spin and its
/// label as the hover text.
pub(crate) fn status_icon(
    ui: &mut egui::Ui,
    style: StepStyle,
    tint: egui::Color32,
) -> egui::Response {
    let size = egui::Vec2::splat(appearance::TOOLBAR_ICON);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    appearance::paint_status_icon(appearance::PaintStatusIconInput {
        ui,
        rect,
        icon: style.icon,
        tint,
        spin: style.spin,
    });
    if style.spin {
        ui.ctx().request_repaint();
    }
    response.on_hover_text(style.label)
}
