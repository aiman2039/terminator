mod agent_presets;
mod agents;
mod defaults;
mod expanded;
mod explorer;
mod left_content;
mod playlists;
mod styles;
pub use agent_presets::AgentPreset;
pub use agents::AgentStateFilter;
pub use explorer::{ExplorerSearchMode, ExplorerView};
pub use left_content::LeftContent;
pub use styles::{ProjectStyle, TerminalStyle};
#[cfg(test)]
mod tests;

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
pub enum VncPasteShortcut {
    #[default]
    ShiftInsert,
    ControlV,
    ControlShiftV,
    CommandV,
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
    #[serde(alias = "Problems")]
    Problems,
    #[serde(alias = "Structure")]
    Structure,
    #[serde(alias = "Vnc")]
    Vnc,
    Hosts,
}

/// A tool this build does not know (written by a newer build) falls back to
/// the default instead of discarding every other preference.
fn lenient_tool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<SidebarTool, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Like [`lenient_tool`]: an unknown side tool keeps the native views.
fn lenient_side_tool<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<SidebarTool>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    Ok(serde_json::from_value(value).unwrap_or(None))
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

/// Right sidebar width. The window header shows all its actions at this
/// width, without the overflow menu.
const DEFAULT_SIDEBAR_WIDTH: f32 = 328.0;

