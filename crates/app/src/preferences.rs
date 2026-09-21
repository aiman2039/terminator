use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};
use terminator_core::{Agent, Notification, Project, Session, TerminalNotice};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSort {
    #[default]
    NameAsc,
    NameDesc,
    LatestActivity,
}

impl ProjectSort {
    pub fn menu_label(self) -> &'static str {
        match self {
            Self::NameAsc => "Name A → Z",
            Self::NameDesc => "Name Z → A",
            Self::LatestActivity => "Latest activity",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistorySort {
    #[default]
    LatestActivity,
    NameAsc,
    NameDesc,
}

impl HistorySort {
    pub fn menu_label(self) -> &'static str {
        match self {
            Self::NameAsc => "Name A → Z",
            Self::NameDesc => "Name Z → A",
            Self::LatestActivity => "Latest activity",
        }
    }
}

pub struct VisibleProjects<'a> {
    pub projects: Vec<Project>,
    pub hidden: &'a HashSet<String>,
    pub sort: ProjectSort,
    pub activity: &'a HashMap<String, u64>,
    pub sessions: &'a [Session],
    pub agents: &'a [Agent],
    pub notifications: &'a [Notification],
    pub terminal_notices: &'a [TerminalNotice],
}

pub fn sort_visible_projects(input: VisibleProjects<'_>) -> Vec<Project> {
    let VisibleProjects {
        mut projects,
        hidden,
        sort,
        activity,
        sessions,
        agents,
        notifications,
        terminal_notices,
    } = input;
    projects.retain(|project| !hidden.contains(&project.id));
    let times = project_times(ProjectTimes {
        sessions,
        agents,
        notifications,
        terminal_notices,
        activity,
    });
    apply_sort(sort, &mut projects, &times);
    projects
}

struct ProjectTimes<'a> {
    sessions: &'a [Session],
    agents: &'a [Agent],
    notifications: &'a [Notification],
    terminal_notices: &'a [TerminalNotice],
    activity: &'a HashMap<String, u64>,
}

fn apply_sort(sort: ProjectSort, projects: &mut [Project], times: &HashMap<String, u64>) {
    match sort {
        ProjectSort::NameAsc => projects.sort_by(name_order),
        ProjectSort::NameDesc => projects.sort_by(|left, right| name_order(right, left)),
        ProjectSort::LatestActivity => projects.sort_by(|left, right| {
            times
                .get(&right.id)
                .copied()
                .unwrap_or(0)
                .cmp(&times.get(&left.id).copied().unwrap_or(0))
                .then_with(|| name_order(left, right))
        }),
    }
}

fn name_order(left: &Project, right: &Project) -> Ordering {
    left.name
        .to_lowercase()
        .cmp(&right.name.to_lowercase())
        .then_with(|| left.name.cmp(&right.name))
        .then_with(|| left.id.cmp(&right.id))
}

fn project_times(input: ProjectTimes<'_>) -> HashMap<String, u64> {
    let ProjectTimes {
        sessions,
        agents,
        notifications,
        terminal_notices,
        activity,
    } = input;
    let mut times = activity.clone();
    let mut owner = HashMap::new();
    for session in sessions {
        owner.insert(session.id.clone(), session.project_id.clone());
        bump(&mut times, &session.project_id, session.created);
    }
    for agent in agents {
        if let Some(project) = owner.get(&agent.session_id) {
            bump(&mut times, project, agent.updated);
        }
    }
    for notice in notifications {
        if let Some(project) = owner.get(&notice.session_id) {
            bump(&mut times, project, notice.created);
        }
    }
    for notice in terminal_notices {
        if let Some(project) = owner.get(&notice.session_id) {
            bump(&mut times, project, notice.created);
        }
    }
    times
}

fn bump(times: &mut HashMap<String, u64>, project: &str, timestamp: u64) {
    let entry = times.entry(project.to_string()).or_insert(0);
    *entry = (*entry).max(timestamp);
}

/// One project group in the global History sidebar with its sessions
/// already filtered and sorted for display.
pub struct HistoryGroup {
    pub project: Project,
    pub sessions: Vec<Session>,
    pub activity: u64,
}

pub struct HistoryInput<'a> {
    pub projects: Vec<Project>,
    pub sessions: &'a [Session],
    pub agents: &'a [Agent],
    pub notifications: &'a [Notification],
    pub terminal_notices: &'a [TerminalNotice],
    pub sort: HistorySort,
    pub filter: &'a str,
}

