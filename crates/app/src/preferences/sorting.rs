use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
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
    pub projects: &'a [Project],
    pub hidden: &'a HashSet<String>,
    pub sort: ProjectSort,
    pub filter: &'a str,
    pub activity: &'a HashMap<String, u64>,
    pub sessions: &'a [Session],
    pub agents: &'a [Agent],
    pub notifications: &'a [Notification],
    pub terminal_notices: &'a [TerminalNotice],
}

pub fn sort_visible_projects(input: VisibleProjects<'_>) -> Vec<&Project> {
    let VisibleProjects {
        projects,
        hidden,
        sort,
        filter,
        activity,
        sessions,
        agents,
        notifications,
        terminal_notices,
    } = input;
    let query = filter.trim().to_lowercase();
    let mut projects: Vec<_> = projects
        .iter()
        .filter(|project| !hidden.contains(&project.id))
        .filter(|project| {
            query.is_empty()
                || project.name.to_lowercase().contains(&query)
                || project
                    .path
                    .display()
                    .to_string()
                    .to_lowercase()
                    .contains(&query)
        })
        .collect();
    let times = if sort == ProjectSort::LatestActivity {
        project_times(ProjectTimes {
            sessions,
            agents,
            notifications,
            terminal_notices,
            activity,
        })
    } else {
        HashMap::new()
    };
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

fn apply_sort(sort: ProjectSort, projects: &mut [&Project], times: &HashMap<String, u64>) {
    match sort {
        ProjectSort::NameAsc => projects.sort_by(|left, right| name_order(left, right)),
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
    let mut activity = HashMap::new();
    for session in sessions {
        activity.insert(session.id.as_str(), session.created);
    }
    for agent in agents {
        if let Some(value) = activity.get_mut(agent.session_id.as_str()) {
            *value = (*value).max(agent.updated);
        }
    }
    for notice in notifications {
        if let Some(value) = activity.get_mut(notice.session_id.as_str()) {
            *value = (*value).max(notice.created);
        }
    }
    for notice in terminal_notices {
        if let Some(value) = activity.get_mut(notice.session_id.as_str()) {
            *value = (*value).max(notice.created);
        }
    }
    let mut by_project: HashMap<&str, Vec<&Session>> = HashMap::new();
    for session in sessions {
        by_project
            .entry(session.project_id.as_str())
            .or_default()
            .push(session);
    }
    let mut groups = Vec::new();
    for project in projects {
        let mut with_activity: Vec<(Session, u64)> = by_project
            .remove(project.id.as_str())
            .unwrap_or_default()
            .into_iter()
            .map(|s| {
                (
                    s.clone(),
                    activity.get(s.id.as_str()).copied().unwrap_or(s.created),
                )
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
            groups.sort_by(|left, right| name_order(&left.project, &right.project));
        }
        HistorySort::NameDesc => {
            groups.sort_by(|left, right| name_order(&right.project, &left.project));
        }
    }
    groups
}

fn session_name_order(left: &Session, right: &Session) -> Ordering {
    left.label
        .to_lowercase()
        .cmp(&right.label.to_lowercase())
        .then_with(|| left.label.cmp(&right.label))
        .then_with(|| left.id.cmp(&right.id))
}

#[cfg(test)]
mod tests {
    use super::*;

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
            .map(|project| project.id.clone())
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
        let input = |sort: ProjectSort| VisibleProjects {
            projects: &projects,
            hidden: &hidden,
            sort,
            filter: "",
            activity: &empty,
            sessions: &[],
            agents: &[],
            notifications: &[],
            terminal_notices: &[],
        };
        assert_eq!(ids(input(ProjectSort::NameAsc)), ["a", "m", "z"]);
        assert_eq!(ids(input(ProjectSort::NameDesc)), ["z", "m", "a"]);
    }

    #[test]
    fn project_filter_matches_name_or_path_case_insensitively() {
        let hidden = HashSet::new();
        let projects = vec![
            project("term", "terminator"),
            project("fomo", "fomo-rh-fe-sol-be"),
            project("other", "Other"),
        ];
        let empty = HashMap::new();
        let filtered = |filter: &str| {
            ids(VisibleProjects {
                projects: &projects,
                hidden: &hidden,
                sort: ProjectSort::NameAsc,
                filter,
                activity: &empty,
                sessions: &[],
                agents: &[],
                notifications: &[],
                terminal_notices: &[],
            })
        };
        assert_eq!(filtered(""), ["fomo", "other", "term"]);
        assert_eq!(filtered("term"), ["term"]);
        assert_eq!(filtered("FOMO"), ["fomo"]);
        assert_eq!(filtered("fomo-rh"), ["fomo"]);
        assert_eq!(filtered("/fomo"), ["fomo"]);
        assert_eq!(filtered("zzz"), Vec::<String>::new());
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
            process: None,
        }];
        let ranked = |activity: &HashMap<String, u64>, agents: &[Agent]| {
            ids(VisibleProjects {
                projects: &projects,
                hidden: &hidden,
                sort: ProjectSort::LatestActivity,
                filter: "",
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
                projects: &[project("a", "a"), project("b", "b")],
                hidden: &hidden,
                sort: ProjectSort::LatestActivity,
                filter: "",
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
            process: None,
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