/// Tools pinned on the right stripe by default: the tools the window
/// header used to hold.
const DEFAULT_RIGHT_PINS: [&str; 4] = ["Explorer", "Git", "History", "Info"];

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
    #[serde(deserialize_with = "lenient_tool")]
    pub tool: SidebarTool,
    /// Saved form of [`LeftContent`]; read it through `left_content`.
    left_agents: bool,
    /// Tool hosted by the left sidebar instead of Projects/Agents.
    #[serde(default, deserialize_with = "lenient_side_tool")]
    left_tool: Option<SidebarTool>,
    /// Projects list hosted by the right sidebar.
    #[serde(default)]
    pub right_projects: bool,
    pub sidebar_width: f32,
    pub left_width: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPreferences {
    pub version: u32,
    pub setup_completed: bool,
    /// The first-run Welcome page was closed.
    pub welcome_seen: bool,
    /// Version of the last run; another version opens What's New.
    pub last_seen_version: Option<String>,
    pub expanded: HashMap<String, bool>,
    pub history_expanded: HashMap<String, bool>,
    /// Terminal title colors and icons, keyed by persistent session ID.
    pub terminal_styles: HashMap<String, TerminalStyle>,
    /// Project display name, color, and icon, keyed by project ID.
    pub project_styles: HashMap<String, ProjectStyle>,
    pub vnc_paste_shortcut: VncPasteShortcut,
    /// Shell terminal type-ahead prediction (#83).
    #[serde(default)]
    pub type_ahead: super::TypeAhead,
    /// A shell command that ran at least this long and ends in a pane
    /// without focus raises a terminal notice. 0 is off.
    #[serde(default)]
    pub long_command_notice_secs: u32,
    #[serde(deserialize_with = "lenient_tool")]
    pub tool: SidebarTool,
    pub visible: bool,
    /// IDE layout: tool stripes, editor center, terminal strip. Default on
    /// for fresh preferences; a saved value (either way) is never overridden.
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
    /// Saved form of [`LeftContent`] (with `left_tool`). Code reads and
    /// writes it only through `left_content` and `set_left_content`.
    left_agents: bool,
    /// Tool hosted by the left sidebar instead of its native Projects/Agents
    /// views. `None` keeps the native views; stripes set this per side.
    #[serde(default, deserialize_with = "lenient_side_tool")]
    left_tool: Option<SidebarTool>,
    /// Projects list hosted by the right sidebar instead of `tool`.
    #[serde(default)]
    pub right_projects: bool,
    /// Pinned tool captions on each edge. Unknown entries are ignored.
    pub left_tool_pins: Vec<String>,
    pub right_tool_pins: Vec<String>,
    /// The one-time default pins for an empty right stripe ran. Missing in
    /// files saved before it, so those get the pins once.
    #[serde(default)]
    pub right_pins_migrated: bool,
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
    /// All-live agent state filter (Working, Done).
    #[serde(default)]
    pub agents_state: AgentStateFilter,
    /// Collapsed All-live project groups (project ids + "unverified").
    #[serde(default)]
    pub agents_collapsed: HashSet<String>,
    /// Sessions pinned to the top of the Agents inbox and live list.
    #[serde(default)]
    pub pinned_agent_sessions: HashSet<String>,
    pub show_ignored: bool,
    /// Explorer follows the focused editor file (`IntelliJ` "Always
    /// select opened file").
    #[serde(default)]
    pub explorer_autoreveal: bool,
    /// Explorer shows every folder on its own row instead of compacting
    /// single-child chains into one `a/b/c` row (the default).
    #[serde(default)]
    pub explorer_flat_folders: bool,
    #[serde(default)]
    pub explorer_view: ExplorerView,
    /// Explorer bookmarks (`IntelliJ` Bookmarks): files and folders to
    /// jump to from the Explorer "…" menu.
    #[serde(default)]
    pub explorer_bookmarks: Vec<PathBuf>,
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
    /// Native diff compare options and highlight mode (gear menu).
    #[serde(default)]
    pub diff_options: crate::diff::DiffOptions,
    /// Native diff changed-files panel is open.
    #[serde(default)]
    pub diff_file_tree: bool,
    /// Recent commit messages, newest first, for the commit box history.
    #[serde(default)]
    pub commit_messages: Vec<String>,
    /// Info sidebar sections and the host-resource block. Missing prefs stay open.
    #[serde(default = "default_true")]
    pub info_process_open: bool,
    #[serde(default = "default_true")]
    pub info_resources_open: bool,
    #[serde(default = "default_true")]
    pub info_show_system: bool,
    pub typography_migrated: bool,
    pub attention_migrated: bool,
    pub keymap_migrated: bool,
    /// Rename terminal's move off Cmd+Shift+R ran (or needed no move).
    #[serde(default)]
    pub keymap_rename_migrated: bool,
    /// The one-time move of main-pane shells to the IDE strip ran.
    #[serde(default)]
    pub ide_strip_migrated: bool,
    pub markdown_modes: HashMap<String, crate::markdown::Mode>,
    pub hidden_projects: HashSet<String>,
    pub project_sort: ProjectSort,
    /// Project ids in the order the user dragged them (Manual sort).
    #[serde(default)]
    pub project_order: Vec<String>,
    #[serde(default)]
    pub project_filter: String,
    pub project_activity: HashMap<String, u64>,
    pub history_sort: HistorySort,
    #[serde(default)]
    pub history_filter: String,
    /// Last agent command typed in the New task form.
    #[serde(default)]
    pub task_agent_command: String,
    /// The commit each New task worktree started from, by worktree path.
    /// Functional state kept here because the daemon has no place for it.
    #[serde(default)]
    pub task_bases: HashMap<PathBuf, crate::worktree_stats::TaskBase>,
    /// New task setup command per source project path, typed (never run)
    /// before the agent command in the new terminal.
    #[serde(default)]
    pub task_setup_commands: HashMap<PathBuf, String>,
    /// Settings → Agents presets, shown with the installed agents.
    pub agent_presets: Vec<AgentPreset>,
    /// Agent launcher keys (`preset:<name>`, `agent:<kind>`), newest first.
    pub agent_launch_recent: Vec<String>,
    /// Hosts sidebar search text.
    #[serde(default)]
    pub hosts_filter: String,
    /// Hosts sidebar groups shown collapsed; "" is Ungrouped.
    #[serde(default)]
    pub hosts_collapsed: HashSet<String>,
    /// Hosts sidebar lists the most recently connected projects first.
    #[serde(default)]
    pub hosts_recent_first: bool,
    /// Release tag the user chose to skip: the status-bar update chip
    /// stays hidden for this version only.
    #[serde(default)]
    pub skipped_update_version: Option<String>,
    /// Most recently opened native editor files, newest first.
    #[serde(default)]
    pub recent_native_files: Vec<PathBuf>,
    /// Files kept at the top of the native editor's recent menu.
    #[serde(default)]
    pub pinned_native_files: HashSet<PathBuf>,
    /// Language-server workspace trust per project root (true = trusted).
    #[serde(default)]
    pub lsp_trust: HashMap<PathBuf, bool>,
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
    /// Browser tab search URL and developer tools.
    #[serde(default)]
    pub browser: crate::browser::extras::BrowserPrefs,
    pub editor_states: super::EditorStates,
    /// IDE strip dock layouts per project (shell terminals). GUI-local view
    /// state; validated on load, invalid docks dropped. Kept last so the
    /// derived `PartialEq` (which gates saves) short-circuits on cheap
    /// fields first. The dock compare itself is structural: see [`StripDocks`].
    #[serde(default)]
    pub ide_strip_docks: StripDocks,
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
        // The header no longer has tool buttons: give an empty right stripe
        // the default pins once. An empty list after this is a user choice.
        if !prefs.right_pins_migrated {
            if prefs.right_tool_pins.is_empty() {
                prefs.right_tool_pins = DEFAULT_RIGHT_PINS.map(str::to_owned).to_vec();
            }
            prefs.right_pins_migrated = true;
        }
        prefs.width = clamp_sidebar_width(prefs.width);
        prefs.player_volume = if prefs.player_volume.is_finite() {
            prefs.player_volume.clamp(0.0, 1.0)
        } else {
            0.8
        };
        prefs.migrate_playlists();
        prefs.normalize_left();
        prefs.prune_expanded_tabs();
        prefs
            .ide_strip_docks
            .0
            .retain(|_, dock| crate::workspace::validate_layout(dock).is_ok());
        Ok(prefs)
    }

    /// Save the current UI state as a named layout preset.
    /// `left_width` is the left sidebar width the GUI shows now.
    pub fn save_current_layout(&mut self, name: &str, left_width: f32) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let mut preset = LayoutPreset {
            ide_mode: self.ide_mode,
            ide_terminal_collapsed: self.ide_terminal_collapsed,
            ide_sidebars_full_height: self.ide_sidebars_full_height,
            visible: self.visible,
            left_visible: self.left_visible,
            tool: self.tool,
            left_agents: false,
            left_tool: None,
            right_projects: self.right_projects,
            sidebar_width: self.width,
            left_width: clamp_left_width(left_width),
        };
        preset.set_left_content(self.left_content());
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
        let left = preset.left_content();
        self.right_projects = preset.right_projects;
        self.width = clamp_sidebar_width(preset.sidebar_width);
        self.active_layout = Some(name.to_string());
        self.set_left_content(left);
        self.named_layouts.get(name)
    }

    pub fn save(&self, data: &Path) -> Result<()> {
        fs::create_dir_all(data)?;
        terminator_core::atomic_write(
            &data.join("ui-preferences.json"),
            &serde_json::to_vec_pretty(self)?,
        )?;
        Ok(())
    }
    #[cfg(test)]
    pub fn toggle(&mut self, tool: SidebarTool) {
        self.visible = self.tool != tool || !self.visible;
        self.tool = tool;
    }
}

/// Right sidebar width in its allowed range; NaN or infinity gives the default.
#[must_use]
fn clamp_sidebar_width(width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(220.0, 480.0)
    } else {
        DEFAULT_SIDEBAR_WIDTH
    }
}

/// Left sidebar width in its panel's range; NaN or infinity gives 225.
#[must_use]
pub fn clamp_left_width(width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(170.0, 420.0)
    } else {
        225.0
    }
}
