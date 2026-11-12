//! The start panel of a project with no tab: the ways to begin, each with
//! the key that runs exactly that action, and a Setup block while
//! something is missing. GUI only; no daemon request.
use super::super::*;

/// One way to begin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartAction {
    NewTerminal,
    NewAgentSession,
    NewTask,
    SearchEverywhere,
}

impl StartAction {
    fn label(self) -> &'static str {
        match self {
            Self::NewTerminal => "New terminal",
            Self::NewAgentSession => Action::NewAgentSession.label(),
            Self::NewTask => "New task worktree…",
            Self::SearchEverywhere => Action::OpenPalette.label(),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::NewTerminal => "Terminal",
            Self::NewAgentSession => "AgentGeneric",
            Self::NewTask => "GitBranch",
            Self::SearchEverywhere => "Search",
        }
    }

    /// The action whose key the row shows; New task has none.
    fn shortcut(self) -> Option<Action> {
        match self {
            Self::NewTerminal => Some(Action::NewTerminal),
            Self::NewAgentSession => Some(Action::NewAgentSession),
            Self::NewTask => None,
            Self::SearchEverywhere => Some(Action::OpenPalette),
        }
    }
}

/// The rows to show: New task only where worktrees are supported.
pub(crate) fn start_actions(worktrees: bool) -> Vec<StartAction> {
    let mut rows = vec![StartAction::NewTerminal, StartAction::NewAgentSession];
    if worktrees {
        rows.push(StartAction::NewTask);
    }
    rows.push(StartAction::SearchEverywhere);
    rows
}

impl App {
    pub(crate) fn workspace_blank(&mut self, ui: &mut egui::Ui) {
        let mut chosen = None;
        let mut setup_hooks = false;
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.2);
            ui.heading("Your workspace, ready.");
            ui.label("Open a terminal. Run the tools you already use.");
            ui.add_space(12.0);
            ui.allocate_ui(egui::vec2(320.0, 0.0), |ui| {
                ui.set_width(320.0);
                for action in start_actions(self.has_worktrees()) {
                    let keys = action
                        .shortcut()
                        .map(|action| self.shortcut_label(action))
                        .unwrap_or_default();
                    let response = appearance::menu_item(appearance::MenuItemInput {
                        ui,
                        label: action.label(),
                        icon: action.icon(),
                        shortcut: &keys,
                    });
                    appearance::paint_focus(ui, &response);
                    #[cfg(feature = "test-support")]
                    diagnostics::record(
                        ui.ctx(),
                        &format!("start-{}", action.label()),
                        response.rect,
                    );
                    if response.clicked() {
                        chosen = Some(action);
                    }
                }
                setup_hooks = self.start_setup_block(ui);
            });
        });
        if setup_hooks {
            self.open_settings();
            self.request_settings_section(SettingsSection::AgentHooks);
        }
        match chosen {
            Some(StartAction::NewTerminal) => self.create(None),
            Some(StartAction::NewAgentSession) => self.open_agent_picker(),
            Some(StartAction::NewTask) => self.open_worktree_wizard(),
            Some(StartAction::SearchEverywhere) => self.open_command_palette(),
            None => {}
        }
    }

    /// Shows while agent hooks are not installed. Returns true when Set up
    /// hooks was clicked: it opens Settings, it never installs.
    fn start_setup_block(&self, ui: &mut egui::Ui) -> bool {
        let missing =
            !self.hook_status.is_empty() && !self.hook_status.values().any(|installed| *installed);
        if !missing {
            return false;
        }
        ui.add_space(16.0);
        settings_controls::sub_heading(ui, "Setup");
        let mut clicked = false;
        ui.horizontal(|ui| {
            ui.weak("Agent hooks are not installed.");
            clicked = ui.button("Set up hooks").clicked();
        });
        clicked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_task_shows_only_where_worktrees_work() {
        assert!(!start_actions(false).contains(&StartAction::NewTask));
        assert!(start_actions(true).contains(&StartAction::NewTask));
        assert_eq!(start_actions(true).first(), Some(&StartAction::NewTerminal));
    }

    #[test]
    fn only_rows_with_their_own_action_show_a_key() {
        assert_eq!(StartAction::NewTask.shortcut(), None);
        assert_eq!(
            StartAction::NewAgentSession.shortcut(),
            Some(Action::NewAgentSession)
        );
    }
}
