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

/// A chip with an icon (none when empty) and a caption, for a worktree, a
/// folder, a gate option, a condition or a loop.
fn icon_chip(ui: &mut egui::Ui, (icon, text, tip): (&str, &str, &str), colors: Colors) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, colors.border))
        .corner_radius(appearance::RADIUS_SMALL)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            if !icon.is_empty() {
                let size = egui::Vec2::splat(appearance::FONT_CAPTION);
                let image = egui::Image::new(crate::icons::source(icon))
                    .tint(colors.weak)
                    .fit_to_exact_size(size);
                ui.add(image);
            }
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
            race_count,
            race_agents,
            ..
        } => {
            command_chip(ui, command, colors);
            if prompt.trim().is_empty() {
                clipped(ui, "No prompt", 1, colors.weak);
            } else {
                clipped(ui, prompt, ROWS, text);
            }
            let race_label = if !race_count.trim().is_empty() {
                format!("{} racers", race_count.trim())
            } else if !race_agents.trim().is_empty() {
                format!("race: {}", race_agents.trim())
            } else {
                String::new()
            };
            let chips = [
                ("GitBranch", worktree.trim(), "The worktree of this step"),
                ("Folder", cwd.trim(), "The folder of this step"),
                ("Split", race_label.as_str(), "Best-of-N race step"),
            ];
            chip_row(ui, &chips, colors);
        }
        Node::Gate { message, pick, .. } => {
            if message.trim().is_empty() {
                clipped(ui, "No question", 1, colors.weak);
            } else {
                clipped(ui, message, ROWS, text);
            }
            let options = value.options();
            let tip = "An option of this gate: one Approve button each";
            let mut chips: Vec<_> = options
                .iter()
                .map(|option| ("", option.as_str(), tip))
                .collect();
            let pick_label = format!("pick {}", pick.trim());
            if !pick.trim().is_empty() {
                chips.push(("GitBranch", pick_label.as_str(), "Pick winning racer"));
            }
            chip_row(ui, &chips, colors);
        }
    }
    branch_chips(ui, value, colors);
}

/// The condition and the loop of a card as chips.
fn branch_chips(ui: &mut egui::Ui, value: &Node, colors: Colors) {
    let branch = value.branch();
    let when = format!("when {}", branch.when.trim());
    let back = format!(
        "loop to {} · {} rounds",
        branch.loop_to.trim(),
        branch.max_rounds
    );
    // One row each: a long chip never squeezes the other.
    if !branch.when.trim().is_empty() {
        let tip = "Runs only when this gate option is chosen; else it is skipped";
        chip_row(ui, &[("Split", when.as_str(), tip)], colors);
    }
    if !branch.loop_to.trim().is_empty() {
        let tip = "When this step is done, the run starts again from that step";
        chip_row(ui, &[("Repeat", back.as_str(), tip)], colors);
    }
}

/// The chips with text, in a row that wraps at the card width.
fn chip_row(ui: &mut egui::Ui, chips: &[(&str, &str, &str)], colors: Colors) {
    if chips.iter().all(|(_, text, _)| text.is_empty()) {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(CARD_WIDTH);
        for chip in chips.iter().filter(|(_, text, _)| !text.is_empty()) {
            icon_chip(ui, *chip, colors);
        }
    });
}

