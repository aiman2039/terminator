//! The colors of the flow editor, from the app theme. The look of a step's
//! state (icon, tone, spin, words) is `flows::status::step_style`, shared
//! with the Flows tool window; the editor only tints it here.
use crate::appearance;
use crate::flows::status::{StepStyle, Tone};

/// The editor's colors, from the app theme.
#[derive(Clone, Copy)]
pub(crate) struct Colors {
    pub(crate) canvas: egui::Color32,
    pub(crate) node: egui::Color32,
    pub(crate) border: egui::Color32,
    /// Weak text, and the steps that the run did not reach.
    pub(crate) weak: egui::Color32,
    /// A running step, and the label of a step card.
    pub(crate) step: egui::Color32,
    /// A done step: a quieter green than a running one.
    pub(crate) done: egui::Color32,
    /// A gate or a question that waits for the user.
    pub(crate) gate: egui::Color32,
    /// A step that failed, and a check error.
    pub(crate) failed: egui::Color32,
    /// The outline of the selected card.
    pub(crate) accent: egui::Color32,
}

impl Colors {
    pub(crate) fn of(theme: &terminator_core::appearance::AppearanceConfig) -> Self {
        Self {
            canvas: appearance::color(&theme.window),
            node: appearance::color(&theme.surface),
            border: appearance::color(&theme.border),
            weak: Tone::Dim.color(theme),
            step: Tone::Running.color(theme),
            done: Tone::Done.color(theme),
            gate: Tone::Waits.color(theme),
            failed: Tone::Failed.color(theme),
            accent: appearance::color(&theme.accent),
        }
    }

    /// The color of a shared tone in this palette.
    pub(crate) fn tone(self, tone: Tone) -> egui::Color32 {
        match tone {
            Tone::Running => self.step,
            Tone::Done => self.done,
            Tone::Waits => self.gate,
            Tone::Failed => self.failed,
            Tone::Dim => self.weak,
        }
    }

    /// The color of a step state.
    pub(crate) fn of_style(self, style: StepStyle) -> egui::Color32 {
        self.tone(style.tone)
    }

    /// The tint of a pin or a wire: its state's color once the run reached
    /// the step, else the quiet border.
    pub(crate) fn wire(self, style: StepStyle) -> egui::Color32 {
        if style.reached {
            self.of_style(style)
        } else {
            self.border
        }
    }
}
