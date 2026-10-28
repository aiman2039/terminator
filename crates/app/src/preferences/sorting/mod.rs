#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
};

#[cfg(test)]
struct HistoryGroupsInput<'a> {
    projects: Vec<Project>,
    sessions: &'a [Session],
    agents: &'a [Agent],
    sort: HistorySort,
    filter: &'a str,
}

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
    let ranking = HistoryRanking {
        activity: session_activity(sessions, agents, notifications, terminal_notices),
        query: filter.trim().to_lowercase(),
        sort,
    };
    let mut by_project: HashMap<&str, Vec<&Session>> = HashMap::new();
    for session in sessions {
        by_project
            .entry(session.project_id.as_str())
            .or_default()
            .push(session);
    }
    let mut groups: Vec<HistoryGroup> = projects
        .into_iter()
        .filter_map(|project| {
            let members = by_project.remove(project.id.as_str()).unwrap_or_default();
            ranking.group(project, members)
        })
        .collect();
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

/// Each session's latest activity: creation, agent updates, and notices.
fn session_activity<'a>(
    sessions: &'a [Session],
    agents: &[Agent],
    notifications: &[Notification],
    terminal_notices: &[TerminalNotice],
) -> HashMap<&'a str, u64> {
    let mut activity = HashMap::new();
    for session in sessions {
        activity.insert(session.id.as_str(), session.created);
    }
    let mut touch = |session: &str, at: u64| {
        if let Some(value) = activity.get_mut(session) {
            *value = (*value).max(at);
        }
    };
    for agent in agents {
        touch(&agent.session_id, agent.updated);
    }
    for notice in notifications {
        touch(&notice.session_id, notice.created);
    }
    for notice in terminal_notices {
        touch(&notice.session_id, notice.created);
    }
    activity
}

/// Filter and sort order shared by every History project group.
struct HistoryRanking<'a> {
    activity: HashMap<&'a str, u64>,
    query: String,
    sort: HistorySort,
}

impl HistoryRanking<'_> {
    /// The project's sessions that pass the filter, sorted; `None` when
    /// nothing is left to show.
    fn group(&self, project: Project, sessions: Vec<&Session>) -> Option<HistoryGroup> {
        let mut with_activity: Vec<(Session, u64)> = sessions
            .into_iter()
            .map(|s| {
                (
                    s.clone(),
                    self.activity
                        .get(s.id.as_str())
                        .copied()
                        .unwrap_or(s.created),
                )
            })
            .collect();
        if with_activity.is_empty() {
            return None;
        }
        if !self.query.is_empty() {
            let project_matches = project.name.to_lowercase().contains(&self.query);
            with_activity
                .retain(|(s, _)| project_matches || s.label.to_lowercase().contains(&self.query));
            if with_activity.is_empty() {
                return None;
            }
        }
        match self.sort {
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
        Some(HistoryGroup {
            project,
            sessions: with_activity.into_iter().map(|(s, _)| s).collect(),
            activity,
        })
    }
}

fn session_name_order(left: &Session, right: &Session) -> Ordering {
    left.label
        .to_lowercase()
        .cmp(&right.label.to_lowercase())
        .then_with(|| left.label.cmp(&right.label))
        .then_with(|| left.id.cmp(&right.id))
}
