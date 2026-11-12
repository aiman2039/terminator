use super::state::SettingsFrame;
use crate::*;

impl App {
    pub(super) fn diff_close_settings(&mut self, ui: &mut egui::Ui) -> egui::Response {
        let supported = self
            .state
            .capabilities
            .iter()
            .any(|c| c == DIFF_CLOSE_SETTINGS_CAPABILITY);
        let response = ui.add_enabled_ui(supported, |ui| {
            if self.field_visible("Diff default layout", "unified split side by side") {
                settings_controls::settings_row(settings_controls::SettingsRowInput {
                    ui,
                    label: "Diff default layout",
                    description: "Default view for new native Git diffs.",
                    add_control: |ui| {
                        settings_controls::segmented(
                            ui,
                            &mut self.settings_draft.diff_split_default,
                            &[("Unified", false), ("Side by side", true)],
                        );
                    },
                });
            }
            if self.field_visible("Editor close timeout", "seconds delay quit") {
                settings_controls::settings_row(settings_controls::SettingsRowInput {
                    ui,
                    label: "Editor close timeout",
                    description: "How long to wait for Neovim to close before force-stopping.",
                    add_control: |ui| {
                        ui.add(
                            egui::Slider::new(
                                &mut self.settings_draft.editor_close_timeout_secs,
                                1..=30,
                            )
                            .suffix(" s"),
                        );
                    },
                });
            }
        });
        self.service_caption(ui, supported);
        response
            .response
            .on_disabled_hover_text(settings_controls::NEEDS_SERVICE)
    }

    /// Under a control the session service cannot use yet: the one
    /// caption, whose link opens Updates.
    pub(super) fn service_caption(&mut self, ui: &mut egui::Ui, supported: bool) {
        if !supported && settings_controls::needs_service_caption(ui) {
            self.request_settings_section(SettingsSection::Updates);
        }
    }

    /// Settings paints as a card inset in the center pane. A click on the
    /// margin around the card closes Settings (unsaved changes still ask).
    pub(crate) fn settings_center(&mut self, ui: &mut egui::Ui) {
        let mut frame = SettingsFrame::default();
        let style = appearance::CenterCardStyle::new(&self.theme);
        let ((), dismissed) =
            appearance::center_card(ui, style, |ui| self.paint_settings(ui, &mut frame));
        frame.hide |= dismissed;
        self.finish_settings_frame(frame);
    }

    fn paint_settings(&mut self, ui: &mut egui::Ui, frame: &mut SettingsFrame) {
        ui.set_min_size(ui.available_size());
        ui.horizontal(|ui| {
            ui.strong("Settings");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    frame.hide = true;
                }
            });
        });
        ui.add_space(6.0);
        let search_width = (ui.available_width() - 32.0).clamp(40.0, 280.0);
        ui.horizontal(|ui| {
            ui.add(
                appearance::singleline(&mut self.settings_search)
                    .hint_text("Search settings")
                    .desired_width(search_width),
            );
            if !self.settings_search.is_empty()
                && appearance::sidebar_action(ui, "X", "Clear search").clicked()
            {
                self.settings_search.clear();
            }
        });
        ui.add_space(8.0);
        self.ensure_visible_section();
        let narrow = ui.available_width() < 620.0;
        if narrow {
            self.paint_settings_tabs(ui);
            ui.separator();
        }
        let footer_h = 44.0;
        let body_h = (ui.available_height() - footer_h).max(80.0);
        if narrow {
            self.paint_settings_body(ui, body_h, frame);
        } else {
            ui.horizontal_top(|ui| {
                ui.set_min_height(body_h);
                self.paint_settings_nav(ui, body_h);
                let divider = ui.cursor().min;
                ui.painter().line_segment(
                    [divider, egui::pos2(divider.x, divider.y + body_h)],
                    ui.visuals().widgets.noninteractive.bg_stroke,
                );
                ui.add_space(12.0);
                self.paint_settings_body(ui, body_h, frame);
            });
        }
        ui.separator();
        self.paint_settings_footer(ui, frame);
    }

    fn paint_settings_footer(&mut self, ui: &mut egui::Ui, frame: &mut SettingsFrame) {
        let validation = self.settings_validation();
        if let Err(error) = &validation {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
        }
        let count = self.settings_change_count();
        let none = (count == 0).then_some("No changes to apply");
        // Right to left: Apply at the far right, Discard changes left of
        // it, and the pending count at the left edge.
        appearance::footer_row(ui, |ui| {
            let why = if self.theme_conflict {
                Some("Appearance changed externally. Reload it in Appearance before applying.")
            } else {
                validation.as_ref().err().map(String::as_str).or(none)
            };
            let response =
                appearance::enabled_unless(ui, why, |ui| appearance::primary_button(ui, "Apply"));
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "settings-apply", response.rect);
            if response.clicked() {
                frame.apply = true;
            }
            let response = appearance::enabled_unless(ui, none, |ui| ui.button("Discard changes"));
            // `settings-cancel` stays the fixture id of the left button.
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "settings-cancel", response.rect);
            if response.clicked() {
                frame.cancel = true;
            }
            if count > 0 {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.weak(format!(
                        "{} not applied",
                        appearance::count_noun(count, "change")
                    ));
                });
            }
        });
    }

    fn finish_settings_frame(&mut self, frame: SettingsFrame) {
        let SettingsFrame {
            apply,
            cancel,
            hide,
            browse,
            grant_folder,
        } = frame;
        if grant_folder {
            self.request_pick(PickKind::Project);
        }
        if let Some(target) = browse {
            self.request_pick(PickKind::Browse(target));
        }
        if apply {
            self.apply_settings();
            self.settings_pending = None;
        }
        if cancel {
            self.discard_settings_draft();
        }
        if hide {
            self.request_settings_close();
        }
    }
}
