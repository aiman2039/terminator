//! The body of a card: a read-only summary of the step (the command as a
//! chip, two lines of the prompt, the worktree and the folder), and in a
//! run a footer with the one action that applies now: Approve and Reject
//! for a gate that waits, the answer of a question, or the way to the
//! step's terminal. The inspector edits the fields.
use super::model::Node;
use super::style::Colors;
use super::viewer::{CARD_WIDTH, Viewer, gate_waits};
use crate::appearance;
use std::sync::Arc;
use terminator_core::flow::{StepRun, StepStatus};

/// Most text rows of a prompt, a question or a note on a card.
const ROWS: usize = 2;

/// `text` in at most `rows` rows of the card width; the full text is on
/// hover when it does not fit.
pub(super) fn clipped(ui: &mut egui::Ui, text: &str, rows: usize, color: egui::Color32) {
    let galley = galley(ui, text, rows, color);
    let (rect, response) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
    let elided = galley.elided;
    ui.painter().galley(rect.min, galley, color);
    if elided {
        response.on_hover_text(text);
    }
}

fn galley(ui: &egui::Ui, text: &str, rows: usize, color: egui::Color32) -> Arc<egui::Galley> {
    let font = egui::FontId::proportional(appearance::FONT_SMALL);
    let flat = text.trim().replace('\n', " ");
    let mut job = egui::text::LayoutJob::simple(flat, font, color, CARD_WIDTH);
    job.wrap.max_rows = rows;
    ui.painter().layout_job(job)
}

/// A chip with an icon and a caption, for a worktree or a folder.
fn icon_chip(ui: &mut egui::Ui, (icon, text, tip): (&str, &str, &str), colors: Colors) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, colors.border))
        .corner_radius(appearance::RADIUS_SMALL)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let size = egui::Vec2::splat(appearance::FONT_CAPTION);
            let image = egui::Image::new(crate::icons::source(icon))
                .tint(colors.weak)
                .fit_to_exact_size(size);
            ui.add(image);
            let label = egui::RichText::new(text)
                .size(appearance::FONT_CAPTION)
                .color(colors.weak);
            ui.add(egui::Label::new(label).truncate());
        })
        .response
        .on_hover_text(tip);
}

/// The command as a monospace chip.
fn command_chip(ui: &mut egui::Ui, command: &str, colors: Colors) {
    egui::Frame::new()
        .fill(colors.canvas)
        .stroke(egui::Stroke::new(1.0, colors.border))
        .corner_radius(appearance::RADIUS_SMALL)
        .inner_margin(egui::Margin::symmetric(6, 2))
        .show(ui, |ui| {
            let text = if command.trim().is_empty() {
                egui::RichText::new("no command").weak()
            } else {
                egui::RichText::new(command.trim()).monospace()
            };
            ui.add(egui::Label::new(text).truncate());
        });
}

/// The read-only summary of a card.
pub(super) fn summary(ui: &mut egui::Ui, value: &Node, colors: Colors) {
    let text = ui.visuals().text_color();
    match value {
        Node::Step {
            command,
            prompt,
            cwd,
            worktree,
            ..
        } => {
            command_chip(ui, command, colors);
            if prompt.trim().is_empty() {
                clipped(ui, "No prompt", 1, colors.weak);
            } else {
                clipped(ui, prompt, ROWS, text);
            }
            let chips = [
                ("GitBranch", worktree.trim(), "The worktree of this step"),
                ("Folder", cwd.trim(), "The folder of this step"),
            ];
            if chips.iter().any(|(_, text, _)| !text.is_empty()) {
                ui.horizontal_wrapped(|ui| {
                    ui.set_max_width(CARD_WIDTH);
                    for chip in chips.into_iter().filter(|(_, text, _)| !text.is_empty()) {
                        icon_chip(ui, chip, colors);
                    }
                });
            }
        }
        Node::Gate { message, .. } => {
            if message.trim().is_empty() {
                clipped(ui, "No question", 1, colors.weak);
            } else {
                clipped(ui, message, ROWS, text);
            }
        }
    }
}

impl Viewer<'_> {
    /// The run footer of a card: the one action that applies now.
    pub(super) fn footer(&mut self, ui: &mut egui::Ui, id: &str, step: &StepRun) {
        ui.add_space(2.0);
        ui.separator();
        if gate_waits(step) {
            appearance::footer_row(ui, |ui| {
                if appearance::primary_button(ui, "Approve").clicked() {
                    self.out.answer = Some((id.to_owned(), true));
                }
                if appearance::danger_button(ui, "Reject").clicked() {
                    self.out.answer = Some((id.to_owned(), false));
                }
            });
            return;
        }
        if let Some(question) = &step.question {
            clipped(ui, question, ROWS, ui.visuals().text_color());
            self.answer_row(ui, id);
            return;
        }
        let (text, color) = match step.status {
            StepStatus::Running => (&step.waits_for, self.colors.weak),
            StepStatus::Failed => (&step.note, self.colors.failed),
            _ => (&step.note, self.colors.weak),
        };
        if let Some(text) = text {
            clipped(ui, text, ROWS, color);
        }
        if let Some(session) = &step.session {
            let label = egui::RichText::new("Open terminal").size(appearance::FONT_CAPTION);
            if appearance::link(ui, label)
                .on_hover_text("Go to the terminal of this step")
                .clicked()
            {
                self.out.go = Some(session.clone());
            }
            // Room for the underline and the descenders: the card body
            // clips at its last row.
            ui.add_space(4.0);
        }
    }

    /// The answer field of a question and Send. Enter in the field also
    /// sends; the answer goes to the runner's answer file, never to a
    /// terminal.
    fn answer_row(&mut self, ui: &mut egui::Ui, id: &str) {
        let draft = self.drafts.entry(id.to_owned()).or_default();
        let mut send = false;
        appearance::footer_row(ui, |ui| {
            let filled = !draft.trim().is_empty();
            send = ui.add_enabled(filled, egui::Button::new("Send")).clicked();
            let field = ui.add(
                appearance::singleline(draft)
                    .hint_text("Answer…")
                    .desired_width(ui.available_width()),
            );
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            send = (send || entered) && filled;
        });
        if send {
            self.out.send = Some(id.to_owned());
        }
    }
}
