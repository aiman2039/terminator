//! The status-bar message chip, its details popover, and the message log.
//! The chip is one row at the toolbar height: an icon and a one-line
//! summary that ends with an ellipsis. The full output, Copy, Dismiss and
//! Retry are in the popover; the last messages are in the log.

use super::status_line;
use crate::*;

/// The chip's summary takes at most this share of the window width.
const CHIP_SHARE: f32 = 0.4;

/// What a click in the details popover asks for.
enum DetailsAction {
    Copy(String),
    Dismiss,
    Retry(workspace_ops::Op),
}

impl App {
    /// The shown error: the chip and its Dismiss button. A click on the
    /// chip opens or closes the details popover.
    pub(crate) fn status_error_chip(&mut self, ui: &mut egui::Ui) {
        let Some(summary) = self.error.summary() else {
            return;
        };
        let failed = appearance::color(&self.theme.status_failed);
        let max_width = (ui.ctx().content_rect().width() * CHIP_SHARE).max(120.0);
        let chip = appearance::icon_label_chip(appearance::IconLabelChipInput {
            ui,
            icon: "CircleAlert",
            label: &summary,
            tint: failed,
            height: appearance::TOOLBAR_BUTTON,
            max_width,
        })
        .on_hover_text("Show details");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-error-chip", chip.rect);
        if chip.clicked() {
            self.error.details_open = !self.error.details_open;
        }
        let dismiss = appearance::sidebar_action(ui, "X", "Dismiss");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-error-dismiss", dismiss.rect);
        if dismiss.clicked() {
            self.dismiss_status_error();
            return;
        }
        if self.error.details_open {
            self.status_error_details(ui.ctx(), chip.rect);
        }
    }

    /// Popover above the chip: title, the full output, Copy, Dismiss, and
    /// Retry when the failed job can run again.
    fn status_error_details(&mut self, ctx: &egui::Context, anchor: egui::Rect) {
        let Some(text) = self.error.text().map(str::to_owned) else {
            return;
        };
        let origin = self.error.origin().cloned();
        let title = origin
            .as_ref()
            .map_or_else(|| "Error".to_owned(), |o| format!("{} failed", o.operation));
        let failed = appearance::color(&self.theme.status_failed);
        let mut action = None;
        egui::Area::new(egui::Id::unique("status-error-details"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::LEFT_BOTTOM)
            .fixed_pos(egui::pos2(anchor.left(), anchor.top() - 8.0))
            .constrain(true)
            .show(ctx, |ui| {
                appearance::popup_frame(ui.style()).show(ui, |ui| {
                    ui.set_width(440.0);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Image::new(icons::source("CircleAlert"))
                                .tint(failed)
                                .fit_to_exact_size(egui::Vec2::splat(appearance::TOOLBAR_ICON)),
                        );
                        ui.strong(&title);
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("status-error-output")
                        .max_height(160.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(egui::RichText::new(text.trim_end()).monospace())
                                    .wrap(),
                            );
                        });
                    ui.add_space(8.0);
                    appearance::footer_row(ui, |ui| {
                        if let Some(op) = origin.as_ref().and_then(|o| o.retry.clone()) {
                            let retry = appearance::primary_button(ui, "Retry");
                            #[cfg(feature = "test-support")]
                            diagnostics::record_ui(ui, "status-error-retry", retry.rect);
                            if retry.clicked() {
                                action = Some(DetailsAction::Retry(op));
                            }
                        }
                        if ui.button("Dismiss").clicked() {
                            action = Some(DetailsAction::Dismiss);
                        }
                        let copy = ui.button("Copy");
                        #[cfg(feature = "test-support")]
                        diagnostics::record_ui(ui, "status-error-copy", copy.rect);
                        if copy.clicked() {
                            action = Some(DetailsAction::Copy(text.clone()));
                        }
                    });
                });
            });
        match action {
            Some(DetailsAction::Copy(text)) => ctx.copy_text(text),
            Some(DetailsAction::Dismiss) => self.dismiss_status_error(),
            // The same queue as the first try, so a running commit still
            // refuses an action that could change it.
            Some(DetailsAction::Retry(op)) => {
                if self.queue_workspace(op) {
                    self.error.clear();
                }
            }
            None => {}
        }
    }

    /// The message log button: the unread error count, if any. A click
    /// opens the last messages.
    pub(crate) fn status_log_button(&mut self, ui: &mut egui::Ui) {
        let unread = self.error.unread();
        let button = if unread > 0 {
            appearance::icon_label_button(appearance::IconLabelButtonInput {
                ui,
                icon: "List",
                label: &unread.to_string(),
                height: appearance::TOOLBAR_BUTTON,
            })
            .on_hover_text("Messages")
        } else {
            appearance::sidebar_action(ui, "List", "Messages")
        };
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-log", button.rect);
        if button.clicked() {
            self.error.log_open = !self.error.log_open;
            self.error.mark_read();
        }
        if self.error.log_open {
            self.status_log_popup(ui.ctx(), button.rect);
        }
    }

    fn status_log_popup(&mut self, ctx: &egui::Context, anchor: egui::Rect) {
        let entries: Vec<status_line::LogEntry> = self.error.log().cloned().collect();
        let failed = appearance::color(&self.theme.status_failed);
        let mut clear = false;
        let mut open_url = None;
        egui::Area::new(egui::Id::unique("status-log"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .fixed_pos(egui::pos2(anchor.right(), anchor.top() - 8.0))
            .constrain(true)
            .show(ctx, |ui| {
                appearance::popup_frame(ui.style()).show(ui, |ui| {
                    ui.set_width(400.0);
                    ui.horizontal(|ui| {
                        ui.strong("Messages");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            clear = ui.small_button("Clear all").clicked();
                        });
                    });
                    if entries.is_empty() {
                        ui.weak("No messages");
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("status-log-rows")
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for entry in &entries {
                                if let Some(url) = log_row(ui, entry, failed) {
                                    open_url = Some(url);
                                }
                            }
                        });
                });
            });
        if clear {
            self.error.clear_log();
        }
        if let Some(url) = open_url {
            let _ = self.jobs.send(Job::Browser(url));
        }
    }
}