/// Sort History projects by last activity and sessions within each project
/// by activity, with name sorts and a case-insensitive name filter.
pub fn sort_history(input: HistoryInput<'_>) -> Vec<HistoryGroup> {
    let HistoryInput {
        projects,
        sessions,
        agents,
        notifications,
        terminal_notices,
        sort,
        filter,
    } = input;
    let query = filter.trim().to_lowercase();
    let mut groups = Vec::new();
    for project in projects {
        let mut with_activity: Vec<(Session, u64)> = sessions
            .iter()
            .filter(|s| s.project_id == project.id)
            .map(|s| {
                let activity = history_session_activity(s, agents, notifications, terminal_notices);
                (s.clone(), activity)
            })
            .collect();
        if with_activity.is_empty() {
            continue;
        }
        if !query.is_empty() {
            let project_matches = project.name.to_lowercase().contains(&query);
            with_activity
                .retain(|(s, _)| project_matches || s.label.to_lowercase().contains(&query));
            if with_activity.is_empty() {
                continue;
            }
        }
        match sort {
            HistorySort::LatestActivity => with_activity.sort_by(|left, right| {
                right
                    .1
                    .cmp(&left.1)
                    .then_with(|| session_name_order(&left.0, &right.0))
            }),
            HistorySort::NameAsc => {
                with_activity.sort_by(|left, right| session_name_order(&left.0, &right.0));
            }
            HistorySort::NameDesc => {
                with_activity.sort_by(|left, right| session_name_order(&right.0, &left.0));
            }
        }
        let activity = with_activity.iter().map(|(_, a)| *a).max().unwrap_or(0);
        groups.push(HistoryGroup {
            project,
            sessions: with_activity.into_iter().map(|(s, _)| s).collect(),
            activity,
        });
    }
    match sort {
        HistorySort::LatestActivity => groups.sort_by(|left, right| {
            right
                .activity
                .cmp(&left.activity)
                .then_with(|| name_order(&left.project, &right.project))
        }),
        HistorySort::NameAsc => {
            groups.sort_by(|left, right| name_order(&left.project, &right.project))
        }
        HistorySort::NameDesc => {
            groups.sort_by(|left, right| name_order(&right.project, &left.project));
        }
    }
    groups
}

fn history_session_activity(
    session: &Session,
    agents: &[Agent],
    notifications: &[Notification],
    terminal_notices: &[TerminalNotice],
) -> u64 {
    let mut activity = session.created;
    for agent in agents.iter().filter(|a| a.session_id == session.id) {
        activity = activity.max(agent.updated);
    }
    for notice in notifications.iter().filter(|n| n.session_id == session.id) {
        activity = activity.max(notice.created);
    }
    for notice in terminal_notices
        .iter()
        .filter(|n| n.session_id == session.id)
    {
        activity = activity.max(notice.created);
    }
    activity
}

fn default_true() -> bool {
    true
}