impl Viewer<'_> {
    /// The run footer of a card: the one action that applies now.
    pub(super) fn footer(&mut self, ui: &mut egui::Ui, id: &str, step: &StepRun) {
        ui.add_space(2.0);
        ui.separator();
        if gate_waits(step) {
            if let Some(target) = &step.pick {
                let racers = self
                    .run
                    .and_then(|steps| steps.get(target))
                    .map(|s| s.racers.as_slice())
                    .unwrap_or(&[]);
                if let Some(choice) =
                    crate::flows::gates::pick_buttons(ui, racers, &mut self.out.open_diff)
                {
                    self.out.answer = Some((id.to_owned(), choice));
                }
            } else if let Some(choice) = crate::flows::gates::gate_buttons(ui, &step.options) {
                self.out.answer = Some((id.to_owned(), choice));
            }
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
        if step.status == StepStatus::Running
            && let Some(update) = self.updates.get(id)
        {
            clipped(ui, update, 1, self.colors.weak);
        }
        // The choice, round, attempt and stall: display only. The card's
        // chip shows the condition already.
        let badges: Vec<String> = crate::flows::gates::branch_labels(step)
            .into_iter()
            .filter(|label| !label.starts_with("when "))
            .chain(crate::flows::status::attempt_label(step))
            .chain(self.stalls.get(id).cloned())
            .collect();
        if !badges.is_empty() {
            clipped(ui, &badges.join(" · "), 1, self.colors.weak);
        }
        if !step.outputs.is_empty() {
            let chips: Vec<_> = step
                .outputs
                .iter()
                .map(|path| ("File", path.as_str(), "Output reported by this step"))
                .collect();
            chip_row(ui, &chips, self.colors);
        }
        self.footer_racers(ui, step);
        self.footer_actions(ui, id, step);
    }

    fn footer_racers(&mut self, ui: &mut egui::Ui, step: &StepRun) {
        for racer in &step.racers {
            let name = match &racer.agent {
                Some(agent) => format!("#{} ({agent})", racer.index),
                None => format!("#{}", racer.index),
            };
            let diffstat = match (racer.added, racer.deleted) {
                (Some(a), Some(d)) => format!(" +{a} −{d}"),
                _ => String::new(),
            };
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(CARD_WIDTH);
                ui.label(
                    egui::RichText::new(format!("{name}: {:?}{diffstat}", racer.status))
                        .size(appearance::FONT_CAPTION)
                        .color(self.colors.weak),
                );
                if let Some(session) = &racer.session
                    && appearance::link(
                        ui,
                        egui::RichText::new("Terminal").size(appearance::FONT_CAPTION),
                    )
                    .clicked()
                {
                    self.out.go = Some(session.clone());
                }
                if let Some(path) = &racer.worktree_path
                    && appearance::link(
                        ui,
                        egui::RichText::new("Diff").size(appearance::FONT_CAPTION),
                    )
                    .clicked()
                {
                    self.out.open_diff = Some(path.clone());
                }
            });
        }
    }

    fn footer_actions(&mut self, ui: &mut egui::Ui, id: &str, step: &StepRun) {
        if self.live && step.status == StepStatus::Running && step.gate.is_none() {
            if step.inbox > 0 {
                clipped(
                    ui,
                    &appearance::count_noun(step.inbox as usize, "queued note"),
                    1,
                    self.colors.weak,
                );
            }
            self.mail_row(ui, id);
        }
        if self.live && step.retryable() {
            appearance::footer_row(ui, |ui| {
                if appearance::primary_button(ui, "Retry")
                    .on_hover_text("Start this step again in a new terminal")
                    .clicked()
                {
                    self.out.retry = Some(id.to_owned());
                }
            });
        }
        if let Some(session) = &step.session {
            ui.horizontal(|ui| {
                let label = egui::RichText::new("Open terminal").size(appearance::FONT_CAPTION);
                if appearance::link(ui, label)
                    .on_hover_text("Go to the terminal of this step")
                    .clicked()
                {
                    self.out.go = Some(session.clone());
                }
                let peek_label = egui::RichText::new("Peek").size(appearance::FONT_CAPTION);
                if appearance::link(ui, peek_label)
                    .on_hover_text("Peek this step's screen (read-only)")
                    .clicked()
                {
                    self.out.peek = Some(session.clone());
                }
            });
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

    /// Mailbox input for a running command step (`flow send`); never writes to a terminal.
    fn mail_row(&mut self, ui: &mut egui::Ui, id: &str) {
        let draft = self.mail_drafts.entry(id.to_owned()).or_default();
        let mut send = false;
        appearance::footer_row(ui, |ui| {
            let filled = !draft.trim().is_empty();
            send = ui
                .add_enabled(filled, egui::Button::new("Send"))
                .on_hover_text("Store a note in this step's mailbox (read via flow inbox)")
                .clicked();
            let field = ui.add(
                appearance::singleline(draft)
                    .hint_text("Send note to inbox…")
                    .desired_width(ui.available_width()),
            );
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            send = (send || entered) && filled;
        });
        if send {
            self.out.send_mail = Some(id.to_owned());
        }
    }
}
