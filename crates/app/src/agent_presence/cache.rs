use super::counts::{AttentionCounts, notice_pending};
use super::owner::OwnerSupport;
use super::presentation::AgentPresentation;
use super::queries::{build_presentation, notice_preview, unknown_presentation};
use terminator_core::{
    AGENT_PRESENCE_CAPABILITY, AgentState, Session, State,
    agents::{self},
};
/// Rebuilt on snapshot/local mutation, freshness changes, and snooze expiry.
/// Time alone does not rebuild stable presentations on every frame.
///
/// Indexes are built once per reconciliation. Unchanged sessions keep their
/// `Arc`. Lookup does not scan sessions, agents, or notifications.
#[derive(Default)]
pub(crate) struct PresentationCache {
    pub(super) entries: std::collections::HashMap<String, CacheEntry>,
    pub(super) attention: std::collections::HashMap<String, AttentionCounts>,
    pub(super) fresh: Option<bool>,
    pub(super) expires: u64,
    /// Address, length, and capacity of the collections that feed the index.
    /// Lookup compares this in constant time. A replaced snapshot cannot keep
    /// a warm empty cache. In-place edits still go through `reconcile`.
    pub(super) inputs: InputStamp,
    pub(super) ready: bool,
    pub(super) missing: Option<std::sync::Arc<AgentPresentation>>,
    pub(super) indexes_built: u64,
    pub(super) notifications_visited: u64,
    pub(super) presentations_rebuilt: u64,
    pub(super) presentations_reused: u64,
}

pub(super) struct CacheEntry {
    deps: PresentationDeps,
    presentation: std::sync::Arc<AgentPresentation>,
}

struct PresentationDeps {
    session: Session,
    support: OwnerSupport,
    presence: Option<agents::TerminalPresence>,
    hook: Option<terminator_core::Agent>,
    attention: AttentionCounts,
    unread: usize,
    fresh: Option<bool>,
    /// `fresh` when the connection reports it. Otherwise the observation is
    /// still inside the legacy staleness window. This is what makes a deadline
    /// replace the presentation instead of reusing a stale `verified` flag.
    presence_current: bool,
    notice_preview: Option<String>,
}

#[derive(Default)]
struct NoticeFold {
    attention: AttentionCounts,
    unread: usize,
    preview_at: u64,
    preview_index: Option<usize>,
}

impl PresentationCache {
    #[cfg(test)]
    pub fn clear(&mut self) {
        self.entries.clear();
        self.attention.clear();
        self.fresh = None;
        self.expires = 0;
        self.inputs = InputStamp::default();
        self.ready = false;
    }

    /// Rebuild indexes once and replace only the presentations whose inputs changed.
    pub fn reconcile(&mut self, state: &State, now: u64, fresh: Option<bool>) {
        self.rebuild(state, now, fresh);
    }

    pub fn get(
        &mut self,
        state: &State,
        id: &str,
        now: u64,
        fresh: Option<bool>,
    ) -> std::sync::Arc<AgentPresentation> {
        self.ensure(state, now, fresh);
        self.entries
            .get(id)
            .map(|entry| std::sync::Arc::clone(&entry.presentation))
            .unwrap_or_else(|| self.missing())
    }

    pub fn generation(&mut self, state: &State, now: u64, fresh: Option<bool>) -> u64 {
        self.ensure(state, now, fresh);
        self.indexes_built
    }

    /// Sum precomputed attention. Duplicate session ids count once.
    /// Ended sessions contribute nothing. Pending input, permission, and
    /// failure stay distinct.
    pub fn attention(
        &mut self,
        state: &State,
        session_ids: &[String],
        now: u64,
        fresh: Option<bool>,
    ) -> AttentionCounts {
        self.ensure(state, now, fresh);
        let mut seen = std::collections::HashSet::with_capacity(session_ids.len());
        let mut counts = AttentionCounts::default();
        for id in session_ids {
            if !seen.insert(id.as_str()) {
                continue;
            }
            if let Some(item) = self.attention.get(id.as_str()) {
                counts.input = counts.input.saturating_add(item.input);
                counts.permission = counts.permission.saturating_add(item.permission);
                counts.failed = counts.failed.saturating_add(item.failed);
            }
        }
        counts
    }

    fn ensure(&mut self, state: &State, now: u64, fresh: Option<bool>) {
        if !self.ready
            || self.fresh != fresh
            || now >= self.expires
            || self.inputs != InputStamp::from_state(state)
        {
            self.rebuild(state, now, fresh);
        }
    }

