//! New task: the extra worktree form rows and what opens after creation.
use super::worktree_ui::{WorktreeDraft, default_branch, default_dest};
use crate::*;

/// What to open after a task worktree exists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TaskStart {
    pub open_terminal: bool,
    pub agent_command: String,
    pub browser_url: String,
    /// The [`WorktreeDraft::id`] that was submitted.
    pub draft: u64,
    /// The new checkout and the commit it started from, read after
    /// `git worktree add`.
    pub base: Option<(PathBuf, crate::worktree_stats::TaskBase)>,
}

impl WorktreeDraft {
    pub(crate) fn task_start(&self) -> TaskStart {
        TaskStart {
            draft: self.id,
            open_terminal: self.open_terminal,
            agent_command: self.agent_command.trim().to_owned(),
            browser_url: self.browser_url.trim().to_owned(),
            base: None,
        }
    }

    /// A new task name gives a new branch and destination.
    fn set_task(&mut self, source: &Path) {
        self.branch = default_branch(&self.task);
        self.dest = default_dest(source, &self.branch);
    }
}

/// Task name row. Editing it sets the branch and destination.
pub(super) fn task_name(ui: &mut egui::Ui, draft: &mut WorktreeDraft, source: Option<&Path>) {
    settings_controls::settings_row(settings_controls::SettingsRowInput {
        ui,
        label: "Task name",
        description: "Sets the branch name and the destination.",
        add_control: |ui| {
            let field = appearance::singleline(&mut draft.task)
                .hint_text("fix-login")
                .desired_width(260.0);
            if ui.add(field).changed()
                && let Some(path) = source
            {
                draft.set_task(path);
            }
        },
    });
}

/// Agent command (only with a terminal) and Browser URL rows. The combo box
/// fills the command from a preset or an installed agent.
pub(super) fn task_extras(
    ui: &mut egui::Ui,
    draft: &mut WorktreeDraft,
    launchers: &[crate::agent_launch::AgentLauncher],
) {
    if draft.open_terminal {
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Agent command",
            description: "Typed into the terminal. Press Enter there to start it.",
            add_control: |ui| {
                ui.add(
                    appearance::singleline(&mut draft.agent_command)
                        .hint_text("claude")
                        .desired_width(260.0),
                );
                agent_choice(ui, &mut draft.agent_command, launchers);
            },
        });
    }
    settings_controls::settings_row(settings_controls::SettingsRowInput {
        ui,
        label: "Browser URL",
        description: "Optional. Opens in a Browser tab.",
        add_control: |ui| {
            ui.add(
                appearance::singleline(&mut draft.browser_url)
                    .hint_text("http://localhost:3000")
                    .desired_width(260.0),
            );
        },
    });
}

/// Presets and installed agents; a pick replaces the command text.
fn agent_choice(
    ui: &mut egui::Ui,
    command: &mut String,
    launchers: &[crate::agent_launch::AgentLauncher],
) {
    if launchers.is_empty() {
        return;
    }
    let shown = launchers
        .iter()
        .find(|launcher| launcher.command == command.trim())
        .map_or("Choose an agent…", |launcher| launcher.name.as_str());
    egui::ComboBox::from_id_salt("task-agent-choice")
        .selected_text(shown)
        .width(260.0)
        .show_ui(ui, |ui| {
            for launcher in launchers {
                let picked = launcher.command == command.trim();
                if ui
                    .selectable_label(picked, &launcher.name)
                    .on_hover_text(&launcher.command)
                    .clicked()
                {
                    command.clone_from(&launcher.command);
                }
            }
        });
}

impl App {
    /// Refuses a bad Browser URL before Git runs, and remembers the agent
    /// command for the next form.
    pub(super) fn accept_task(&mut self, draft: &WorktreeDraft) -> bool {
        let start = draft.task_start();
        if !start.browser_url.is_empty()
            && let Err(error) = BrowserTarget::from_http_url(&start.browser_url)
        {
            self.error.set(format!("Browser URL: {error:#}"));
            return false;
        }
        if draft.open_terminal {
            self.preferences.task_agent_command = start.agent_command;
        }
        true
    }

    /// Opens what the New task form asked for in the new project. The agent
    /// command is typed without Enter: the user starts the agent.
    pub(crate) fn start_task(&mut self, project: &str, mut start: TaskStart) {
        // The task base outlives this selection: it counts the row's
        // numbers and the Changes since task start page.
        if let Some((path, base)) = start.base.take() {
            self.preferences.task_bases.insert(path, base);
        }
        if self.selected.as_deref() != Some(project) {
            return;
        }
        if !start.browser_url.is_empty()
            && let Err(error) = self.open_browser_url(project, &start.browser_url, None)
        {
            self.error
                .set(format!("Could not open {}: {error:#}", start.browser_url));
        }
        if start.open_terminal {
            let post = PostCreate {
                input: start.agent_command,
                enter: false,
            };
            self.create_workspace_tab_with(None, None, Some(post));
        }
    }
}
