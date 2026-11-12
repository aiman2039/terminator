//! Settings → Appearance: Follow OS light/dark and terminal theme import.
//! An import only fills the draft; Apply saves it to `config.toml`
//! (comments and other keys stay), so no saved theme changes unasked.
use super::super::state::BrowseTarget;
use crate::*;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use terminator_core::appearance::{self as theme_core, TerminalColors};

/// Search words for the two rows (section haystack).
pub(in crate::settings_ui) const THEME_FOLLOW_KEYWORDS: &str = " follow os system light dark \
     automatic appearance import terminal theme ghostty kitty iterm2 itermcolors file";

/// A theme file read in the background, then its result line.
#[derive(Default)]
pub(crate) struct ThemeImport {
    pending: Option<(PathBuf, Receiver<Result<TerminalColors, String>>)>,
    note: Option<(bool, String)>,
}

/// Read and parse one theme file. Blocking: call it on a worker thread.
fn read_theme(path: &std::path::Path) -> Result<TerminalColors, String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    TerminalColors::parse(&text).map_err(|error| error.to_string())
}

impl App {
    /// The OS light/dark row: a checkbox and one preset for each mode.
    pub(super) fn follow_os_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible(
            "Follow OS appearance",
            "follow os system light dark automatic",
        ) {
            return;
        }
        let follow = &mut self.theme_draft.follow;
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Follow OS appearance",
            description: "Use one preset while the OS is light and one while it is dark.",
            add_control: |ui| {
                ui.checkbox(&mut follow.follow_os_theme, "Follow OS light/dark");
                if !follow.follow_os_theme {
                    return;
                }
                for (dark, label) in [(false, "Light"), (true, "Dark")] {
                    let mut chosen = follow.preset_name(dark).to_owned();
                    egui::ComboBox::from_id_salt(("follow-os-preset", dark))
                        .selected_text(format!("{label}: {chosen}"))
                        .show_ui(ui, |ui| {
                            for (name, _) in theme_core::PRESETS {
                                ui.selectable_value(&mut chosen, name.to_owned(), name);
                            }
                        });
                    if chosen != follow.preset_name(dark) {
                        let slot = if dark {
                            &mut follow.dark_theme
                        } else {
                            &mut follow.light_theme
                        };
                        *slot = Some(chosen);
                    }
                }
            },
        });
    }

    /// Import a Ghostty, kitty or iTerm2 theme file into the draft.
    pub(super) fn theme_import_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible(
            "Import terminal theme",
            "import ghostty kitty iterm2 itermcolors file",
        ) {
            return;
        }
        self.poll_theme_import();
        let mut pick = false;
        let busy = self.theme_import.pending.is_some();
        let note = self.theme_import.note.clone();
        let failed = appearance::color(&self.theme.status_failed);
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Import terminal theme",
            description: "Ghostty or kitty theme files and iTerm2 .itermcolors. Fills the \
                          colors below; Apply saves them.",
            add_control: |ui| {
                pick = ui
                    .add_enabled(!busy, egui::Button::new("Import…"))
                    .clicked();
                match note {
                    Some((true, text)) => {
                        ui.weak(text);
                    }
                    Some((false, text)) => {
                        ui.colored_label(failed, text);
                    }
                    None => {}
                }
            },
        });
        if pick {
            self.request_pick(PickKind::Browse(BrowseTarget::ThemeFile));
        }
        if busy {
            ui.ctx().request_repaint();
        }
    }

    /// Read a picked theme file on a worker thread.
    pub(crate) fn start_theme_import(&mut self, path: PathBuf) {
        let (sender, receiver) = channel();
        let file = path.clone();
        let spawned = std::thread::Builder::new()
            .name("theme-import".into())
            .spawn(move || {
                let _ = sender.send(read_theme(&file));
            });
        self.theme_import.note = None;
        match spawned {
            Ok(_) => self.theme_import.pending = Some((path, receiver)),
            Err(error) => self.theme_import.note = Some((false, error.to_string())),
        }
    }

    /// Take a finished import: its colors go into the draft only.
    fn poll_theme_import(&mut self) {
        let Some((path, receiver)) = &self.theme_import.pending else {
            return;
        };
        let Ok(result) = receiver.try_recv() else {
            return;
        };
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.theme_import.pending = None;
        self.theme_import.note = Some(match result {
            Ok(colors) => {
                self.theme_draft = self.theme_draft.with_terminal_colors(&colors);
                // Imported colors would not show while the OS picks a preset.
                self.theme_draft.follow.follow_os_theme = false;
                (true, format!("Imported {name}. Apply saves it."))
            }
            Err(error) => (false, format!("{name}: {error}")),
        });
    }
}
