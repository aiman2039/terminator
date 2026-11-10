//! Browser tab rows. GUI preferences: saved with navigation preferences,
//! not the daemon.
use crate::browser::extras;
use crate::*;

impl App {
    pub(super) fn browser_settings_rows(&mut self, ui: &mut egui::Ui) {
        self.browser_search_row(ui);
        self.browser_devtools_row(ui);
    }

    fn browser_search_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible(
            "Browser search",
            "browser search engine address bar url query",
        ) {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Browser search",
            description: "Browser tab address text that is not a web address searches here. %s is the search text. Use https.",
            add_control: |ui| {
                let search = &mut self.preferences.browser.search_url;
                ui.add(appearance::singleline(search).desired_width(280.0));
                if !extras::valid_search_template(search) {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        "Not an https URL with %s; the default search is used.",
                    );
                }
                if search != extras::DEFAULT_SEARCH_URL && ui.small_button("Reset").clicked() {
                    *search = extras::DEFAULT_SEARCH_URL.into();
                }
            },
        });
    }

    fn browser_devtools_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible(
            "Browser developer tools",
            "browser devtools inspector web inspect",
        ) {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Browser developer tools",
            description: "Right click → Inspect Element and the Developer tools menu item in Browser tabs. Open Browser tabs reload when this changes.",
            add_control: |ui| {
                ui.checkbox(
                    &mut self.preferences.browser.devtools,
                    "Enable developer tools",
                );
            },
        });
    }
}
