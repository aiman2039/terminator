//! Settings → Appearance: terminal font family and CJK fallback font.
//! The system list and the font bytes load on a service pool; a saved
//! family that is missing keeps the bundled `JetBrains Mono` chain.
use crate::appearance::fonts::{self, FontList, LoadedFonts, Op, Reply};
use crate::*;

const BUNDLED_MONO: &str = "JetBrains Mono (bundled)";
const NO_CJK: &str = "None (system fallback)";

impl App {
    /// Load the saved font choice when it differs from the installed one.
    pub(crate) fn sync_fonts(&mut self, ctx: &egui::Context) {
        let wanted = self.theme.fonts.clone();
        if wanted == self.fonts.requested {
            return;
        }
        self.fonts.requested = wanted.clone();
        if wanted.terminal().is_none() && wanted.cjk().is_none() {
            self.install_fonts(ctx, &LoadedFonts::default());
            return;
        }
        let _ = self.jobs.send(Job::Fonts(Op::Load(wanted)));
    }

    pub(crate) fn handle_fonts(&mut self, ctx: &egui::Context, reply: Reply) {
        match reply {
            Reply::Listed(list) => self.fonts.list = Some(list),
            // A newer choice replaced this load.
            Reply::Loaded(prefs, _) if prefs != self.fonts.requested => {}
            Reply::Loaded(_, Ok(loaded)) => self.install_fonts(ctx, &loaded),
            Reply::Loaded(_, Err(error)) => {
                self.install_fonts(ctx, &LoadedFonts::default());
                self.fonts.error = Some(error);
            }
        }
    }

    fn install_fonts(&mut self, ctx: &egui::Context, loaded: &LoadedFonts) {
        ctx.set_fonts(fonts::definitions(loaded));
        self.fonts.missing.clone_from(&loaded.missing);
        self.fonts.error = None;
    }

    pub(super) fn font_rows(&mut self, ui: &mut egui::Ui) {
        let show = [
            self.field_visible("Terminal font", "font family monospace nerd powerline"),
            self.field_visible("CJK fallback font", "font chinese japanese korean"),
        ];
        if !show.contains(&true) {
            return;
        }
        if !self.fonts.list_requested {
            self.fonts.list_requested = true;
            let _ = self.jobs.send(Job::Fonts(Op::List));
        }
        let list = match &self.fonts.list {
            Some(Ok(list)) => Some(list.clone()),
            _ => None,
        };
        if show[0] {
            self.terminal_font_row(ui, list.as_ref());
        }
        if show[1] {
            self.cjk_font_row(ui, list.as_ref());
        }
    }

    fn terminal_font_row(&mut self, ui: &mut egui::Ui, list: Option<&FontList>) {
        let failed = appearance::color(&self.theme.status_failed);
        let missing = self.missing_font(self.theme_draft.fonts.terminal(), list, |l| &l.monospace);
        let status = self.font_list_status();
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Terminal font",
            description: "Terminal and editor text. Choose a Nerd Font for prompt glyphs.",
            add_control: |ui| {
                font_combo(FontComboInput {
                    ui,
                    id: "terminal-font",
                    value: &mut self.theme_draft.fonts.terminal_font,
                    default: BUNDLED_MONO,
                    families: list.map(|l| l.monospace.as_slice()),
                });
                font_notes(ui, status.as_deref(), missing.as_deref(), failed);
            },
        });
    }

    fn cjk_font_row(&mut self, ui: &mut egui::Ui, list: Option<&FontList>) {
        let failed = appearance::color(&self.theme.status_failed);
        let missing = self.missing_font(self.theme_draft.fonts.cjk(), list, |l| &l.cjk);
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "CJK fallback font",
            description: "Used for Chinese, Japanese and Korean text after the other fonts.",
            add_control: |ui| {
                font_combo(FontComboInput {
                    ui,
                    id: "cjk-font",
                    value: &mut self.theme_draft.fonts.cjk_font,
                    default: NO_CJK,
                    families: list.map(|l| l.cjk.as_slice()),
                });
                font_notes(ui, None, missing.as_deref(), failed);
            },
        });
    }

    /// The warning for a chosen family that is not installed.
    fn missing_font(
        &self,
        family: Option<&str>,
        list: Option<&FontList>,
        families: impl Fn(&FontList) -> &Vec<String>,
    ) -> Option<String> {
        let family = family?;
        let listed = list.map(|list| families(list).iter().any(|name| name == family));
        let failed = self.fonts.missing.iter().any(|name| name == family);
        (failed || listed == Some(false))
            .then(|| format!("{family} is not installed. The bundled font stays in use."))
    }

    fn font_list_status(&self) -> Option<String> {
        match (&self.fonts.list, &self.fonts.error) {
            (_, Some(error)) => Some(error.clone()),
            (Some(Err(error)), None) => Some(error.clone()),
            (None, None) => Some("Loading system fonts…".into()),
            (Some(Ok(_)), None) => None,
        }
    }
}

struct FontComboInput<'a> {
    ui: &'a mut egui::Ui,
    id: &'static str,
    value: &'a mut Option<String>,
    default: &'static str,
    families: Option<&'a [String]>,
}

fn font_combo(input: FontComboInput<'_>) {
    let FontComboInput {
        ui,
        id,
        value,
        default,
        families,
    } = input;
    let selected = value.clone().unwrap_or_else(|| default.into());
    egui::ComboBox::from_id_salt(id)
        .width(ui.available_width().clamp(180.0, 320.0))
        .selected_text(selected)
        .height(320.0)
        .show_ui(ui, |ui| {
            if ui.selectable_label(value.is_none(), default).clicked() {
                *value = None;
            }
            for family in families.unwrap_or_default() {
                let chosen = value.as_deref() == Some(family.as_str());
                if ui.selectable_label(chosen, family).clicked() {
                    *value = Some(family.clone());
                }
            }
        });
}

fn font_notes(
    ui: &mut egui::Ui,
    status: Option<&str>,
    missing: Option<&str>,
    failed: egui::Color32,
) {
    if let Some(status) = status {
        ui.weak(status);
    }
    if let Some(missing) = missing {
        ui.colored_label(failed, missing);
    }
}
