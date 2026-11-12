//! First-run Welcome page and the What's New page after an update. Both
//! are center pages in `appearance::center_card`. Welcome joins the setup
//! steps: add a project, hooks for the agents found on PATH (Install is
//! an explicit click; the page launches no agent), the keymap, SSH host
//! import and a short inbox tour. What's New shows the bundled
//! CHANGELOG.md section of this version; nothing is fetched.
#[cfg(test)]
mod tests;
mod whats_new;

pub(crate) use whats_new::changelog_section;

use crate::*;

/// Welcome and What's New state.
#[derive(Default)]
pub(crate) struct Onboarding {
    /// The start-up check ran (once, after the state loaded).
    checked: bool,
    notes_cache: egui_commonmark::CommonMarkCache,
    /// This version's changelog section, cut once.
    notes: Option<String>,
}

/// This build's version.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What the start-up check opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartPage {
    Welcome,
    WhatsNew,
    Nothing,
}

/// Welcome on a first run (no project, setup not done), What's New when
/// the saved version differs from this build, else nothing.
pub(crate) fn start_page(prefs: &crate::preferences::UiPreferences, projects: usize) -> StartPage {
    if projects == 0 && !prefs.setup_completed && !prefs.welcome_seen {
        return StartPage::Welcome;
    }
    match prefs.last_seen_version.as_deref() {
        Some(seen) if seen != VERSION => StartPage::WhatsNew,
        _ => StartPage::Nothing,
    }
}

enum WelcomeAction {
    None,
    AddProject,
    Install(String),
    Keymap(bool),
    ImportSsh,
    OpenInbox,
    Close,
}

impl App {
    /// Once per run, after the state loaded and only when preferences
    /// save: open Welcome or What's New, and remember this version.
    pub(crate) fn onboarding_tick(&mut self) {
        // Unwritable preferences would show the page on every start.
        if self.onboarding.checked || !self.state_loaded || !self.preferences_writable {
            return;
        }
        self.onboarding.checked = true;
        match start_page(&self.preferences, self.state.projects.len()) {
            StartPage::Welcome => self.open_welcome(),
            StartPage::WhatsNew => self.show_center(CenterPage::WhatsNew),
            StartPage::Nothing => {}
        }
        self.preferences.last_seen_version = Some(VERSION.to_owned());
    }

    pub(crate) fn open_welcome(&mut self) {
        let _ = self.jobs.send(Job::HookStatus);
        self.show_center(CenterPage::Welcome);
    }

