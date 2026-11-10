use super::state::SettingsSection;
use crate::*;

impl App {
    /// Narrow cards put section choices above the form instead of beside it.
    pub(super) fn paint_settings_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for section in SettingsSection::ALL {
                if !self.section_visible(section) {
                    continue;
                }
                let row =
                    appearance::text_tab(ui, section.title(), self.settings_section == section);
                #[cfg(feature = "test-support")]
                diagnostics::record(
                    ui.ctx(),
                    &format!("settings-section:{}", section.title()),
                    row.rect,
                );
                if row.clicked() {
                    self.request_settings_section(section);
                }
            }
        });
    }

    pub(super) fn ensure_visible_section(&mut self) {
        if !self.section_visible(self.settings_section)
            && let Some(section) = SettingsSection::ALL
                .into_iter()
                .find(|section| self.section_visible(*section))
        {
            self.settings_section = section;
        }
    }

    pub(super) fn paint_settings_nav(&mut self, ui: &mut egui::Ui, height: f32) {
        ui.vertical(|ui| {
            ui.set_width(142.0);
            ui.set_min_height(height);
            ui.spacing_mut().item_spacing.y = 4.0;
            for section in SettingsSection::ALL {
                if !self.section_visible(section) {
                    continue;
                }
                let row = appearance::row(appearance::RowInput {
                    tint: ui.visuals().weak_text_color(),
                    ui,
                    label: section.title(),
                    icon: section.icon(),
                    selected: self.settings_section == section,
                    height: 30.0,
                    trailing: "",
                });
                #[cfg(feature = "test-support")]
                diagnostics::record(
                    ui.ctx(),
                    &format!("settings-section:{}", section.title()),
                    row.rect,
                );
                if row.clicked() {
                    self.request_settings_section(section);
                }
            }
        });
    }
}
