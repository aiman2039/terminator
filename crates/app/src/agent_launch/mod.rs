//! Agent launchers: Settings → Agents presets and the catalog agents found
//! on PATH. A launcher opens a new terminal tab in the current project and
//! types the agent command there without Enter: the user starts the agent.
//! Launchers run only on an explicit pick (menu, palette, New task form).
#[cfg(test)]
mod tests;

use crate::preferences::AgentPreset;
use crate::*;
use terminator_core::agents;

/// A catalog agent found on PATH. `program` is the found file name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InstalledAgent {
    pub(crate) kind: String,
    pub(crate) program: String,
}

/// Probes PATH for every catalog agent. Blocking: run it on a job thread.
pub(crate) fn installed_agents() -> Vec<InstalledAgent> {
    agents::CATALOG
        .iter()
        .filter_map(|info| {
            let path = agents::find_installed(info.kind)?;
            let program = path.file_name()?.to_string_lossy().into_owned();
            Some(InstalledAgent {
                kind: info.kind.to_owned(),
                program,
            })
        })
        .collect()
}

/// One "New terminal with …" entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AgentLauncher {
    /// Recency key: `preset:<name>` or `agent:<kind>`.
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) icon: &'static str,
    /// Shell text typed into the new terminal.
    pub(crate) command: String,
}

impl AgentLauncher {
    pub(crate) fn title(&self) -> String {
        format!("New terminal with {}", self.name)
    }

    /// The command is typed, never sent: `enter` is always false.
    pub(crate) fn post_create(&self) -> PostCreate {
        PostCreate {
            input: self.command.clone(),
            enter: false,
        }
    }
}

/// Presets (named, with a program) then installed catalog agents, the most
/// recently launched first. A repeated key keeps its first entry.
pub(crate) fn launchers(
    presets: &[AgentPreset],
    installed: &[InstalledAgent],
    recent: &[String],
) -> Vec<AgentLauncher> {
    let from_presets = presets.iter().filter_map(|preset| {
        let name = preset.name.trim();
        (!name.is_empty()).then_some(())?;
        Some(AgentLauncher {
            key: format!("preset:{name}"),
            name: name.to_owned(),
            icon: agents::icon_key(&preset.kind),
            command: preset.command()?,
        })
    });
    let from_path = installed.iter().filter_map(|agent| {
        Some(AgentLauncher {
            key: format!("agent:{}", agent.kind),
            name: agents::display_name(&agent.kind).to_owned(),
            icon: agents::icon_key(&agent.kind),
            command: shlex::try_quote(&agent.program).ok()?.into_owned(),
        })
    });
    let mut seen = std::collections::HashSet::new();
    let mut list: Vec<AgentLauncher> = from_presets
        .chain(from_path)
        .filter(|launcher| seen.insert(launcher.key.clone()))
        .collect();
    list.sort_by_key(|launcher| {
        recent
            .iter()
            .position(|key| *key == launcher.key)
            .unwrap_or(usize::MAX)
    });
    list
}

impl App {
    pub(crate) fn agent_launchers(&self) -> Vec<AgentLauncher> {
        launchers(
            &self.preferences.agent_presets,
            &self.installed_agents,
            &self.preferences.agent_launch_recent,
        )
    }

    /// Opens a terminal tab in the current project and types the agent
    /// command without Enter. Only call it for an explicit user pick.
    pub(crate) fn launch_agent(&mut self, launcher: &AgentLauncher) {
        if self.selected_project().is_none() {
            self.info = Some("Select a project, then start the agent.".into());
            return;
        }
        self.preferences.note_agent_launch(&launcher.key);
        self.create_workspace_tab_with(None, None, Some(launcher.post_create()));
    }

    /// New agent session: Search Everywhere listing only the launchers.
    pub(crate) fn open_agent_picker(&mut self) {
        if self.agent_launchers().is_empty() {
            self.info = Some("No agent found on PATH. Add a preset in Settings → Agents.".into());
            return;
        }
        self.open_command_palette();
        self.palette.filters = crate::palette::FILTER_LAUNCH;
    }

    /// "New terminal with …" rows for the workspace new-tab menu.
    pub(crate) fn agent_launcher_menu_items(&mut self, ui: &mut egui::Ui) {
        let list = self.agent_launchers();
        if list.is_empty() {
            return;
        }
        ui.separator();
        ui.weak("Agents");
        for launcher in &list {
            if appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: &launcher.title(),
                icon: launcher.icon,
                shortcut: "",
            })
            .on_hover_text(&launcher.command)
            .clicked()
            {
                self.launch_agent(launcher);
                ui.close();
            }
        }
    }
}
