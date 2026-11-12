use crate::*;

/// Preset slots shown by the Theme segmented control; `CUSTOM` is never selectable.
const SAVED: usize = 0;
const CUSTOM: usize = PRESETS.len() + 1;

/// Built-in presets in control order; slot `index + 1` picks `PRESETS[index]`.
type Preset = (&'static str, fn() -> AppearanceConfig);

const PRESETS: [Preset; 6] = [
    ("Carbon", AppearanceConfig::carbon),
    ("Islands dark", AppearanceConfig::islands_dark),
    ("IntelliJ dark", AppearanceConfig::intellij_dark),
    ("High contrast", AppearanceConfig::high_contrast),
    ("Terminator dark", AppearanceConfig::terminator_dark),
    ("IntelliJ light", AppearanceConfig::intellij_light),
];

impl App {
    pub(super) fn appearance_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Appearance");
        self.theme_conflict_banner(ui);
        ui.weak("Preview colors and borders. Apply saves your changes.");
        ui.add_space(8.0);
        self.theme_preset_row(ui);
        self.accent_row(ui);
        self.density_row(ui);
        self.font_rows(ui);
        if ui.button("Reset appearance").clicked() {
            self.theme_draft = AppearanceConfig::default();
        }
        ui.add_space(8.0);
        self.color_groups(ui);
    }

    fn theme_conflict_banner(&mut self, ui: &mut egui::Ui) {
        if !self.theme_conflict {
            return;
        }
        ui.colored_label(
            appearance::color(&self.theme.status_failed),
            "Configuration changed externally. Reload before applying.",
        );
        if ui.button("Reload appearance").clicked() {
            self.theme_draft = self.theme_committed.clone();
            self.theme_conflict = false;
        }
    }

    fn theme_preset_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Theme", "preset dark contrast") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Theme",
            description: "Presets fill colors. Advanced hex stays available below.",
            add_control: |ui| {
                let mut choice = self.theme_preset_choice();
                let options: Vec<(&str, usize)> = std::iter::once(("Current saved", SAVED))
                    .chain(
                        PRESETS
                            .iter()
                            .enumerate()
                            .map(|(index, (label, _))| (*label, index.saturating_add(1))),
                    )
                    .collect();
                if settings_controls::segmented(ui, &mut choice, &options) {
                    self.apply_theme_preset(choice);
                }
                if choice == CUSTOM {
                    ui.weak("Custom colors");
                }
            },
        });
    }

    /// Which preset the draft colors currently match, or `CUSTOM`.
    fn theme_preset_choice(&self) -> usize {
        if self.theme_draft.colors_match(&self.theme_committed) {
            return SAVED;
        }
        PRESETS
            .iter()
            .position(|(_, preset)| self.theme_draft.colors_match(&preset()))
            .map_or(CUSTOM, |index| index.saturating_add(1))
    }

    /// Replace the draft colors with a preset, keeping the sizing fields.
    fn apply_theme_preset(&mut self, choice: usize) {
        let density = self.theme_draft.density;
        let divider = self.theme_draft.pane_divider_width;
        let border = self.theme_draft.border_width;
        let ui_scale = self.theme_draft.ui_scale;
        let editor = self.theme_draft.editor;
        let fonts = std::mem::take(&mut self.theme_draft.fonts);
        self.theme_draft = choice
            .checked_sub(1)
            .and_then(|index| PRESETS.get(index))
            .map_or_else(|| self.theme_committed.clone(), |(_, preset)| preset());
        self.theme_draft.density = density;
        self.theme_draft.pane_divider_width = divider;
        self.theme_draft.border_width = border;
        self.theme_draft.ui_scale = ui_scale;
        self.theme_draft.editor = editor;
        self.theme_draft.fonts = fonts;
    }

    fn accent_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Accent", "accent selection") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Accent",
            description: "Selection follows the accent, except in neutral themes.",
            add_control: |ui| {
                let mut color = terminator_core::appearance::rgb(&self.theme_draft.accent)
                    .unwrap_or([56, 113, 225]);
                if ui.color_edit_button_srgb(&mut color).changed() {
                    self.theme_draft.apply_accent(ThemeHelpers::hex(color));
                }
            },
        });
    }

    fn density_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Density", "compact comfortable spacing") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Density",
            description: "Compact shortens sidebar rows and control height.",
            add_control: |ui| {
                settings_controls::segmented(
                    ui,
                    &mut self.theme_draft.density,
                    &[
                        (
                            "Comfortable",
                            terminator_core::appearance::Density::Comfortable,
                        ),
                        ("Compact", terminator_core::appearance::Density::Compact),
                    ],
                );
            },
        });
        self.interface_size_row(ui);
    }

    fn interface_size_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Interface size", "zoom scale text ui larger smaller") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Interface size",
            description: "Scales the whole window, terminal and editor text included.",
            add_control: |ui| {
                let labels = ["90 %", "100 %", "110 %", "125 %", "150 %"];
                let options: Vec<(&str, f32)> = labels
                    .into_iter()
                    .zip(AppearanceConfig::UI_SCALES)
                    .collect();
                settings_controls::segmented(ui, &mut self.theme_draft.ui_scale, &options);
            },
        });
    }

    fn color_groups(&mut self, ui: &mut egui::Ui) {
        for (group, title, keywords) in [
            (0, "Interface", "window surface hover border text"),
            (1, "Terminal", "terminal background foreground"),
            (2, "Git status", "git added modified deleted"),
            (3, "Agent status", "running waiting failed"),
        ] {
            if !self.field_visible(title, keywords) && !self.settings_search.is_empty() {
                continue;
            }
            // A search that kept this group open shows its fields.
            let searching = !self.settings_search.trim().is_empty();
            egui::CollapsingHeader::new(title)
                .id_salt(("appearance-group", group))
                .default_open(false)
                .open(searching.then_some(true))
                .show(ui, |ui| {
                    egui::Grid::new(("appearance-fields", group))
                        .num_columns(2)
                        .min_col_width(155.0)
                        .spacing([20.0, 10.0])
                        .show(ui, |ui| self.color_group_fields(ui, group));
                });
        }
    }

    fn color_group_fields(&mut self, ui: &mut egui::Ui, group: i32) {
        let failed = appearance::color(&self.theme.status_failed);
        for (name, value) in self.theme_draft.colors_mut() {
            if ThemeHelpers::color_category(name) != group {
                continue;
            }
            ui.label(ThemeHelpers::color_label(name));
            ui.horizontal(|ui| ThemeHelpers::color_field(ui, value, failed));
            ui.end_row();
        }
        if group == 0 {
            self.primary_field(ui, failed);
            self.border_width_fields(ui);
        }
    }

    /// Optional main-button fill; unset follows the accent.
    fn primary_field(&mut self, ui: &mut egui::Ui, failed: egui::Color32) {
        ui.label("Main button");
        ui.horizontal(|ui| {
            if let Some(value) = &mut self.theme_draft.primary {
                ThemeHelpers::color_field(ui, value, failed);
                if ui.button("Use accent").clicked() {
                    self.theme_draft.primary = None;
                }
            } else {
                ui.weak("Accent");
                if ui.button("Set color").clicked() {
                    self.theme_draft.primary = Some(self.theme_draft.accent.clone());
                }
            }
        });
        ui.end_row();
    }

    fn border_width_fields(&mut self, ui: &mut egui::Ui) {
        ui.label("Border width");
        ui.add(
            egui::DragValue::new(&mut self.theme_draft.border_width)
                .range(0.0..=8.0)
                .speed(0.1)
                .suffix(" pt"),
        );
        ui.end_row();
        ui.label("Pane divider width");
        ui.add(
            egui::DragValue::new(&mut self.theme_draft.pane_divider_width)
                .range(1.0..=24.0)
                .suffix(" pt"),
        );
        ui.end_row();
    }
}

