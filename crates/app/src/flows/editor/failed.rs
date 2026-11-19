//! A flow file that is not a correct flow: a centered card with the error,
//! Open as text (at the line of the failing step when it is known) and
//! Read again.
use super::Slot;
use super::inspector::error_place;
use super::style::Colors;
use crate::app::OpenFileModeInput;
use crate::*;
use std::path::Path;
use terminator_core::flow::files::LoadError;

/// Why a flow file does not open in the editor, and the line to repair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) error: String,
    /// The 1-based line of the failing step or of the syntax error.
    pub(crate) line: Option<u32>,
}

impl Failure {
    pub(super) fn plain(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            line: None,
        }
    }
}

/// The 1-based line of `text` that `error` concerns: the line of a syntax
/// error, or the `id = "…"` line of the step that the error names.
pub(crate) fn failing_line(text: &str, error: &LoadError) -> Option<u32> {
    let LoadError::Flow(error) = error else {
        return None;
    };
    let message = error.to_string();
    if let Some(rest) = message.split("at line ").nth(1) {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        return digits.parse().ok();
    }
    let step = error_place(error).0?;
    let wanted = [format!("\"{step}\""), format!("'{step}'")];
    let index = text.lines().position(|line| {
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        key.trim() == "id" && wanted.iter().any(|quoted| value.trim() == quoted)
    })?;
    u32::try_from(index.saturating_add(1)).ok()
}

/// Reads the failing line off the GUI thread (the loader's worker).
pub(super) fn failure_of(path: &Path, error: &LoadError) -> Failure {
    let line = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| failing_line(&text, error));
    Failure {
        error: error.to_string(),
        line,
    }
}

/// The width of the card.
const CARD_WIDTH: f32 = 520.0;

impl App {
    /// The card of a file that is not a correct flow. `None` after Read
    /// again: the next frame reads the file again.
    pub(super) fn flow_failed_view(
        &mut self,
        ui: &mut egui::Ui,
        path: &Path,
        failure: Failure,
    ) -> Option<Slot> {
        let colors = Colors::of(&self.theme);
        let outer = ui.available_rect_before_wrap();
        ui.painter()
            .rect_filled(outer, egui::CornerRadius::ZERO, colors.canvas);
        let width = CARD_WIDTH.min(outer.width() - 32.0).max(160.0);
        let top = outer.top() + ((outer.height() - 280.0) / 2.0).max(16.0);
        let rect = egui::Rect::from_min_max(
            egui::pos2(outer.center().x - width / 2.0, top),
            egui::pos2(outer.center().x + width / 2.0, outer.bottom()),
        );
        let mut card = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        let style = appearance::CenterCardStyle::new(&self.theme);
        let (mut open, mut again) = (false, false);
        style.frame().show(&mut card, |ui| {
            ui.set_width(ui.available_width());
            let title = egui::RichText::new("This flow cannot open in the editor")
                .size(appearance::FONT_HEADING)
                .strong();
            ui.label(title);
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            match failure.line {
                Some(line) => ui.weak(format!("{name} · line {line}")),
                None => ui.weak(format!("{name} · 1 problem")),
            };
            ui.add_space(8.0);
            egui::Frame::new()
                .fill(colors.failed.gamma_multiply(0.12))
                .stroke(egui::Stroke::new(1.0, colors.failed.gamma_multiply(0.6)))
                .corner_radius(appearance::RADIUS_MEDIUM)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(&failure.error).wrap());
                });
            ui.add_space(12.0);
            appearance::footer_row(ui, |ui| {
                open = appearance::primary_button(ui, "Open as text").clicked();
                again = ui.button("Read again").clicked();
            });
        });
        ui.allocate_rect(outer, egui::Sense::hover());
        if open {
            self.open_file_mode(OpenFileModeInput {
                path: path.into(),
                line: failure.line,
                split: None,
                external: false,
                text: true,
            });
        }
        (!again).then_some(Slot::Failed(failure))
    }
}
