use super::super::*;

const ICONS: &[&str] = &[
    "Terminal",
    "FileCode",
    "Monitor",
    "Database",
    "GitBranch",
    "Folder",
];
const COLORS: &[(&str, &str)] = &[
    ("Blue", "#5AA9FF"),
    ("Green", "#66CC99"),
    ("Yellow", "#E8C66A"),
    ("Orange", "#F2A16B"),
    ("Red", "#F08080"),
    ("Purple", "#BB9AF7"),
];

impl App {
    pub(super) fn terminal_title_color(&self, sid: &str) -> Color32 {
        self.preferences
            .terminal_styles
            .get(sid)
            .and_then(|style| style.color.as_deref())
            .map(appearance::color)
            .unwrap_or_else(|| appearance::color(&self.theme.accent))
    }

    pub(super) fn terminal_title_icon(&self, sid: &str, fallback: &'static str) -> &'static str {
        self.preferences
            .terminal_styles
            .get(sid)
            .and_then(|style| style.icon.as_deref())
            .and_then(|icon| ICONS.iter().copied().find(|candidate| *candidate == icon))
            .unwrap_or(fallback)
    }

    pub(super) fn terminal_style_menu(&mut self, ui: &mut egui::Ui, sid: &str) {
        ui.menu_button("Terminal appearance", |ui| {
            ui.label("Title color");
            for &(name, hex) in COLORS {
                if ui
                    .button(egui::RichText::new(name).color(appearance::color(hex)))
                    .clicked()
                {
                    self.preferences
                        .terminal_styles
                        .entry(sid.to_owned())
                        .or_default()
                        .color = Some(hex.into());
                }
            }
            ui.separator();
            ui.label("Terminal icon");
            for &icon in ICONS {
                if appearance::menu_item(ui, icon, icon, "").clicked() {
                    self.preferences
                        .terminal_styles
                        .entry(sid.to_owned())
                        .or_default()
                        .icon = Some(icon.into());
                }
            }
            ui.separator();
            if ui.button("Reset appearance").clicked() {
                self.preferences.terminal_styles.remove(sid);
                ui.close();
            }
        });
    }
}