/// One log row: icon, the message's first line (a URL in it is a link),
/// and its age. Returns a clicked URL.
fn log_row(
    ui: &mut egui::Ui,
    entry: &status_line::LogEntry,
    failed: egui::Color32,
) -> Option<String> {
    let (icon, tint) = if entry.error {
        ("CircleAlert", failed)
    } else {
        ("Info", ui.visuals().weak_text_color())
    };
    let line = status_line::first_line(&entry.text);
    let url = line
        .split_whitespace()
        .find(|word| word.starts_with("https://") || word.starts_with("http://"));
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.add(
            egui::Image::new(icons::source(icon))
                .tint(tint)
                .fit_to_exact_size(egui::Vec2::splat(appearance::TOOLBAR_ICON)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.weak(egui::RichText::new(age(entry.at.elapsed())).small());
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let row = match url {
                    Some(url) => {
                        let link = ui.link(egui::RichText::new(line).small());
                        if link.clicked() {
                            clicked = Some(url.to_owned());
                        }
                        link
                    }
                    None => ui.add(egui::Label::new(egui::RichText::new(line).small()).truncate()),
                };
                row.on_hover_text(&entry.text);
            });
        });
    });
    clicked
}

/// "now", "5 min ago", "2 h ago".
fn age(elapsed: std::time::Duration) -> String {
    let minutes = elapsed.as_secs() / 60;
    match minutes {
        0 => "now".into(),
        1..=59 => format!("{minutes} min ago"),
        _ => format!("{} h ago", minutes / 60),
    }
}

#[cfg(test)]
#[test]
fn log_ages_read_as_minutes_then_hours() {
    use std::time::Duration;
    assert_eq!(age(Duration::from_secs(10)), "now");
    assert_eq!(age(Duration::from_mins(5)), "5 min ago");
    assert_eq!(age(Duration::from_mins(122)), "2 h ago");
}
