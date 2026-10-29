use crate::*;

/// Preset slots shown by the Theme segmented control; `CUSTOM` is never selectable.
const SAVED: usize = 0;
const CUSTOM: usize = 5;

impl App {
    pub(super) fn appearance_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Appearance");
        self.theme_conflict_banner(ui);
        ui.weak("Preview colors and borders. Apply saves your changes.");
        ui.add_space(8.0);
        self.theme_preset_row(ui);
        self.accent_row(ui);
        self.density_row(ui);
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
                if settings_controls::segmented(
                    ui,
                    &mut choice,
                    &[
                        ("Current saved", SAVED),
                        ("IntelliJ dark", 1),
                        ("High contrast", 2),
                        ("Terminator dark", 3),
                        ("IntelliJ light", 4),
                    ],
                ) {
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
        [
            AppearanceConfig::intellij_dark(),
            AppearanceConfig::high_contrast(),
            AppearanceConfig::terminator_dark(),
            AppearanceConfig::intellij_light(),
        ]
        .iter()
        .position(|preset| self.theme_draft.colors_match(preset))
        .map_or(CUSTOM, |index| index.saturating_add(1))
    }

    /// Replace the draft colors with a preset, keeping the sizing fields.
    fn apply_theme_preset(&mut self, choice: usize) {
        let density = self.theme_draft.density;
        let divider = self.theme_draft.pane_divider_width;
        let border = self.theme_draft.border_width;
        let editor = self.theme_draft.editor;
        self.theme_draft = match choice {
            1 => AppearanceConfig::intellij_dark(),
            2 => AppearanceConfig::high_contrast(),
            3 => AppearanceConfig::terminator_dark(),
            4 => AppearanceConfig::intellij_light(),
            _ => self.theme_committed.clone(),
        };
        self.theme_draft.density = density;
        self.theme_draft.pane_divider_width = divider;
        self.theme_draft.border_width = border;
        self.theme_draft.editor = editor;
    }

    fn accent_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Accent", "accent selection") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Accent",
            description: "Selection color is derived from the accent.",
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
            egui::CollapsingHeader::new(title)
                .id_salt(("appearance-group", group))
                .default_open(false)
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
            self.border_width_fields(ui);
        }
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
            "window" => "Workspace background".into(),
            "surface" => "Sidebar background".into(),
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
