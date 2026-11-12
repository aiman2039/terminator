//! Quick terminal: a system-wide hotkey (Settings → Keymap, unbound by
//! default) shows the window focused on a dedicated scratch terminal of
//! the chosen project, or hides the window when that terminal already has
//! focus. The scratch terminal is an ordinary daemon-owned shell; closing
//! the GUI leaves it running and the next press finds it again.
mod hotkey;
#[cfg(test)]
mod tests;

#[cfg(test)]
use hotkey::to_hotkey;
pub(crate) use hotkey::{Hotkey, wayland_session};

use crate::*;

/// A scratch terminal that waits longer than this is given up.
const CREATE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(crate) struct QuickTerminal {
    pub(crate) hotkey: Hotkey,
    /// The scratch terminal being created: project and top-level tab id.
    pending: Option<(String, String, Instant)>,
}

/// What one hotkey press does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Toggle {
    /// The scratch terminal has focus in the focused window.
    Hide,
    /// Show the window on this live scratch terminal.
    Focus(String),
    /// Show the window and open a new scratch terminal in this project.
    Create(String),
    /// No project to open it in.
    NoProject,
}

/// Bring the window forward, as the menu-bar status item does.
fn show_window(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

/// The press decision. Pure, so it is testable without a window.
pub(crate) fn decide(
    window_focused: bool,
    active: Option<&str>,
    scratch: Option<&str>,
    project: Option<&str>,
) -> Toggle {
    match (scratch, project) {
        (Some(sid), _) if window_focused && active == Some(sid) => Toggle::Hide,
        (Some(sid), _) => Toggle::Focus(sid.to_owned()),
        (None, Some(project)) => Toggle::Create(project.to_owned()),
        (None, None) => Toggle::NoProject,
    }
}

impl App {
    /// The live scratch terminal, if its session still runs.
    pub(crate) fn quick_terminal_session(&self) -> Option<String> {
        let sid = self.preferences.quick_terminal.session.as_deref()?;
        self.state
            .sessions
            .iter()
            .any(|s| s.id == sid && s.lifecycle.live())
            .then(|| sid.to_owned())
    }

    /// The chosen project when it is open, else the selected one.
    pub(crate) fn quick_terminal_project(&self) -> Option<String> {
        self.preferences
            .quick_terminal
            .project
            .clone()
            .filter(|id| self.state.projects.iter().any(|p| &p.id == id))
            .or_else(|| self.selected_project().map(|p| p.id.clone()))
    }

    /// One press of the quick terminal key.
    pub(crate) fn toggle_quick_terminal(&mut self, ctx: &egui::Context) {
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
        let scratch = self.quick_terminal_session();
        let project = self.quick_terminal_project();
        match decide(
            focused,
            self.active_session.as_deref(),
            scratch.as_deref(),
            project.as_deref(),
        ) {
            Toggle::Hide => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            Toggle::Focus(sid) => {
                show_window(ctx);
                self.go_session(&sid);
            }
            Toggle::Create(project) => {
                show_window(ctx);
                self.create_quick_terminal(project);
            }
            Toggle::NoProject => {
                show_window(ctx);
                self.info = Some("Open a project for the quick terminal.".into());
            }
        }
    }

    fn create_quick_terminal(&mut self, project: String) {
        if self.quick_terminal.pending.is_some() {
            return;
        }
        if self.selected.as_deref() != Some(project.as_str()) {
            self.select_project(project.clone());
        }
        let tab = terminator_core::id();
        let request = Request::Create {
            project: project.clone(),
            cwd: None,
            file: None,
            line: None,
            column: None,
            editor: false,
        };
        if self
            .jobs
            .send(Job::rpc(request, After::Workspace(tab.clone(), Vec::new())))
            .is_ok()
        {
            self.quick_terminal.pending = Some((project, tab, Instant::now()));
        }
    }

    /// Keep the registration in step with Settings, run a press, and
    /// remember a new scratch terminal. Runs in `logic`, also while the
    /// window is hidden.
    pub(crate) fn poll_quick_terminal(&mut self, ctx: &egui::Context) {
        let binding =
            shortcuts::binding(&self.state.settings.keybindings, Action::QuickTerminal.id());
        if !cfg!(test) {
            self.quick_terminal.hotkey.sync(ctx, &binding);
        }
        if self.quick_terminal.hotkey.take_press() {
            self.toggle_quick_terminal(ctx);
        }
        self.adopt_quick_terminal();
    }

    /// The pending scratch tab's terminal, once the daemon placed it.
    pub(crate) fn adopt_quick_terminal(&mut self) {
        let Some((project, tab, since)) = self.quick_terminal.pending.clone() else {
            return;
        };
        let sid = self
            .layouts
            .get(&project)
            .and_then(|w| w.tabs.iter().find(|group| group.id == tab))
            .and_then(|group| {
                group.layout.iter_all_tabs().find_map(|(_, tab)| match tab {
                    Tab::Terminal(sid) => Some(sid.clone()),
                    _ => None,
                })
            });
        if let Some(sid) = sid {
            self.preferences.quick_terminal.session = Some(sid);
            self.quick_terminal.pending = None;
        } else if since.elapsed() > CREATE_TIMEOUT {
            self.quick_terminal.pending = None;
        }
    }

    /// The in-app key: the same toggle, unless the system hotkey already
    /// took the press.
    pub(crate) fn quick_terminal_shortcut(&mut self, ctx: &egui::Context) {
        if !self.quick_terminal.hotkey.active() {
            self.toggle_quick_terminal(ctx);
        }
    }

    /// Settings → Keymap: the project of the quick terminal and why the
    /// hotkey is off.
    pub(crate) fn quick_terminal_settings(&mut self, ui: &mut egui::Ui) {
        let projects: Vec<(String, String)> = self
            .state
            .projects
            .iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();
        let chosen = &mut self.preferences.quick_terminal.project;
        let shown = chosen
            .as_ref()
            .and_then(|id| projects.iter().find(|(pid, _)| pid == id))
            .map_or("Selected project", |(_, name)| name.as_str())
            .to_owned();
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Quick terminal project",
            description: "Where the quick terminal hotkey opens its scratch terminal.",
            add_control: |ui| {
                egui::ComboBox::from_id_salt("quick-terminal-project")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(chosen, None, "Selected project");
                        for (id, name) in projects {
                            ui.selectable_value(chosen, Some(id), name);
                        }
                    });
            },
        });
        let note = if wayland_session() {
            Some("The quick terminal hotkey is not available on Wayland.".to_owned())
        } else {
            self.quick_terminal.hotkey.note.clone()
        };
        if let Some(note) = note {
            settings_controls::description_line(ui, &note);
        }
    }
}
