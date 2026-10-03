//! GUI-only prepared sidebar data. Snapshot/presentation changes invalidate the
//! indexes; scrolling, selection and expansion reuse the same immutable data.
use crate::{
    App,
    preferences::{self, HistoryGroup, HistoryInput, HistorySort, ProjectSort, VisibleProjects},
    sidebar_ui::{NoticeGroup, group_notices, notice_rank},
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};
use terminator_core::{AgentState, Notification, Session, SessionKind, TerminalNotice};

#[derive(PartialEq, Eq)]
struct Stamp {
    generation: u64,
    projects: (usize, usize, usize),
    worktrees: (usize, usize, usize),
    terminal_notices: (usize, usize, usize),
}

#[derive(Default)]
pub(super) struct Cache {
    stamp: Option<Stamp>,
    index: Arc<Index>,
    projects: Option<(ProjectKey, Arc<Vec<Arc<terminator_core::Project>>>)>,
    history: Option<(HistorySort, String, Arc<Vec<HistoryGroup>>)>,
    unread: Option<(Option<String>, Arc<Vec<NoticeGroup>>)>,
    live_rows: Option<(String, String, Arc<LiveRows>)>,
    pub git: Option<Arc<crate::sidebar_ui::PreparedGit>>,
    pub explorer: Option<ExplorerCache>,
    pub files_revision: u64,
}
#[derive(Default)]
pub(super) struct Index {
    pub sessions: HashMap<String, Arc<Session>>,
    pub by_project: HashMap<String, Vec<Arc<Session>>>,
    pub terminal_notes: HashMap<String, TerminalNotice>,
    pub live_counts: HashMap<String, usize>,
    pub faces: HashMap<String, (&'static str, AgentState)>,
    pub worktree_states: HashMap<String, (bool, bool)>,
    pub history_brands: HashMap<(String, String), (&'static str, String)>,
    pub kinds: HashMap<String, Vec<String>>,
    pub pending: Vec<Notification>,
    pub pending_groups: Vec<NoticeGroup>,
    pub terminal_pending: Vec<TerminalNotice>,
    pub unread_count: usize,
    pub waiting_count: usize,
    pub terminal_layout_keys: HashMap<String, u64>,
    pub resumable: Vec<Session>,
    pub children: HashMap<String, Vec<Arc<terminator_core::Project>>>,
    pub parents: HashMap<String, Vec<String>>,
    pub managed: HashSet<String>,
}
#[derive(PartialEq, Eq)]
struct ProjectKey {
    hidden: HashSet<String>,
    sort: ProjectSort,
    filter: String,
    activity: HashMap<String, u64>,
}
#[derive(Default)]
pub(super) struct LiveRows {
    pub groups: Vec<(Arc<terminator_core::Project>, Vec<Arc<Session>>)>,
    pub unverified: Vec<Arc<Session>>,
}
pub(super) struct ExplorerCache {
    pub revision: u64,
    pub root: PathBuf,
    pub expanded: HashSet<PathBuf>,
    pub ignored: bool,
    pub query: String,
    pub rows: Arc<Vec<ExplorerRow>>,
    pub dirs: Vec<PathBuf>,
}
pub(super) struct ExplorerRow {
    pub entry: Option<terminator_git::Entry>,
    pub path: PathBuf,
    pub label: String,
    pub depth: usize,
}
impl Cache {
    pub fn clear_files(&mut self) {
        self.files_revision = self.files_revision.wrapping_add(1);
        self.explorer = None;
        self.git = None;
    }
}
impl App {
    pub(super) fn sidebar_index(&self) -> Arc<Index> {
        self.sidebar_index_at(crate::now())
    }

    pub(super) fn sidebar_index_at(&self, moment: u64) -> Arc<Index> {
        let generation = self.presentations.borrow_mut().generation(
            &self.state,
            moment,
            self.services.presence_fresh(),
        );
        let stamp = Stamp {
            generation,
            projects: (
                self.state.projects.as_ptr() as usize,
                self.state.projects.len(),
                self.state.projects.capacity(),
            ),
            worktrees: (
                self.state.worktrees.as_ptr() as usize,
                self.state.worktrees.len(),
                self.state.worktrees.capacity(),
            ),
            terminal_notices: (
                self.state.terminal_notices.as_ptr() as usize,
                self.state.terminal_notices.len(),
                self.state.terminal_notices.capacity(),
            ),
        };
        if self.sidebar_cache.borrow().stamp.as_ref() == Some(&stamp) {
            return Arc::clone(&self.sidebar_cache.borrow().index);
        }
        let mut index = Index::default();
        let resumable: HashSet<_> = self
            .state
            .agents
            .iter()
            .filter(|a| a.resumable())
            .map(|a| a.session_id.as_str())
            .collect();
        for session in &self.state.sessions {
            let session = Arc::new(session.clone());
            index
                .sessions
                .insert(session.id.clone(), Arc::clone(&session));
            index
                .by_project
                .entry(session.project_id.clone())
                .or_default()
                .push(Arc::clone(&session));
            if !session.lifecycle.live() {
                if resumable.contains(session.id.as_str()) {
                    index.resumable.push(session.as_ref().clone());
                }
                continue;
            }
            let presented = self.present_session(&session.id);
            let flags = index
                .worktree_states
                .entry(session.project_id.clone())
                .or_default();
            match presented.lifecycle {
                Some(AgentState::WaitingInput | AgentState::WaitingPermission) => flags.0 = true,
                Some(AgentState::Running) => flags.1 = true,
                _ => {}
            }
            if session.kind == SessionKind::Editor {
                continue;
            }
            let count = index
                .live_counts
                .entry(session.project_id.clone())
                .or_default();
            *count = count.saturating_add(1);
            if let Some(brand) = presented.brand_icon {
                let state = presented.lifecycle.unwrap_or(AgentState::Unknown);
                let rank = |state| match state {
                    AgentState::Running => 0,
                    AgentState::WaitingInput | AgentState::WaitingPermission => 1,
                    AgentState::Failed => 2,
                    AgentState::Completed => 3,
                    _ => 4,
                };
                if index
                    .faces
                    .get(&session.project_id)
                    .is_none_or(|(_, old)| rank(state) < rank(*old))
                {
                    index
                        .faces
                        .insert(session.project_id.clone(), (brand, state));
                }
            }
        }
        for agent in &self.state.agents {
            index
                .history_brands
                .entry((agent.session_id.clone(), agent.invocation_id.clone()))
                .or_insert_with(|| {
                    (
                        terminator_core::agents::icon_key(&agent.kind),
                        terminator_core::agents::display_name(&agent.kind).to_string(),
                    )
                });
            index
                .kinds
                .entry(agent.session_id.clone())
                .or_default()
                .push(agent.kind.clone());
        }
        let now = moment;
        for notice in &self.state.notifications {
            if !notice.read && super::sidebar_ui::notice_pending(notice, now) {
                index.unread_count = index.unread_count.saturating_add(1);
            }
            if !notice.dismissed
                && !notice.resolved
                && notice.snoozed_until <= now
                && index.sessions.contains_key(&notice.session_id)
            {
                index.pending.push(notice.clone());
            }
        }
        index.pending.sort_by_key(|n| {
            (
                n.resolved,
                notice_rank(n.state),
                std::cmp::Reverse(n.created),
            )
        });
        index.waiting_count = index
            .pending
            .iter()
            .filter(|n| crate::sidebar_ui::notice_waiting(n))
            .count();
        index.pending_groups = group_notices(index.pending.clone());
        for notice in &self.state.terminal_notices {
            use std::hash::{Hash, Hasher};
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            notice.id.hash(&mut hash);
            notice.title.hash(&mut hash);
            notice.body.hash(&mut hash);
            index
                .terminal_layout_keys
                .insert(notice.id.clone(), hash.finish());
            index
                .terminal_notes
                .insert(notice.session_id.clone(), notice.clone());
            if !notice.dismissed && index.sessions.contains_key(&notice.session_id) {
                index.terminal_pending.push(notice.clone());
            }
        }
        index.terminal_pending.reverse();
        let projects_by_id: HashMap<_, _> = self
            .state
            .projects
            .iter()
            .map(|p| (p.id.as_str(), p))
            .collect();
        index.managed = self
            .state
            .worktrees
            .iter()
            .filter(|w| !w.removed)
            .map(|w| w.project_id.clone())
            .collect();
        for project in &self.state.projects {
            if index.managed.contains(&project.id) {
                continue;
            }
            let children: Vec<_> = self
                .state
                .worktrees
                .iter()
                .filter(|w| {
                    !w.removed
                        && w.project_id != project.id
                        && (w.common_dir.starts_with(&project.path)
                            || w.common_dir.parent() == Some(project.path.as_path()))
                })
                .filter_map(|w| projects_by_id.get(w.project_id.as_str()))
                .map(|p| Arc::new((*p).clone()))
                .collect();
            for child in &children {
                index
                    .parents
                    .entry(child.id.clone())
                    .or_default()
                    .push(project.id.clone());
            }
            index.children.insert(project.id.clone(), children);
        }
        let index = Arc::new(index);
        let mut cache = self.sidebar_cache.borrow_mut();
        cache.stamp = Some(stamp);
        cache.index = Arc::clone(&index);
        cache.projects = None;
        cache.history = None;
        cache.unread = None;
        cache.live_rows = None;
        index
    }
    pub(super) fn cached_projects(&self) -> Arc<Vec<Arc<terminator_core::Project>>> {
        self.sidebar_index();
        let mut cache = self.sidebar_cache.borrow_mut();
        if let Some((key, rows)) = &cache.projects
            && key.hidden == self.preferences.hidden_projects
            && key.sort == self.preferences.project_sort
            && key.filter == self.preferences.project_filter
            && key.activity == self.preferences.project_activity
        {
            return Arc::clone(rows);
        }
        let rows: Arc<Vec<Arc<terminator_core::Project>>> = Arc::new(
            preferences::sort_visible_projects(VisibleProjects {
                projects: &self.state.projects,
                hidden: &self.preferences.hidden_projects,
                sort: self.preferences.project_sort,
                filter: &self.preferences.project_filter,
                activity: &self.preferences.project_activity,
                sessions: &self.state.sessions,
                agents: &self.state.agents,
                notifications: &self.state.notifications,
                terminal_notices: &self.state.terminal_notices,
            })
            .into_iter()
            .map(|p| self.sidebar_project(p))
            .collect(),
        );
        cache.projects = Some((
            ProjectKey {
                hidden: self.preferences.hidden_projects.clone(),
                sort: self.preferences.project_sort,
                filter: self.preferences.project_filter.clone(),
                activity: self.preferences.project_activity.clone(),
            },
            Arc::clone(&rows),
        ));
        rows
    }
    pub(super) fn cached_history(&self) -> Arc<Vec<HistoryGroup>> {
        let index = self.sidebar_index();
        let mut cache = self.sidebar_cache.borrow_mut();
        if let Some((sort, filter, rows)) = &cache.history
            && *sort == self.preferences.history_sort
            && *filter == self.preferences.history_filter
        {
            return Arc::clone(rows);
        }
        let rows = Arc::new(preferences::sort_history(HistoryInput {
            projects: self.state.projects.clone(),
            sessions: &index.resumable,
            agents: &self.state.agents,
            notifications: &self.state.notifications,
            terminal_notices: &self.state.terminal_notices,
            sort: self.preferences.history_sort,
            filter: &self.preferences.history_filter,
        }));
        cache.history = Some((
            self.preferences.history_sort,
            self.preferences.history_filter.clone(),
            Arc::clone(&rows),
        ));
        rows
    }
    pub(super) fn cached_live_rows(&self) -> Arc<LiveRows> {
        let index = self.sidebar_index();
        let query = self.preferences.agents_search.trim().to_lowercase();
        let kind = &self.preferences.agents_filter;
        let mut cache = self.sidebar_cache.borrow_mut();
        if let Some((old_query, old_kind, rows)) = &cache.live_rows
            && *old_query == query
            && old_kind == kind
        {
            return Arc::clone(rows);
        }
        let mut live: HashMap<String, Vec<Arc<Session>>> = HashMap::new();
        let mut rows = LiveRows::default();
        for session in self.state.sessions.iter().filter(|s| s.lifecycle.live()) {
            let presented = self.present_session(&session.id);
            if !presented.live && presented.lifecycle.is_none() {
                continue;
            }
            let kinds = if presented.live {
                &presented.detected_kinds
            } else {
                index
                    .kinds
                    .get(&session.id)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
            };
            if !kind.is_empty() && !kinds.iter().any(|k| k == kind) {
                continue;
            }
            if !query.is_empty() {
                let project = self
                    .state
                    .projects
                    .iter()
                    .find(|p| p.id == session.project_id)
                    .map(|p| p.name.as_str())
                    .unwrap_or_default();
                let mut haystack =
                    format!("{} {project} {}", session.label, presented.status_label)
                        .to_lowercase();
                for kind in kinds {
                    haystack.push(' ');
                    haystack.push_str(&terminator_core::agents::display_name(kind).to_lowercase());
                }
                if !query.split_whitespace().all(|word| haystack.contains(word)) {
                    continue;
                }
            }
            let Some(session) = index.sessions.get(&session.id).map(Arc::clone) else {
                continue;
            };
            if presented.live {
                live.entry(session.project_id.clone())
                    .or_default()
                    .push(session);
            } else {
                rows.unverified.push(session);
            }
        }
        for project in &self.state.projects {
            if let Some(sessions) = live.remove(&project.id) {
                rows.groups.push((self.sidebar_project(project), sessions));
            }
        }
        let rows = Arc::new(rows);
        cache.live_rows = Some((query, kind.clone(), Arc::clone(&rows)));
        rows
    }
    pub(super) fn explorer_rows(&mut self, root: &std::path::Path) -> Arc<Vec<ExplorerRow>> {
        let cache = self.sidebar_cache.get_mut();
        if let Some(model) = &cache.explorer
            && model.revision == cache.files_revision
            && model.root == root
            && model.expanded == self.expanded_dirs
            && model.ignored == self.preferences.show_ignored
            && model.query == self.explorer_query
        {
            self.visible_dirs.extend(model.dirs.iter().cloned());
            return Arc::clone(&model.rows);
        }
        fn walk(
            app: &App,
            path: &std::path::Path,
            depth: usize,
            query: Option<&str>,
            rows: &mut Vec<ExplorerRow>,
            dirs: &mut Vec<PathBuf>,
        ) {
            if depth > 20 {
                return;
            }
            dirs.push(path.to_owned());
            if app.directory_errors.contains_key(path) || !app.dirs.contains_key(path) {
                rows.push(ExplorerRow {
                    entry: None,
                    path: path.to_owned(),
                    label: String::new(),
                    depth,
                });
            }
            if let Some(entries) = app.dirs.get(path) {
                for entry in entries {
                    if entry.ignored && !app.preferences.show_ignored {
                        continue;
                    }
                    let label = entry
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    if !crate::sidebar_ui::explorer_query_keeps_file(&label, entry.directory, query)
                    {
                        continue;
                    }
                    rows.push(ExplorerRow {
                        entry: Some(entry.clone()),
                        path: entry.path.clone(),
                        label,
                        depth,
                    });
                    if entry.directory && app.expanded_dirs.contains(&entry.path) {
                        walk(app, &entry.path, depth.saturating_add(1), query, rows, dirs);
                    }
                }
            }
        }
        let query = (!self.explorer_query.is_empty()).then(|| self.explorer_query.to_lowercase());
        let mut rows = Vec::new();
        let mut dirs = Vec::new();
        walk(self, root, 0, query.as_deref(), &mut rows, &mut dirs);
        self.visible_dirs.extend(dirs.iter().cloned());
        let rows = Arc::new(rows);
        let cache = self.sidebar_cache.get_mut();
        cache.explorer = Some(ExplorerCache {
            revision: cache.files_revision,
            root: root.to_owned(),
            expanded: self.expanded_dirs.clone(),
            ignored: self.preferences.show_ignored,
            query: self.explorer_query.clone(),
            rows: Arc::clone(&rows),
            dirs,
        });
        rows
    }
    pub(super) fn cached_unread_groups(&self) -> Arc<Vec<NoticeGroup>> {
        let index = self.sidebar_index();
        let mut cache = self.sidebar_cache.borrow_mut();
        if let Some((selected, rows)) = &cache.unread
            && *selected == self.unread_selected
        {
            return Arc::clone(rows);
        }
        let mut notices: Vec<_> = index
            .pending
            .iter()
            .filter(|n| !n.read || self.unread_selected.as_deref() == Some(n.id.as_str()))
            .cloned()
            .collect();
        notices.sort_by_key(|n| std::cmp::Reverse(n.created));
        let rows = Arc::new(group_notices(notices));
        cache.unread = Some((self.unread_selected.clone(), Arc::clone(&rows)));
        rows
    }
}
