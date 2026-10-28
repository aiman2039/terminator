//! Snapshot-derived sidebar index, rebuilt only when the stamp changes.
use super::{Index, Stamp};
use crate::{
    App, daemon_connection,
    sidebar_ui::{group_notices, notice_rank},
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use terminator_core::{
    Agent, AgentState, Notification, Session, SessionKind, State, TerminalNotice,
};

impl App {
    pub(crate) fn sidebar_index_at(&self, moment: u64) -> Arc<Index> {
        let stamp = self.sidebar_stamp(moment);
        if self.sidebar_cache.borrow().stamp.as_ref() == Some(&stamp) {
            return Arc::clone(&self.sidebar_cache.borrow().index);
        }
        let mut index = Index::default();
        self.index_sessions(&mut index);
        index.add_agents(&self.state.agents);
        index.add_notifications(&self.state.notifications, moment);
        index.add_terminal_notices(&self.state.terminal_notices);
        index.add_worktree_children(&self.state);
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

    fn sidebar_stamp(&self, moment: u64) -> Stamp {
        let generation = self.presentations.borrow_mut().generation(
            &self.state,
            moment,
            self.services.presence_fresh(),
        );
        Stamp {
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
        }
    }

    fn index_sessions(&self, index: &mut Index) {
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
            if daemon_connection::session_owner_available(&self.state, &session) {
                self.index_live_session(index, &session);
            }
        }
    }

    fn index_live_session(&self, index: &mut Index, session: &Session) {
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
            return;
        }
        let count = index
            .live_counts
            .entry(session.project_id.clone())
            .or_default();
        *count = count.saturating_add(1);
        if let Some(brand) = presented.brand_icon {
            let state = presented.lifecycle.unwrap_or(AgentState::Unknown);
            index.offer_face(&session.project_id, brand, state);
        }
    }
}

/// Lower ranks win the project's face: running, waiting, failed, done.
fn face_rank(state: AgentState) -> u8 {
    match state {
        AgentState::Running => 0,
        AgentState::WaitingInput | AgentState::WaitingPermission => 1,
        AgentState::Failed => 2,
        AgentState::Completed => 3,
        _ => 4,
    }
}

impl Index {
    fn offer_face(&mut self, project: &str, brand: &'static str, state: AgentState) {
        if self
            .faces
            .get(project)
            .is_none_or(|(_, old)| face_rank(state) < face_rank(*old))
        {
            self.faces.insert(project.to_owned(), (brand, state));
        }
    }

    fn add_agents(&mut self, agents: &[Agent]) {
        for agent in agents {
            self.history_brands
                .entry((agent.session_id.clone(), agent.invocation_id.clone()))
                .or_insert_with(|| {
                    (
                        terminator_core::agents::icon_key(&agent.kind),
                        terminator_core::agents::display_name(&agent.kind).to_string(),
                    )
                });
            self.kinds
                .entry(agent.session_id.clone())
                .or_default()
                .push(agent.kind.clone());
        }
    }

    fn add_notifications(&mut self, notifications: &[Notification], now: u64) {
        for notice in notifications {
            if !notice.read && crate::sidebar_ui::notice_pending(notice, now) {
                self.unread_count = self.unread_count.saturating_add(1);
            }
            if !notice.dismissed
                && !notice.resolved
                && notice.snoozed_until <= now
                && self.sessions.contains_key(&notice.session_id)
            {
                self.pending.push(notice.clone());
            }
        }
        self.pending.sort_by_key(|n| {
            (
                n.resolved,
                notice_rank(n.state),
                std::cmp::Reverse(n.created),
            )
        });
        self.waiting_count = self
            .pending
            .iter()
            .filter(|n| crate::sidebar_ui::notice_waiting(n))
            .count();
        self.pending_groups = group_notices(self.pending.clone());
    }

    fn add_terminal_notices(&mut self, notices: &[TerminalNotice]) {
        use std::hash::{Hash, Hasher};
        for notice in notices {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            notice.id.hash(&mut hash);
            notice.title.hash(&mut hash);
            notice.body.hash(&mut hash);
            self.terminal_layout_keys
                .insert(notice.id.clone(), hash.finish());
            self.terminal_notes
                .insert(notice.session_id.clone(), notice.clone());
            if !notice.dismissed && self.sessions.contains_key(&notice.session_id) {
                self.terminal_pending.push(notice.clone());
            }
        }
        self.terminal_pending.reverse();
    }

    fn add_worktree_children(&mut self, state: &State) {
        let projects_by_id: HashMap<_, _> =
            state.projects.iter().map(|p| (p.id.as_str(), p)).collect();
        self.managed = state
            .worktrees
            .iter()
            .filter(|w| !w.removed)
            .map(|w| w.project_id.clone())
            .collect();
        for project in &state.projects {
            if self.managed.contains(&project.id) {
                continue;
            }
            let children: Vec<_> = state
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
                self.parents
                    .entry(child.id.clone())
                    .or_default()
                    .push(project.id.clone());
            }
            self.children.insert(project.id.clone(), children);
        }
    }
}
