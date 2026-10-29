//! Fixed-height notification bar under the diff toolbar. Status messages,
//! the discard confirmation, and hunk progress show here, so the panes
//! below never move when a message comes or goes.
use super::super::super::*;
use super::{DiffFrame, DiffPass};

/// Height of the bar; it is reserved even when there is nothing to say.
pub(super) const NOTICE_HEIGHT: f32 = 26.0;

/// What the bar shows, most urgent first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Notice {
    /// Discard of change `index` waits for confirmation.
    ConfirmDiscard(usize),
    /// A hunk request is running.
    Busy,
    /// The last request's result or error.
    Message(String),
    /// A read-only snapshot hint.
    ReadOnly(&'static str),
    /// Nothing to report.
    Empty,
}

/// The notice for this frame.
pub(super) fn notice(frame: &DiffFrame<'_>, review: &diff::review::State) -> Notice {
    if let Some(index) = review.confirm_discard {
        Notice::ConfirmDiscard(index)
    } else if review.busy {
        Notice::Busy
    } else if let Some(message) = &review.message {
        Notice::Message(message.clone())
    } else if frame.remote {
        Notice::ReadOnly("Remote snapshot · read-only. Use the SSH terminal to edit Git state.")
    } else if frame.read_only {
        Notice::ReadOnly("Perforce snapshot · read-only. Use Open file to edit the workspace file.")
    } else {
        Notice::Empty
    }
}

impl App {
    pub(super) fn diff_notice_bar(ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let width = ui.available_width();
        let bar = notice(frame, &pass.review);
        let fill = match bar {
            Notice::ConfirmDiscard(_) => ui.visuals().extreme_bg_color,
            _ => Color32::TRANSPARENT,
        };
        egui::Frame::NONE.fill(fill).show(ui, |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(width, NOTICE_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_size(egui::vec2(width, NOTICE_HEIGHT));
                    ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                    ui.add_space(6.0);
                    Self::diff_notice_content(ui, frame, pass, bar);
                },
            );
        });
    }

    fn diff_notice_content(
        ui: &mut egui::Ui,
        frame: &DiffFrame<'_>,
        pass: &mut DiffPass,
        bar: Notice,
    ) {
        match bar {
            Notice::ConfirmDiscard(index) => {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("Discard change {} from disk?", index.saturating_add(1)),
                );
                let discard = ui.add_enabled(frame.enabled, egui::Button::new("Discard hunk"));
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "diff-confirm-discard", discard.rect);
                if discard.clicked() {
                    pass.confirmed = Some(index);
                }
                if ui.button("Cancel").clicked() {
                    pass.review.confirm_discard = None;
                }
            }
            Notice::Busy => {
                ui.spinner();
                ui.label("Applying hunk…");
            }
            Notice::Message(message) => {
                ui.add(egui::Label::new(message).truncate());
                if appearance::sidebar_action(ui, "X", "Dismiss").clicked() {
                    pass.review.message = None;
                }
            }
            Notice::ReadOnly(text) => {
                ui.add(egui::Label::new(egui::RichText::new(text).weak()).truncate());
            }
            Notice::Empty => {}
        }
    }
}
