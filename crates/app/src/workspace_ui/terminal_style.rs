use super::super::*;

const ICONS: &[&str] = &[
    "Terminal",
    "FileCode",
    "Monitor",
    "Database",
    "GitBranch",
    "Folder",
];
pub(crate) const TITLE_COLORS: &[(&str, &str)] = &[
    ("Blue", "#5AA9FF"),
    ("Green", "#66CC99"),
    ("Yellow", "#E8C66A"),
    ("Orange", "#F2A16B"),
    ("Red", "#F08080"),
    ("Purple", "#BB9AF7"),
];

impl App {
    /// The title color the user picked in Terminal appearance, if any. Only
    /// a chosen color tints a terminal's tab line, glyph and title.
    pub(super) fn terminal_chosen_title_color(&self, sid: &str) -> Option<Color32> {
        self.preferences
            .terminal_styles
            .get(sid)
            .and_then(|style| style.color.as_deref())
            .map(appearance::color)
    }

    /// The program's OSC window title for `sid`, shown as a tooltip and a
    /// subtitle. Display only; none once the user named the tab.
    pub(crate) fn terminal_program_title(&self, sid: &str) -> Option<&str> {
        program_title(&self.state, &self.preferences, sid)
    }

    /// A rename path named this terminal's tab: its label now wins over the
    /// program's window title.
    pub(crate) fn mark_label_manual(&mut self, sid: &str) {
        self.preferences
            .terminal_styles
            .entry(sid.to_owned())
            .or_default()
            .label_manual = true;
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
            for &(name, hex) in TITLE_COLORS {
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
                if appearance::menu_item(appearance::MenuItemInput {
                    ui,
                    label: icon,
                    icon,
                    shortcut: "",
                })
                .clicked()
                {
                    self.preferences
                        .terminal_styles
                        .entry(sid.to_owned())
                        .or_default()
                        .icon = Some(icon.into());
                }
            }
            ui.separator();
            if ui.button("Reset appearance").clicked() {
                // Appearance only: a user-given tab name stays manual.
                let manual = self
                    .preferences
                    .terminal_styles
                    .remove(sid)
                    .is_some_and(|style| style.label_manual);
                if manual {
                    self.mark_label_manual(sid);
                }
                ui.close();
            }
        });
    }
}

/// The live session's window title when the daemon sends titles and the
/// user has not named the tab.
pub(crate) fn program_title<'a>(
    state: &'a State,
    preferences: &crate::preferences::UiPreferences,
    sid: &str,
) -> Option<&'a str> {
    let manual = preferences
        .terminal_styles
        .get(sid)
        .is_some_and(|style| style.label_manual);
    if manual {
        return None;
    }
    terminator_core::terminal_title::for_session(state, sid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titled_state() -> State {
        let mut state: State = serde_json::from_value(serde_json::json!({
            "sessions": [{
                "id": "s", "project_id": "p", "label": "zsh", "cwd": "/",
                "kind": "shell", "file": null, "lifecycle": "running", "created": 0,
                "exit_code": null, "rows": 24, "cols": 80, "generation": "g",
                "pid": 1, "truncated": false, "cwd_confirmed": true
            }]
        }))
        .expect("state");
        state
            .capabilities
            .push(terminator_core::terminal_title::CAPABILITY.into());
        state.terminal_titles.insert("s".into(), "htop".into());
        state
    }

    #[test]
    fn manual_label_wins_over_the_program_title() {
        let state = titled_state();
        let mut preferences = crate::preferences::UiPreferences::default();
        assert_eq!(program_title(&state, &preferences, "s"), Some("htop"));
        preferences
            .terminal_styles
            .entry("s".into())
            .or_default()
            .label_manual = true;
        assert_eq!(program_title(&state, &preferences, "s"), None);
    }

    #[test]
    fn program_title_needs_the_daemon_capability() {
        let mut state = titled_state();
        state.capabilities.clear();
        let preferences = crate::preferences::UiPreferences::default();
        assert_eq!(program_title(&state, &preferences, "s"), None);
    }
}