fn session_name_order(left: &Session, right: &Session) -> Ordering {
    left.label
        .to_lowercase()
        .cmp(&right.label.to_lowercase())
        .then_with(|| left.label.cmp(&right.label))
        .then_with(|| left.id.cmp(&right.id))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarTool {
    #[default]
    Explorer,
    Agents,
    Git,
    History,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPreferences {
    pub version: u32,
    pub setup_completed: bool,
    pub expanded: HashMap<String, bool>,
    pub history_expanded: HashMap<String, bool>,
    pub tool: SidebarTool,
    pub visible: bool,
    /// Projects column. Missing files stay open; `bool`'s serde default is false.
    #[serde(default = "default_true")]
    pub left_visible: bool,
    pub left_agents: bool,
    pub width: f32,
    pub all_projects: bool,
    pub show_ignored: bool,
    pub typography_migrated: bool,
    pub attention_migrated: bool,
    pub markdown_modes: HashMap<String, crate::markdown::Mode>,
    pub hidden_projects: HashSet<String>,
    pub project_sort: ProjectSort,
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
}
impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            version: 1,
            setup_completed: false,
            expanded: HashMap::new(),
            history_expanded: HashMap::new(),
            tool: SidebarTool::Explorer,
            visible: true,
            left_visible: true,
            left_agents: false,
            width: 285.0,
            all_projects: false,
            show_ignored: false,
            typography_migrated: false,
            attention_migrated: false,
            markdown_modes: HashMap::new(),
            hidden_projects: HashSet::new(),
            project_sort: ProjectSort::NameAsc,
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
        let mut prefs: Self = serde_json::from_slice(&bytes).context("Invalid UI preferences")?;
        anyhow::ensure!(prefs.version == 1, "Unsupported UI preference version");
        prefs.width = if prefs.width.is_finite() {
            prefs.width.clamp(220.0, 480.0)
        } else {
            285.0
        };
        prefs.player_volume = if prefs.player_volume.is_finite() {
            prefs.player_volume.clamp(0.0, 1.0)
        } else {
            0.8
        };
        prefs.migrate_playlists();
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
        {
            self.selected_playlist = self.playlists[0].name.clone();
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
        p.all_projects = true;
        p.typography_migrated = true;
        p.attention_migrated = true;
        p.hidden_projects.insert("hidden-project".into());
        p.project_sort = ProjectSort::LatestActivity;
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
        fs::write(dir.path().join("ui-preferences.json"), r#"{"version":2}"#).unwrap();
        assert!(UiPreferences::load(dir.path()).is_err());
        fs::write(dir.path().join("ui-preferences.json"), r#"{"width":900}"#).unwrap();
        assert_eq!(UiPreferences::load(dir.path()).unwrap().width, 480.0);
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

    fn project(id: &str, name: &str) -> Project {
        Project {
            id: id.into(),
            name: name.into(),
            path: format!("/{id}").into(),
            layout: serde_json::Value::Null,
        }
    }

    fn session(id: &str, project: &str, created: u64) -> Session {
        Session {
            review: false,
            id: id.into(),
            project_id: project.into(),
            label: id.into(),
            cwd: format!("/{project}").into(),
            kind: terminator_core::SessionKind::Shell,
            file: None,
            lifecycle: terminator_core::Lifecycle::Running,
            created,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "test".into(),
            pid: None,
            truncated: false,
            cwd_confirmed: true,
        }
    }

    fn ids(input: VisibleProjects<'_>) -> Vec<String> {
        sort_visible_projects(input)
            .into_iter()
            .map(|project| project.id)
            .collect()
    }

    #[test]
    fn name_sort_is_case_insensitive_and_omits_hidden_projects() {
        let hidden = HashSet::from(["skip".into()]);
        let projects = vec![
            project("z", "Banana"),
            project("skip", "aaaa"),
            project("a", "apple"),
            project("m", "Banana"),
        ];
        let empty = HashMap::new();
        let input = |sort: ProjectSort, projects: Vec<Project>| VisibleProjects {
            projects,
            hidden: &hidden,
            sort,
            activity: &empty,
            sessions: &[],
            agents: &[],
            notifications: &[],
            terminal_notices: &[],
        };
        assert_eq!(
            ids(input(ProjectSort::NameAsc, projects.clone())),
            ["a", "m", "z"]
        );
        assert_eq!(ids(input(ProjectSort::NameDesc, projects)), ["z", "m", "a"]);
    }

    #[test]
    fn latest_activity_uses_max_timestamp_and_name_tie_break() {
        let hidden = HashSet::new();
        let projects = vec![project("z", "zebra"), project("a", "alpha")];
        let sessions = [session("sz", "z", 10), session("sa", "a", 5)];
        let empty = HashMap::new();
        let mut agents = vec![Agent {
            invocation_id: "i".into(),
            session_id: "sa".into(),
            kind: "custom".into(),
            provider_session_id: None,
            state: terminator_core::AgentState::Running,
            sequence: None,
            updated: 20,
            resume: None,
        }];
        let ranked = |activity: &HashMap<String, u64>, agents: &[Agent]| {
            ids(VisibleProjects {
                projects: projects.clone(),
                hidden: &hidden,
                sort: ProjectSort::LatestActivity,
                activity,
                sessions: &sessions,
                agents,
                notifications: &[],
                terminal_notices: &[],
            })
        };
        assert_eq!(ranked(&empty, &[]), ["z", "a"]);
        assert_eq!(ranked(&empty, &agents), ["a", "z"]);
        agents[0].updated = 10;
        assert_eq!(ranked(&empty, &agents), ["a", "z"]);
        let mut activity = HashMap::new();
        activity.insert("z".into(), 30);
        assert_eq!(ranked(&activity, &agents), ["z", "a"]);
    }

    #[test]
    fn latest_activity_includes_notice_timestamps() {
        let hidden = HashSet::new();
        let empty = HashMap::new();
        let sessions = [session("sa", "a", 1), session("sb", "b", 2)];
        let notifications = [Notification {
            id: "n".into(),
            session_id: "sa".into(),
            invocation_id: "i".into(),
            request_id: None,
            state: terminator_core::AgentState::Completed,
            summary: String::new(),
            details: String::new(),
            created: 8,
            read: true,
            dismissed: false,
            resolved: true,
            snoozed_until: 0,
        }];
        let terminal_notices = [TerminalNotice {
            id: "t".into(),
            session_id: "sb".into(),
            title: String::new(),
            body: String::new(),
            created: 9,
            dismissed: false,
        }];
        assert_eq!(
            ids(VisibleProjects {
                projects: vec![project("a", "a"), project("b", "b")],
                hidden: &hidden,
                sort: ProjectSort::LatestActivity,
                activity: &empty,
                sessions: &sessions,
                agents: &[],
                notifications: &notifications,
                terminal_notices: &terminal_notices,
            }),
            ["b", "a"]
        );
    }

    fn history_session(id: &str, project: &str, label: &str, created: u64) -> Session {
        let mut s = session(id, project, created);
        s.label = label.into();
        s.lifecycle = terminator_core::Lifecycle::Ended;
        s
    }

    fn history_agent(session: &str, updated: u64) -> Agent {
        Agent {
            invocation_id: format!("agent-{session}"),
            session_id: session.into(),
            kind: "custom".into(),
            provider_session_id: None,
            state: terminator_core::AgentState::Stopped,
            sequence: None,
            updated,
            resume: None,
        }
    }

    fn history_groups(
        projects: Vec<Project>,
        sessions: &[Session],
        agents: &[Agent],
        sort: HistorySort,
        filter: &str,
    ) -> Vec<(String, Vec<String>)> {
        sort_history(HistoryInput {
            projects,
            sessions,
            agents,
            notifications: &[],
            terminal_notices: &[],
            sort,
            filter,
        })
        .into_iter()
        .map(|group| {
            (
                group.project.id,
                group.sessions.into_iter().map(|s| s.id).collect(),
            )
        })
        .collect()
    }

    #[test]
    fn history_sorts_projects_by_last_activity_and_sessions_by_activity() {
        let projects = vec![project("a", "alpha"), project("b", "beta")];
        let sessions = vec![
            history_session("a-old", "a", "Terminal 1", 10),
            history_session("a-new", "a", "Terminal 4", 12),
            history_session("b-only", "b", "Terminal 2", 11),
        ];
        let agents = vec![history_agent("a-old", 30), history_agent("b-only", 20)];
        let groups = history_groups(
            projects,
            &sessions,
            &agents,
            HistorySort::LatestActivity,
            "",
        );
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "a");
        assert_eq!(groups[0].1, vec!["a-old".to_string(), "a-new".to_string()]);
        assert_eq!(groups[1].0, "b");
    }

    #[test]
    fn history_name_sort_orders_projects_and_sessions() {
        let projects = vec![project("b", "beta"), project("a", "alpha")];
        let sessions = vec![
            history_session("s2", "a", "Terminal 4", 2),
            history_session("s1", "a", "Terminal 1", 1),
        ];
        let groups = history_groups(projects.clone(), &sessions, &[], HistorySort::NameAsc, "");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, "a");
        assert_eq!(groups[0].1, vec!["s1".to_string(), "s2".to_string()]);
        let groups = history_groups(projects, &sessions, &[], HistorySort::NameDesc, "");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, "a");
        assert_eq!(groups[0].1, vec!["s2".to_string(), "s1".to_string()]);
        let projects = vec![project("b", "beta"), project("a", "alpha")];
        let sessions = vec![
            history_session("sa", "a", "Terminal 1", 1),
            history_session("sb", "b", "Terminal 2", 1),
        ];
        let groups = history_groups(projects, &sessions, &[], HistorySort::NameDesc, "");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "b");
        assert_eq!(groups[1].0, "a");
    }

    #[test]
    fn history_filter_matches_session_and_project_names() {
        let projects = vec![project("a", "alpha"), project("b", "beta")];
        let sessions = vec![
            history_session("s1", "a", "Terminal 1", 1),
            history_session("s2", "a", "Terminal 4", 2),
            history_session("s3", "b", "Terminal 2", 3),
        ];
        let groups = history_groups(
            projects.clone(),
            &sessions,
            &[],
            HistorySort::LatestActivity,
            "terminal 4",
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, "a");
        assert_eq!(groups[0].1, vec!["s2".to_string()]);
        let groups = history_groups(
            projects,
            &sessions,
            &[],
            HistorySort::LatestActivity,
            "BETA",
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, "b");
        assert_eq!(groups[0].1, vec!["s3".to_string()]);
    }
}