    pub(crate) fn welcome_center(&mut self, ui: &mut egui::Ui) {
        let style = appearance::CenterCardStyle::new(&self.theme);
        let (action, dismissed) = appearance::center_card(ui, style, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("welcome-page")
                .auto_shrink([false, false])
                .show(ui, |ui| self.welcome_card(ui))
                .inner
        });
        match action {
            WelcomeAction::AddProject => {
                let _ = self.request_pick(PickKind::Project);
            }
            WelcomeAction::Install(kind) => {
                let _ = self.jobs.send(Job::Install(kind, false));
                let _ = self.jobs.send(Job::HookStatus);
            }
            WelcomeAction::Keymap(jetbrains) => self.save_keymap_choice(jetbrains),
            WelcomeAction::ImportSsh => self.open_ssh_import(),
            WelcomeAction::OpenInbox => self.open_agents_inbox(),
            WelcomeAction::Close => self.finish_welcome(),
            WelcomeAction::None if dismissed => self.finish_welcome(),
            WelcomeAction::None => {}
        }
    }

    fn finish_welcome(&mut self) {
        self.preferences.welcome_seen = true;
        self.hide_center_overlay();
    }

    /// The keymap pick saves at once: it is an explicit click.
    fn save_keymap_choice(&mut self, jetbrains: bool) {
        let mut config = self.theme_committed.clone();
        config.editor.editor_jetbrains_keys = jetbrains;
        if self.settings_session {
            self.theme_draft.editor.editor_jetbrains_keys = jetbrains;
        }
        let _ = self.jobs.send(Job::SaveAppearance(
            Box::new(config),
            self.theme_source.clone(),
        ));
    }

    fn welcome_card(&self, ui: &mut egui::Ui) -> WelcomeAction {
        let mut action = WelcomeAction::None;
        ui.horizontal(|ui| {
            ui.heading("Welcome to Terminator");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    action = WelcomeAction::Close;
                }
            });
        });
        settings_controls::description_line(
            ui,
            "A few steps to set up. Each one is optional, and Settings has them all later.",
        );
        settings_controls::sub_heading(ui, "1. Add a project");
        if ui.button("Choose project folder…").clicked() {
            action = WelcomeAction::AddProject;
        }
        self.welcome_agents(ui, &mut action);
        self.welcome_keymap(ui, &mut action);
        settings_controls::sub_heading(ui, "4. SSH hosts");
        settings_controls::description_line(
            ui,
            "Import hosts from ~/.ssh/config to open remote projects and terminals.",
        );
        if ui.button("Import SSH hosts…").clicked() {
            action = WelcomeAction::ImportSsh;
        }
        settings_controls::sub_heading(ui, "5. The Agents inbox");
        settings_controls::description_line(
            ui,
            "When an agent waits for you or finishes, its card shows in the Agents inbox \
             (the bell in the sidebar) and the tab gets a dot. Click a card to go to that \
             terminal. Next agent needing attention jumps to the next one.",
        );
        if ui.button("Show the inbox").clicked() {
            action = WelcomeAction::OpenInbox;
        }
        ui.add_space(12.0);
        appearance::footer_row(ui, |ui| {
            if appearance::primary_button(ui, "Done").clicked() {
                action = WelcomeAction::Close;
            }
        });
        action
    }

    /// Agents found on PATH, each with its hook state and Install.
    fn welcome_agents(&self, ui: &mut egui::Ui, action: &mut WelcomeAction) {
        settings_controls::sub_heading(ui, "2. Agent hooks");
        settings_controls::description_line(
            ui,
            "Hooks let Terminator show when an agent waits or is done. Install changes only \
             that agent's settings file and keeps a backup. Agents are never started for you.",
        );
        let found: Vec<&str> = terminator_integrations::AGENTS
            .iter()
            .copied()
            .filter(|kind| {
                self.installed_agents
                    .iter()
                    .any(|agent| agent.kind == *kind)
            })
            .collect();
        if found.is_empty() {
            ui.weak("No agent found on PATH yet. Settings → Agents lists them all.");
            return;
        }
        egui::Grid::new("welcome-agents")
            .num_columns(3)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                for kind in found {
                    ui.label(terminator_core::agents::display_name(kind));
                    let installed = self.hook_status.get(kind).copied();
                    ui.weak(match installed {
                        Some(true) => "Hooks installed",
                        Some(false) => "No hooks",
                        None => "Checking…",
                    });
                    let label = if installed == Some(true) {
                        "Repair"
                    } else {
                        "Install"
                    };
                    if ui.button(label).clicked() {
                        *action = WelcomeAction::Install(kind.to_owned());
                    }
                    ui.end_row();
                }
            });
    }

    fn welcome_keymap(&self, ui: &mut egui::Ui, action: &mut WelcomeAction) {
        settings_controls::sub_heading(ui, "3. Keymap");
        settings_controls::description_line(
            ui,
            "JetBrains keys: double Shift, Find Action, Go to File in every pane.",
        );
        let mut jetbrains = self.theme_committed.editor.editor_jetbrains_keys;
        if settings_controls::segmented(
            ui,
            &mut jetbrains,
            &[("JetBrains keys", true), ("Default keys", false)],
        ) {
            *action = WelcomeAction::Keymap(jetbrains);
        }
    }

    /// The What's New page: this version's CHANGELOG.md section.
    pub(crate) fn whats_new_center(&mut self, ui: &mut egui::Ui) {
        let style = appearance::CenterCardStyle::new(&self.theme);
        let Onboarding {
            notes, notes_cache, ..
        } = &mut self.onboarding;
        let notes = notes.get_or_insert_with(|| changelog_section(whats_new::CHANGELOG, VERSION));
        let cache = notes_cache;
        let (close, dismissed) = appearance::center_card(ui, style, |ui| {
            let mut close = false;
            ui.horizontal(|ui| {
                ui.heading(format!("What's New in {VERSION}"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = appearance::sidebar_action(ui, "X", "Close").clicked();
                });
            });
            ui.add_space(6.0);
            let footer = ui.spacing().interact_size.y + 12.0;
            egui::ScrollArea::vertical()
                .id_salt("whats-new-notes")
                .max_height((ui.available_height() - footer).max(80.0))
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    egui_commonmark::CommonMarkViewer::new().show(ui, cache, notes);
                });
            ui.add_space(6.0);
            appearance::footer_row(ui, |ui| {
                close |= appearance::primary_button(ui, "Close").clicked();
            });
            close
        });
        if close || dismissed {
            self.hide_center_overlay();
        }
    }
}
