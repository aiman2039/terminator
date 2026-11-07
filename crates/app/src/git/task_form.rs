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
}

impl WorktreeDraft {
    pub(crate) fn task_start(&self) -> TaskStart {
        TaskStart {
            draft: self.id,
            open_terminal: self.open_terminal,
            agent_command: self.agent_command.trim().to_owned(),
            browser_url: self.browser_url.trim().to_owned(),
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

/// Agent command (only with a terminal) and Browser URL rows.
pub(super) fn task_extras(ui: &mut egui::Ui, draft: &mut WorktreeDraft) {
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
    pub(crate) fn start_task(&mut self, project: &str, start: TaskStart) {
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
