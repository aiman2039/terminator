use super::sorting::{HistorySort, ProjectSort};
use super::strip::StripDocks;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

fn default_true() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SidebarTool {
    #[default]
    #[serde(alias = "Explorer")]
    Explorer,
    #[serde(alias = "Agents")]
    Agents,
    #[serde(alias = "Git")]
    Git,
    #[serde(alias = "History")]
    History,
    #[serde(alias = "Info")]
    Info,
}

/// Explorer search behavior: filter the visible tree by name, or search file
/// contents and show a result list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorerSearchMode {
    #[default]
    Names,
    Contents,
}

/// Agents sidebar views. Needs attention preserves the existing inbox;
/// All live is the verified-agent overview; Unread follows read state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentsTab {
    #[default]
    NeedsAttention,
    AllLive,
    Unread,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadioStation {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub icon: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    pub name: String,
    pub tracks: Vec<PathBuf>,
}

/// Right sidebar width that fits six header actions plus the overflow menu:
/// Explorer through Settings stay on the bar, and search does not.
const DEFAULT_SIDEBAR_WIDTH: f32 = 328.0;

/// A named layout preset capturing IDE mode, sidebar visibility, tool selection,
/// sidebar widths, and terminal strip state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutPreset {
    pub ide_mode: bool,
    pub ide_terminal_collapsed: bool,
    /// Sidebars run the full height beside the IDE terminal strip.
    /// Missing presets keep that layout.
    #[serde(default = "default_true")]
    pub ide_sidebars_full_height: bool,
    pub visible: bool,
    pub left_visible: bool,
    pub tool: SidebarTool,
    pub left_agents: bool,
    pub sidebar_width: f32,
    pub left_width: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPreferences {
    pub version: u32,
    pub setup_completed: bool,
    pub expanded: HashMap<String, bool>,
    pub history_expanded: HashMap<String, bool>,
    pub tool: SidebarTool,
    pub visible: bool,
    /// IDE layout preset: fixed explorer/terminal zones instead of the
    /// free-floating dock. Persisted so the user's preference survives restart.
    pub ide_mode: bool,
    /// Bottom IDE terminal strip collapsed (IDE mode only).
    #[serde(default)]
    pub ide_terminal_collapsed: bool,
    /// IDE sidebars run beside the terminal strip instead of stopping above it.
    /// False gives the strip the full window width. Missing prefs stay full height.
    #[serde(default = "default_true")]
    pub ide_sidebars_full_height: bool,
    /// Named layout presets for quick switching.
    #[serde(default)]
    pub named_layouts: HashMap<String, LayoutPreset>,
    /// Currently active named layout, if any.
    #[serde(default)]
    pub active_layout: Option<String>,
    /// Projects column. Missing files stay open; `bool`'s serde default is false.
    #[serde(default = "default_true")]
    pub left_visible: bool,
    pub left_agents: bool,
    pub width: f32,
    /// Selected Agents sidebar view.
    #[serde(default)]
    pub agents_tab: AgentsTab,
    /// All-live text search (session, project, agent, status).
    #[serde(default)]
    pub agents_search: String,
    /// All-live agent filter: "" for all kinds, else a hook `agent_kind`.
    #[serde(default)]
    pub agents_filter: String,
    /// Collapsed All-live project groups (project ids + "unverified").
    #[serde(default)]
    pub agents_collapsed: HashSet<String>,
    pub show_ignored: bool,
    /// Explorer search mode and Options kebab (`Aa`, whole word, regex) plus
    /// the Contents include/exclude globs.
    #[serde(default)]
    pub explorer_search_mode: ExplorerSearchMode,
    #[serde(default)]
    pub explorer_include: String,
    #[serde(default)]
    pub explorer_exclude: String,
    #[serde(default)]
    pub explorer_match_case: bool,
    #[serde(default)]
    pub explorer_whole_word: bool,
    #[serde(default)]
    pub explorer_regex: bool,
    /// Git sidebar "View as list": flat file list instead of grouped sections.
    #[serde(default)]
    pub git_view_list: bool,
    /// Info sidebar sections and the host-resource block. Missing prefs stay open.
    #[serde(default = "default_true")]
    pub info_process_open: bool,
    #[serde(default = "default_true")]
    pub info_resources_open: bool,
    #[serde(default = "default_true")]
    pub info_show_system: bool,
    pub typography_migrated: bool,
    pub attention_migrated: bool,
    pub markdown_modes: HashMap<String, crate::markdown::Mode>,
    pub hidden_projects: HashSet<String>,
    pub project_sort: ProjectSort,
    #[serde(default)]
    pub project_filter: String,
    pub project_activity: HashMap<String, u64>,
    pub history_sort: HistorySort,
    #[serde(default)]
    pub history_filter: String,
    pub playlists: Vec<Playlist>,
    pub selected_playlist: String,
    #[serde(default, skip_serializing)]
    pub player_playlists: HashMap<String, Vec<PathBuf>>,
    pub player_index: HashMap<String, usize>,
    pub player_volume: f32,
    pub player_shuffle: bool,
    pub player_repeat: bool,
    pub player_samples_seeded: bool,
    pub player_chrome_collapsed: bool,
    pub player_radio_mode: bool,
    pub radio_stations: Vec<RadioStation>,
    /// IDE strip dock layouts per project (shell terminals). GUI-local view
    /// state; validated on load, invalid docks dropped. Kept last so the
    /// derived `PartialEq` (which gates saves) short-circuits on cheap
    /// fields first. The dock compare itself is structural: see [`StripDocks`].
    #[serde(default)]
    pub ide_strip_docks: StripDocks,
}
impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            version: 2,
            setup_completed: false,
            expanded: HashMap::new(),
            history_expanded: HashMap::new(),
            tool: SidebarTool::Explorer,
            visible: true,
            ide_mode: false,
            ide_terminal_collapsed: false,
            ide_sidebars_full_height: true,
            named_layouts: HashMap::new(),
            active_layout: None,
            left_visible: true,
            left_agents: false,
            width: DEFAULT_SIDEBAR_WIDTH,
            agents_tab: AgentsTab::NeedsAttention,
            agents_search: String::new(),
            agents_filter: String::new(),
            agents_collapsed: HashSet::new(),
            show_ignored: false,
            explorer_search_mode: ExplorerSearchMode::Names,
            explorer_include: String::new(),
            explorer_exclude: String::new(),
            explorer_match_case: false,
            explorer_whole_word: false,
            explorer_regex: false,
            git_view_list: false,
            info_process_open: true,
            info_resources_open: true,
            info_show_system: true,
            typography_migrated: false,
            attention_migrated: false,
            markdown_modes: HashMap::new(),
            hidden_projects: HashSet::new(),
            project_sort: ProjectSort::NameAsc,
            project_filter: String::new(),
            project_activity: HashMap::new(),
            history_sort: HistorySort::LatestActivity,
            history_filter: String::new(),
            playlists: Vec::new(),
            selected_playlist: String::new(),
            player_playlists: HashMap::new(),
            player_index: HashMap::new(),
            player_volume: 0.8,
            player_shuffle: false,
            player_repeat: false,
            player_samples_seeded: false,
            player_chrome_collapsed: false,
            player_radio_mode: false,
            radio_stations: Vec::new(),
            ide_strip_docks: StripDocks::default(),
        }
    }
}
impl UiPreferences {
    pub fn needs_setup(&self, state_loaded: bool, project_count: usize) -> bool {
        state_loaded && project_count == 0 && !self.setup_completed
    }
    pub fn load(data: &Path) -> Result<Self> {
        let bytes = match fs::read(data.join("ui-preferences.json")) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        let mut value: serde_json::Value =
            serde_json::from_slice(&bytes).context("Invalid UI preferences")?;
        if let Some(docks) = value.get_mut("ide_strip_docks") {
            // Fresh dock rects are infinite (`Rect::NOTHING`) and serialize
            // as null; restore them like saved main-dock layouts do.
            *docks = terminator_core::sanitize_layout(docks.take());
        }
        let mut prefs: Self = serde_json::from_value(value).context("Invalid UI preferences")?;
        anyhow::ensure!(
            prefs.version == 1 || prefs.version == 2,
            "Unsupported UI preference version"
        );
        // Version 1 never persisted ide_mode; ignore any stray value once, then
        // migrate so the rule does not re-fire on every launch.
        if prefs.version == 1 {
            prefs.ide_mode = false;
            prefs.version = 2;
        }
        prefs.width = if prefs.width.is_finite() {
            prefs.width.clamp(220.0, 480.0)
        } else {
            DEFAULT_SIDEBAR_WIDTH
        };
        prefs.player_volume = if prefs.player_volume.is_finite() {
            prefs.player_volume.clamp(0.0, 1.0)
        } else {
            0.8
        };
        prefs.migrate_playlists();
        prefs
            .ide_strip_docks
            .0
            .retain(|_, dock| crate::workspace::validate_layout(dock).is_ok());
        Ok(prefs)
    }