    fn missing(&mut self) -> std::sync::Arc<AgentPresentation> {
        std::sync::Arc::clone(
            self.missing
                .get_or_insert_with(|| std::sync::Arc::new(unknown_presentation())),
        )
    }

    fn rebuild(&mut self, state: &State, now: u64, fresh: Option<bool>) {
        self.indexes_built = self.indexes_built.saturating_add(1);
        self.notifications_visited = self
            .notifications_visited
            .saturating_add(state.notifications.len() as u64);
        let mut live = std::collections::HashSet::with_capacity(state.sessions.len());
        for session in &state.sessions {
            if session.lifecycle.live() {
                live.insert(session.id.as_str());
            }
        }
        let generations_empty = state.generations.is_empty();
        let legacy = OwnerSupport {
            supported: state
                .capabilities
                .iter()
                .any(|c| c == AGENT_PRESENCE_CAPABILITY),
            available: true,
        };
        let mut owners = std::collections::HashMap::with_capacity(state.generations.len());
        if !generations_empty {
            for health in &state.generations {
                owners
                    .entry(health.owner.id.as_str())
                    .or_insert(OwnerSupport {
                        supported: health
                            .capabilities
                            .iter()
                            .any(|c| c == AGENT_PRESENCE_CAPABILITY),
                        available: health.error.is_none(),
                    });
            }
        }
        let mut presence_by_id = std::collections::HashMap::with_capacity(state.presence.len());
        for item in &state.presence {
            presence_by_id
                .entry(item.session_id.as_str())
                .or_insert(item);
        }
        let mut grouped: std::collections::HashMap<&str, Vec<&terminator_core::Agent>> =
            std::collections::HashMap::new();
        for agent in &state.agents {
            grouped
                .entry(agent.session_id.as_str())
                .or_default()
                .push(agent);
        }
        let mut selected = std::collections::HashMap::with_capacity(grouped.len());
        for (id, group) in &grouped {
            if let Some(agent) = agents::select_matched_agent(group.iter().copied()) {
                selected.insert(*id, agent);
            }
        }
        let mut folds: std::collections::HashMap<&str, NoticeFold> =
            std::collections::HashMap::new();
        let mut expires = u64::MAX;
        for (index, notice) in state.notifications.iter().enumerate() {
            if notice.snoozed_until > now {
                expires = expires.min(notice.snoozed_until);
            }
            let fold = folds.entry(notice.session_id.as_str()).or_default();
            if live.contains(notice.session_id.as_str()) && notice_pending(notice, now) {
                match notice.state {
                    AgentState::WaitingInput => {
                        fold.attention.input = fold.attention.input.saturating_add(1);
                    }
                    AgentState::WaitingPermission => {
                        fold.attention.permission = fold.attention.permission.saturating_add(1);
                    }
                    AgentState::Failed => {
                        fold.attention.failed = fold.attention.failed.saturating_add(1);
                    }
                    _ => {}
                }
            }
            if !notice.read && notice_pending(notice, now) {
                fold.unread = fold.unread.saturating_add(1);
            }
            if !notice.dismissed
                && (fold.preview_index.is_none() || notice.created >= fold.preview_at)
            {
                fold.preview_at = notice.created;
                fold.preview_index = Some(index);
            }
        }
        if fresh.is_none() {
            for item in &state.presence {
                let at = item
                    .observed_at
                    .saturating_add(agents::STALE_AFTER_SECS + 1);
                if at > now {
                    expires = expires.min(at);
                }
            }
        }
        self.attention.clear();
        for (id, fold) in &folds {
            if !fold.attention.is_empty() {
                self.attention.insert((*id).to_string(), fold.attention);
            }
        }
        let mut seen = std::collections::HashSet::with_capacity(state.sessions.len());
        for session in &state.sessions {
            if !seen.insert(session.id.as_str()) {
                continue;
            }
            let support = if generations_empty {
                legacy
            } else {
                owners
                    .get(session.generation.as_str())
                    .copied()
                    .unwrap_or(OwnerSupport {
                        supported: false,
                        available: false,
                    })
            };
            let presence = presence_by_id.get(session.id.as_str()).copied().cloned();
            let hook = selected.get(session.id.as_str()).copied().cloned();
            let fold = folds.get(session.id.as_str());
            let attention = fold.map(|item| item.attention).unwrap_or_default();
            let unread = fold.map(|item| item.unread).unwrap_or(0);
            let presence_current =
                fresh.unwrap_or_else(|| agents::presence_verified(presence.as_ref(), now));
            let notice_preview = hook.as_ref().and_then(|_| {
                fold.and_then(|item| item.preview_index).and_then(|index| {
                    let notice = state.notifications.get(index)?;
                    let text = notice_preview(&notice.summary);
                    (!text.is_empty() && text != notice.state.label()).then_some(text)
                })
            });
            let (unchanged, presentation) = {
                let view = SessionView {
                    session,
                    support,
                    presence: presence.as_ref(),
                    hook: hook.as_ref(),
                    attention,
                    unread,
                    fresh,
                    presence_current,
                    notice_preview: &notice_preview,
                };
                let unchanged = self
                    .entries
                    .get(&session.id)
                    .is_some_and(|entry| entry.deps.matches(&view));
                let presentation =
                    (!unchanged).then(|| std::sync::Arc::new(build_presentation(&view, now)));
                (unchanged, presentation)
            };
            if unchanged {
                self.presentations_reused = self.presentations_reused.saturating_add(1);
                continue;
            }
            let Some(presentation) = presentation else {
                continue;
            };
            self.entries.insert(
                session.id.clone(),
                CacheEntry {
                    deps: PresentationDeps {
                        session: session.clone(),
                        support,
                        presence,
                        hook,
                        attention,
                        unread,
                        fresh,
                        presence_current,
                        notice_preview,
                    },
                    presentation,
                },
            );
            self.presentations_rebuilt = self.presentations_rebuilt.saturating_add(1);
        }
        self.entries.retain(|id, _| seen.contains(id.as_str()));
        self.fresh = fresh;
        self.expires = expires;
        self.inputs = InputStamp::from_state(state);
        self.ready = true;
    }
}

