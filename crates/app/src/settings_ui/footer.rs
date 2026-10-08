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
                settings_controls::settings_row(
                    ui,
                    "Diff default layout",
                    "Default view for new native Git diffs.",
                    |ui| {
                        settings_controls::segmented(
                            ui,
                            &mut self.settings_draft.diff_split_default,
                            &[("Unified", false), ("Side by side", true)],
                        );
                    },
                );
            }
            if self.field_visible("Editor close timeout", "seconds delay quit") {
                settings_controls::settings_row(
                    ui,
                    "Editor close timeout",
                    "How long to wait for Neovim to close before force-stopping.",
                    |ui| {
                        ui.add(
                            egui::Slider::new(
                                &mut self.settings_draft.editor_close_timeout_secs,
                                1..=30,
                            )
                            .suffix(" s"),
                        );
                    },
                );
            }
        });
        response.response.on_disabled_hover_text(
            "These settings require an updated daemon. Restart it after finishing your live sessions.",
        )
    }

    pub(crate) fn settings_center(&mut self, ui: &mut egui::Ui) {
        let mut frame = SettingsFrame::default();
        self.paint_settings(ui, &mut frame);
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
        ui.horizontal(|ui| {
            ui.add(
                appearance::singleline(&mut self.settings_search)
                    .hint_text("Search settings")
                    .desired_width(280.0),
            );
            if !self.settings_search.is_empty() && ui.small_button("Clear").clicked() {
                self.settings_search.clear();
            }
        });
        ui.add_space(8.0);
        self.ensure_visible_section();
        let footer_h = 44.0;
        let body_h = (ui.available_height() - footer_h).max(80.0);
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
        ui.separator();
        self.paint_settings_footer(ui, frame);
    }

    fn paint_settings_footer(&mut self, ui: &mut egui::Ui, frame: &mut SettingsFrame) {
        let validation = self.settings_validation();
        if let Err(error) = &validation {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    validation.is_ok() && !self.theme_conflict,
                    egui::Button::new("Apply"),
                )
                .clicked()
            {
                frame.apply = true;
            }
            let response = ui.button("Cancel");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "settings-cancel", response.rect);
            if response.clicked() {
                frame.cancel = true;
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
            self.add_project = true;
        }
        if let Some(target) = browse {
            self.browse_target = Some(target);
        }
        if apply {
            self.apply_settings();
            self.settings_pending = None;
        }
        if cancel || hide {
            self.request_settings_close();
        }
    }
}
