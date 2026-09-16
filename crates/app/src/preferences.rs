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
    pub player_playlists: HashMap<String, Vec<PathBuf>>,
    pub player_index: HashMap<String, usize>,
    pub player_volume: f32,
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
            player_playlists: HashMap::new(),
            player_index: HashMap::new(),
            player_volume: 0.8,
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
        assert_eq!(old.project_sort, ProjectSort::NameAsc);
        assert!(old.project_activity.is_empty());
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
}
