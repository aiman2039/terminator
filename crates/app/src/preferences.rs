use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarTool {
    #[default]
    Explorer,
    Agents,
    Git,
    History,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPreferences {
    pub version: u32,
    pub expanded: HashMap<String, bool>,
    pub history_expanded: HashMap<String, bool>,
    pub tool: SidebarTool,
    pub visible: bool,
    pub left_agents: bool,
    pub width: f32,
    pub all_projects: bool,
    pub show_ignored: bool,
    pub typography_migrated: bool,
    pub attention_migrated: bool,
    pub markdown_modes: HashMap<String, crate::markdown::Mode>,
    pub hidden_projects: HashSet<String>,
}
impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            version: 1,
            expanded: HashMap::new(),
            history_expanded: HashMap::new(),
            tool: SidebarTool::Explorer,
            visible: true,
            left_agents: false,
            width: 285.0,
            all_projects: false,
            show_ignored: false,
            typography_migrated: false,
            attention_migrated: false,
            markdown_modes: HashMap::new(),
            hidden_projects: HashSet::new(),
        }
    }
}
impl UiPreferences {
    pub fn load(data: &Path) -> Result<Self> {
        let bytes = match fs::read(data.join("ui-preferences.json")) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        let mut prefs: Self = serde_json::from_slice(&bytes).context("Invalid UI preferences")?;
        anyhow::ensure!(prefs.version == 1, "Unsupported UI preference version");
        prefs.width = if prefs.width.is_finite() {
            prefs.width.clamp(220.0, 480.0)
        } else {
            285.0
        };
        Ok(prefs)
    }
    pub fn save(&self, data: &Path) -> Result<()> {
        fs::create_dir_all(data)?;
        terminator_core::atomic_write(
            &data.join("ui-preferences.json"),
            &serde_json::to_vec_pretty(self)?,
        )?;
        Ok(())
    }
    pub fn includes_project(&self, owner: &str, selected: Option<&str>) -> bool {
        self.all_projects || selected == Some(owner)
    }
    pub fn toggle(&mut self, tool: SidebarTool) {
        self.visible = self.tool != tool || !self.visible;
        self.tool = tool;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_preferences_request_attention_migration_once() {
        let old: UiPreferences =
            serde_json::from_str(r#"{"version":1,"typography_migrated":true}"#).unwrap();
        assert!(!old.attention_migrated);
        assert!(!old.left_agents);
        assert!(old.typography_migrated);
        assert!(old.markdown_modes.is_empty());
        assert!(old.hidden_projects.is_empty());
    }
    #[test]
    fn restart_preserves_independent_expansion_sidebar_and_migration() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = UiPreferences::default();
        assert!(p.history_expanded.is_empty());
        p.history_expanded.insert("a".into(), true);
        p.expanded.insert("a".into(), false);
        p.expanded.insert("b".into(), true);
        p.toggle(SidebarTool::Agents);
        p.toggle(SidebarTool::Agents);
        p.left_agents = true;
        p.width = 410.0;
        p.all_projects = true;
        p.typography_migrated = true;
        p.attention_migrated = true;
        p.hidden_projects.insert("hidden-project".into());
        p.markdown_modes
            .insert("editor-a".into(), crate::markdown::Mode::Split);
        p.markdown_modes
            .insert("editor-b".into(), crate::markdown::Mode::Preview);
        p.save(dir.path()).unwrap();
        assert_eq!(p, UiPreferences::load(dir.path()).unwrap());
        p.toggle(SidebarTool::Git);
        assert!(p.visible);
    }
    #[test]
    fn unknown_version_is_not_overwritten_and_width_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("ui-preferences.json"), r#"{"version":2}"#).unwrap();
        assert!(UiPreferences::load(dir.path()).is_err());
        fs::write(dir.path().join("ui-preferences.json"), r#"{"width":900}"#).unwrap();
        assert_eq!(UiPreferences::load(dir.path()).unwrap().width, 480.0);
    }
}
