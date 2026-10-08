use super::state::SettingsSection;
use crate::*;

impl App {
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
                let row = appearance::row(
                    ui,
                    section.title(),
                    section.icon(),
                    self.settings_section == section,
                    30.0,
                    "",
                    ui.visuals().weak_text_color(),
                );
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
