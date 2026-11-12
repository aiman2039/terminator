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
                let row = appearance::text_tab(
                    ui,
                    section.title(),
                    self.settings_section.canonical() == section,
                );
                #[cfg(feature = "test-support")]
                record_section(ui, section, row.rect);
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

    /// Whether the search hides every section.
    pub(super) fn settings_search_empty(&self) -> bool {
        !SettingsSection::ALL
            .into_iter()
            .any(|section| self.section_visible(section))
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
                    selected: self.settings_section.canonical() == section,
                    height: 30.0,
                    trailing: "",
                });
                #[cfg(feature = "test-support")]
                record_section(ui, section, row.rect);
                if row.clicked() {
                    self.request_settings_section(section);
                }
            }
        });
    }

    /// No section matches the search: say so, with a way back.
    pub(super) fn paint_settings_no_match(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            ui.label(format!(
                "No settings match “{}”.",
                self.settings_search.trim()
            ));
            ui.add_space(8.0);
            if ui.button("Clear search").clicked() {
                self.settings_search.clear();
            }
        });
    }
}

/// The section's nav row under its fixture id and its older ids.
#[cfg(feature = "test-support")]
fn record_section(ui: &egui::Ui, section: SettingsSection, rect: egui::Rect) {
    for name in std::iter::once(section.title()).chain(section.fixture_aliases().iter().copied()) {
        diagnostics::record(ui.ctx(), &format!("settings-section:{name}"), rect);
    }
}