struct ThemeHelpers;

impl ThemeHelpers {
    fn hex(color: [u8; 3]) -> String {
        let [red, green, blue] = color;
        format!("#{red:02X}{green:02X}{blue:02X}")
    }

    fn color_category(name: &str) -> i32 {
        if name.starts_with("terminal_") {
            1
        } else if name.starts_with("git_") {
            2
        } else if name.starts_with("status_") {
            3
        } else {
            0
        }
    }

    fn color_label(name: &str) -> String {
        match name {
            "window" => "Workspace and sidebar background".into(),
            "surface" => "Frame background".into(),
            "hover" => "Row highlight".into(),
            "text" => "Primary text".into(),
            "secondary" => "Secondary text".into(),
            _ => {
                let label = name.replace('_', " ");
                let mut chars = label.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().to_string() + chars.as_str(),
                    None => label,
                }
            }
        }
    }

    fn color_field(ui: &mut egui::Ui, value: &mut String, failed: egui::Color32) {
        let mut color = terminator_core::appearance::rgb(value).unwrap_or([0, 0, 0]);
        if ui.color_edit_button_srgb(&mut color).changed() {
            *value = Self::hex(color);
        }
        ui.add_sized(
            [108.0, 28.0],
            appearance::singleline(value).font(egui::TextStyle::Monospace),
        );
        if terminator_core::appearance::rgb(value).is_err() {
            ui.colored_label(failed, "#RRGGBB");
        }
    }
}