    fn migrate_playlists(&mut self) {
        if self.playlists.is_empty() && !self.player_playlists.is_empty() {
            let mut tracks = Vec::new();
            let mut owners: Vec<_> = self.player_playlists.keys().cloned().collect();
            owners.sort();
            for owner in owners {
                let Some(list) = self.player_playlists.get(&owner) else {
                    continue;
                };
                for path in list {
                    if !tracks.contains(path) {
                        tracks.push(path.clone());
                    }
                }
            }
            if !tracks.is_empty() {
                self.playlists.push(Playlist {
                    name: "Default".into(),
                    tracks,
                });
                if self.selected_playlist.is_empty() {
                    self.selected_playlist = "Default".into();
                }
            }
        }
        self.player_playlists.clear();
        if self
            .playlists
            .iter()
            .all(|playlist| playlist.name != self.selected_playlist)
        {
            self.selected_playlist = self
                .playlists
                .first()
                .map(|playlist| playlist.name.clone())
                .unwrap_or_default();
        }
    }
    /// Save the current UI state as a named layout preset.
    pub fn save_current_layout(&mut self, name: &str) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let preset = LayoutPreset {
            ide_mode: self.ide_mode,
            ide_terminal_collapsed: self.ide_terminal_collapsed,
            ide_sidebars_full_height: self.ide_sidebars_full_height,
            visible: self.visible,
            left_visible: self.left_visible,
            tool: self.tool,
            left_agents: self.left_agents,
            sidebar_width: self.width,
            left_width: 225.0,
        };
        self.named_layouts.insert(name.clone(), preset);
        self.active_layout = Some(name);
    }

    /// Apply a named layout preset to the current preferences.
    pub fn apply_layout(&mut self, name: &str) -> Option<&LayoutPreset> {
        let preset = self.named_layouts.get(name)?;
        self.ide_mode = preset.ide_mode;
        self.ide_terminal_collapsed = preset.ide_terminal_collapsed;
        self.ide_sidebars_full_height = preset.ide_sidebars_full_height;
        self.visible = preset.visible;
        self.left_visible = preset.left_visible;
        self.tool = preset.tool;
        self.left_agents = preset.left_agents;
        self.width = preset.sidebar_width;
        self.active_layout = Some(name.to_string());
        Some(preset)
    }

    /// Delete a named layout preset.
    #[allow(dead_code)]
    pub fn delete_layout(&mut self, name: &str) {
        self.named_layouts.remove(name);
        if self.active_layout.as_deref() == Some(name) {
            self.active_layout = None;
        }
    }

    pub fn save(&self, data: &Path) -> Result<()> {
        fs::create_dir_all(data)?;
        terminator_core::atomic_write(
            &data.join("ui-preferences.json"),
            &serde_json::to_vec_pretty(self)?,
        )?;
        Ok(())
    }
    pub fn toggle(&mut self, tool: SidebarTool) {
        self.visible = self.tool != tool || !self.visible;
        self.tool = tool;
    }

    pub fn ensure_default_playlist(&mut self) {
        if self.playlists.is_empty() {
            self.playlists.push(Playlist {
                name: "Default".into(),
                tracks: Vec::new(),
            });
        }
        if self
            .playlists
            .iter()
            .all(|playlist| playlist.name != self.selected_playlist)
            && let Some(playlist) = self.playlists.first()
        {
            self.selected_playlist.clone_from(&playlist.name);
        }
    }

    pub fn selected_tracks(&self) -> &[PathBuf] {
        self.playlists
            .iter()
            .find(|playlist| playlist.name == self.selected_playlist)
            .map(|playlist| playlist.tracks.as_slice())
            .unwrap_or(&[])
    }

    pub fn selected_tracks_mut(&mut self) -> Option<&mut Vec<PathBuf>> {
        let name = self.selected_playlist.clone();
        self.playlists
            .iter_mut()
            .find(|playlist| playlist.name == name)
            .map(|playlist| &mut playlist.tracks)
    }

    pub fn create_playlist(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || self.playlists.iter().any(|playlist| playlist.name == name) {
            return false;
        }
        self.playlists.push(Playlist {
            name: name.into(),
            tracks: Vec::new(),
        });
        self.selected_playlist = name.into();
        true
    }

    pub fn rename_selected_playlist(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty()
            || self
                .playlists
                .iter()
                .any(|playlist| playlist.name == name && playlist.name != self.selected_playlist)
        {
            return false;
        }
        let Some(playlist) = self
            .playlists
            .iter_mut()
            .find(|playlist| playlist.name == self.selected_playlist)
        else {
            return false;
        };
        if let Some(index) = self.player_index.remove(&playlist.name) {
            self.player_index.insert(name.into(), index);
        }
        playlist.name = name.into();
        self.selected_playlist = name.into();
        true
    }

    pub fn delete_selected_playlist(&mut self) {
        let name = self.selected_playlist.clone();
        self.playlists.retain(|playlist| playlist.name != name);
        self.player_index.remove(&name);
        self.selected_playlist = self
            .playlists
            .first()
            .map(|playlist| playlist.name.clone())
            .unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_project_playlists_merge_into_default() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("ui-preferences.json"),
            r#"{"version":1,"player_playlists":{"a":["/a/one.mp3"],"b":["/b/two.mp3","/a/one.mp3"]}}"#,
        )
        .unwrap();
        let prefs = UiPreferences::load(dir.path()).unwrap();
        assert_eq!(prefs.selected_playlist, "Default");
        assert_eq!(
            prefs.selected_tracks(),
            [PathBuf::from("/a/one.mp3"), PathBuf::from("/b/two.mp3")].as_slice()
        );
        assert!(prefs.player_playlists.is_empty());
        prefs.save(dir.path()).unwrap();
        let raw = fs::read_to_string(dir.path().join("ui-preferences.json")).unwrap();
        assert!(!raw.contains("player_playlists"));
        assert!(raw.contains("Default"));
    }

    #[test]
    fn ide_mode_is_persisted_and_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = UiPreferences::load(dir.path()).unwrap();
        assert!(!prefs.ide_mode);
        assert!(!prefs.ide_terminal_collapsed);
        let mut prefs = prefs;
        prefs.ide_mode = true;
        prefs.ide_terminal_collapsed = true;
        prefs.save(dir.path()).unwrap();
        let raw = fs::read_to_string(dir.path().join("ui-preferences.json")).unwrap();
        assert!(raw.contains("ide_mode"));
        assert!(UiPreferences::load(dir.path()).unwrap().ide_mode);
        assert!(
            UiPreferences::load(dir.path())
                .unwrap()
                .ide_terminal_collapsed
        );
        // Stale version-1 files written before version-2 still open with IDE mode off,
        // and the load migrates them so the reset does not re-fire.
        fs::write(
            dir.path().join("ui-preferences.json"),
            r#"{"version":1,"ide_mode":true}"#,
        )
        .unwrap();
        let migrated = UiPreferences::load(dir.path()).unwrap();
        assert!(!migrated.ide_mode);
        assert_eq!(migrated.version, 2);
        migrated.save(dir.path()).unwrap();
        let raw = fs::read_to_string(dir.path().join("ui-preferences.json")).unwrap();
        assert!(raw.contains(r#""version": 2"#));
        let mut reopened = UiPreferences::load(dir.path()).unwrap();
        assert_eq!(reopened.version, 2);
        reopened.ide_mode = true;
        reopened.save(dir.path()).unwrap();
        assert!(UiPreferences::load(dir.path()).unwrap().ide_mode);
    }

    #[test]
    fn ide_sidebar_height_defaults_full_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = UiPreferences::load(dir.path()).unwrap();
        assert!(prefs.ide_sidebars_full_height);
        let mut prefs = prefs;
        prefs.ide_mode = true;
        prefs.ide_sidebars_full_height = false;
        prefs.save_current_layout("wide");
        prefs.ide_sidebars_full_height = true;
        prefs.apply_layout("wide").unwrap();
        assert!(!prefs.ide_sidebars_full_height);
        prefs.save(dir.path()).unwrap();
        assert!(
            !UiPreferences::load(dir.path())
                .unwrap()
                .ide_sidebars_full_height
        );
        let mut value: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.path().join("ui-preferences.json")).unwrap(),
        )
        .unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("ide_sidebars_full_height");
        value["named_layouts"]["wide"]
            .as_object_mut()
            .unwrap()
            .remove("ide_sidebars_full_height");
        fs::write(
            dir.path().join("ui-preferences.json"),
            serde_json::to_string(&value).unwrap(),
        )
        .unwrap();
        let loaded = UiPreferences::load(dir.path()).unwrap();
        assert!(loaded.ide_sidebars_full_height);
        assert!(
            loaded
                .named_layouts
                .get("wide")
                .unwrap()
                .ide_sidebars_full_height
        );
    }

    #[test]
    fn playlists_can_be_created_renamed_and_deleted() {
        let mut prefs = UiPreferences::default();
        prefs.ensure_default_playlist();
        assert!(prefs.create_playlist("Focus"));
        assert!(!prefs.create_playlist("Focus"));
        assert_eq!(prefs.selected_playlist, "Focus");
        assert!(prefs.rename_selected_playlist("Deep"));
        assert_eq!(prefs.selected_playlist, "Deep");
        prefs.delete_selected_playlist();
        assert_eq!(prefs.selected_playlist, "Default");
        prefs.delete_selected_playlist();
        assert!(prefs.playlists.is_empty());
        prefs.ensure_default_playlist();
        assert_eq!(prefs.selected_playlist, "Default");
    }

    #[test]
    fn old_preferences_request_attention_migration_once() {
        let old: UiPreferences =
            serde_json::from_str(r#"{"version":1,"typography_migrated":true}"#).unwrap();
        assert!(!old.attention_migrated);
        assert!(old.left_visible);
        assert!(!old.left_agents);
        assert!(old.typography_migrated);
        assert!(old.markdown_modes.is_empty());
        assert!(old.hidden_projects.is_empty());
        assert_eq!(old.project_sort, ProjectSort::NameAsc);
        assert!(old.project_activity.is_empty());
        assert_eq!(old.history_sort, HistorySort::LatestActivity);
        assert!(old.history_filter.is_empty());
        assert!(old.info_process_open);
        assert!(old.info_resources_open);
        assert!(old.info_show_system);
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
        p.setup_completed = true;
        p.left_agents = true;
        p.width = 410.0;
        p.typography_migrated = true;
        p.attention_migrated = true;
        p.hidden_projects.insert("hidden-project".into());
        p.project_sort = ProjectSort::LatestActivity;
        p.project_filter = "term".into();
        p.project_activity.insert("a".into(), 42);
        p.history_sort = HistorySort::NameDesc;
        p.history_filter = "term".into();
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
        fs::write(dir.path().join("ui-preferences.json"), r#"{"version":3}"#).unwrap();
        assert!(UiPreferences::load(dir.path()).is_err());
        fs::write(dir.path().join("ui-preferences.json"), r#"{"width":900}"#).unwrap();
        assert_eq!(UiPreferences::load(dir.path()).unwrap().width, 480.0);
    }

    #[test]
    fn legacy_pascal_case_tool_loads() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("ui-preferences.json"),
            r#"{"version":2,"tool":"Git","visible":true}"#,
        )
        .unwrap();
        let prefs = UiPreferences::load(dir.path()).unwrap();
        assert_eq!(prefs.tool, SidebarTool::Git);
        assert!(prefs.visible);
    }

    #[test]
    fn saved_tool_uses_snake_case() {
        let dir = tempfile::tempdir().unwrap();
        let prefs = UiPreferences {
            tool: SidebarTool::Git,
            ..UiPreferences::default()
        };
        prefs.save(dir.path()).unwrap();
        let raw: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("ui-preferences.json")).unwrap())
                .unwrap();
        assert_eq!(raw.get("tool"), Some(&serde_json::json!("git")));
    }

    #[test]
    fn setup_waits_for_inventory_and_counts_hidden_projects() {
        let mut preferences = UiPreferences::default();
        assert!(!preferences.needs_setup(false, 0));
        assert!(preferences.needs_setup(true, 0));
        preferences.hidden_projects.insert("hidden".into());
        assert!(!preferences.needs_setup(true, 1));
        preferences.setup_completed = true;
        assert!(!preferences.needs_setup(true, 0));
    }
}
