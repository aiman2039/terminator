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
    /// A review note is being typed.
    Note,
    /// A hunk request is running.
    Busy,
    /// Lines picked in hunk `index` for a line-level action.
    Picked { hunk: usize, count: usize },
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
    } else if review.note.is_some() {
        Notice::Note
    } else if review.busy {
        Notice::Busy
    } else if let Some((hunk, lines)) = review.picked.as_ref().filter(|(_, l)| !l.is_empty()) {
        Notice::Picked {
            hunk: *hunk,
            count: lines.len(),
        }
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
                let picked = pass.review.picked_in(index);
                let what = if picked == 0 {
                    format!("change {}", index.saturating_add(1))
                } else {
                    format!(
                        "{} of change {}",
                        diff::actions::lines_label(picked, "picked"),
                        index.saturating_add(1)
                    )
                };
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("Discard {what} from disk? (Enter)"),
                );
                let discard = ui.add_enabled(frame.enabled, egui::Button::new("Discard"));
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "diff-confirm-discard", discard.rect);
                if discard.clicked() {
                    pass.confirmed = Some(index);
                }
                if ui.button("Cancel").clicked() {
                    pass.review.confirm_discard = None;
                }
            }
            Notice::Note => Self::diff_note_field(ui, pass),
            Notice::Busy => {
                ui.spinner();
                ui.label("Applying hunk…");
            }
            Notice::Picked { hunk, count } => {
                ui.label(format!(
                    "{} picked in change {} · Esc to clear",
                    diff::actions::lines_label(count, ""),
                    hunk.saturating_add(1)
                ));
                if appearance::sidebar_action(ui, "X", "Clear picked lines (Esc)").clicked() {
                    pass.review.clear_picks();
                }
            }
            Notice::Message(message) => {
                if pass.review.offer_undo && !pass.review.undo.is_empty() {
                    let undo = ui.add_enabled(frame.enabled, egui::Button::new("Undo"));
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), "diff-notice-undo", undo.rect);
                    if undo.clicked() {
                        pass.undo = true;
                        pass.review.offer_undo = false;
                    }
                }
                ui.add(egui::Label::new(message).truncate());
                if appearance::sidebar_action(ui, "X", "Dismiss").clicked() {
                    pass.review.message = None;
                    pass.review.offer_undo = false;
                }
            }
            Notice::ReadOnly(text) => {
                ui.add(egui::Label::new(egui::RichText::new(text).weak()).truncate());
            }
            Notice::Empty => {}
        }
    }

    /// The note field: Enter or Add keeps the note, Esc or Cancel drops it.
    fn diff_note_field(ui: &mut egui::Ui, pass: &mut DiffPass) {
        let Some(draft) = pass.review.note.as_mut() else {
            return;
        };
        let kind = if draft.removed {
            " (removed lines)"
        } else {
            ""
        };
        ui.weak(format!("Note on {}{kind}:", draft.lines()));
        let width = (ui.available_width() - 140.0).clamp(120.0, 420.0);
        let field = ui.add(
            appearance::singleline(&mut draft.text)
                .hint_text("What should the agent change?")
                .desired_width(width),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-note-field", field.rect);
        if std::mem::take(&mut draft.focus) {
            field.request_focus();
        }
        let (enter, escape) = ui.input(|input| {
            (
                input.key_pressed(egui::Key::Enter),
                input.key_pressed(egui::Key::Escape),
            )
        });
        let filled = !draft.text.trim().is_empty();
        let add = ui.add_enabled(filled, egui::Button::new("Add note"));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-note-add", add.rect);
        let submitted = add.clicked() || (field.lost_focus() && enter && filled);
        if submitted {
            pass.add_note = pass.review.note.take();
        } else if ui.button("Cancel").clicked() || (field.lost_focus() && escape) {
            pass.review.note = None;
        }
    }
}