/// Constant-time identity of one `Vec`: allocation, length, and capacity.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct CollectionId {
    address: usize,
    len: usize,
    capacity: usize,
}

impl CollectionId {
    fn new(address: usize, len: usize, capacity: usize) -> Self {
        Self {
            address,
            len,
            capacity,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct InputStamp {
    sessions: CollectionId,
    agents: CollectionId,
    notifications: CollectionId,
    presence: CollectionId,
    generations: CollectionId,
    capabilities: CollectionId,
}

impl InputStamp {
    fn from_state(state: &State) -> Self {
        Self {
            sessions: CollectionId::new(
                state.sessions.as_ptr().addr(),
                state.sessions.len(),
                state.sessions.capacity(),
            ),
            agents: CollectionId::new(
                state.agents.as_ptr().addr(),
                state.agents.len(),
                state.agents.capacity(),
            ),
            notifications: CollectionId::new(
                state.notifications.as_ptr().addr(),
                state.notifications.len(),
                state.notifications.capacity(),
            ),
            presence: CollectionId::new(
                state.presence.as_ptr().addr(),
                state.presence.len(),
                state.presence.capacity(),
            ),
            generations: CollectionId::new(
                state.generations.as_ptr().addr(),
                state.generations.len(),
                state.generations.capacity(),
            ),
            capabilities: CollectionId::new(
                state.capabilities.as_ptr().addr(),
                state.capabilities.len(),
                state.capabilities.capacity(),
            ),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct SessionView<'a> {
    pub(super) session: &'a Session,
    pub(super) support: OwnerSupport,
    pub(super) presence: Option<&'a agents::TerminalPresence>,
    pub(super) hook: Option<&'a terminator_core::Agent>,
    pub(super) attention: AttentionCounts,
    pub(super) unread: usize,
    pub(super) fresh: Option<bool>,
    pub(super) presence_current: bool,
    pub(super) notice_preview: &'a Option<String>,
}

impl PresentationDeps {
    fn matches(&self, view: &SessionView<'_>) -> bool {
        &self.session == view.session
            && self.support == view.support
            && self.presence.as_ref() == view.presence
            && self.hook.as_ref() == view.hook
            && self.attention == view.attention
            && self.unread == view.unread
            && self.fresh == view.fresh
            && self.presence_current == view.presence_current
            && &self.notice_preview == view.notice_preview
    }
}

pub(super) fn ago(timestamp: u64, now: u64) -> String {
    let age = now.saturating_sub(timestamp);
    if age < 90 {
        format!("{age}s ago")
    } else if age < 5400 {
        format!("{}m ago", age / 60)
    } else {
        format!("{}h ago", age / 3600)
    }
}
