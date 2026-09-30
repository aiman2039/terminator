//! Shared agent identity/status presentation. One model backs workspace tabs,
//! terminal-strip tabs, split-pane headers, session rows, and agent rows so
//! identity, lifecycle, unread state, and diagnostics render consistently.
//!
//! Lifecycle status always comes from hook records. A verified live process
//! without hook state shows "Status unavailable", never "Working" or "Idle".
use terminator_core::{
    AGENT_PRESENCE_CAPABILITY, AgentState, Session, State,
    agents::{self, PresenceOutcome},
};

/// Status glyph per lifecycle state. The brand icon beside it identifies the
/// agent; this glyph only reports hook lifecycle.
#[must_use]
pub fn attention_status_icon(state: AgentState) -> &'static str {
    match state {
        AgentState::WaitingInput => "MessageCircleQuestion",
        AgentState::WaitingPermission => "ShieldQuestion",
        AgentState::Completed => "CircleCheck",
        AgentState::Failed => "CircleAlert",
        AgentState::Running => "LoaderCircle",
        AgentState::Stopped => "CircleStop",
        AgentState::Unknown => "CircleQuestion",
    }
}

/// Pending-attention breakdown. Input requests, permission requests, and
/// failures stay distinct in workspace-tab aggregates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttentionCounts {
    pub input: usize,
    pub permission: usize,
    pub failed: usize,
}

impl AttentionCounts {
    #[must_use]
    pub fn waiting(self) -> usize {
        self.input + self.permission
    }
    #[must_use]
    pub fn total(self) -> usize {
        self.input + self.permission + self.failed
    }
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.total() == 0
    }
}

#[must_use]
pub fn notice_pending(notice: &terminator_core::Notification, now: u64) -> bool {
    !notice.dismissed && !notice.resolved && notice.snoozed_until <= now
}

/// Spec for one tab's attention. The GUI uses [`PresentationCache::attention`].
#[cfg(test)]
#[must_use]
pub fn tab_attention(state: &State, session_ids: &[String], now: u64) -> AttentionCounts {
    let live: std::collections::HashSet<_> = state
        .sessions
        .iter()
        .filter(|s| s.lifecycle.live())
        .map(|s| s.id.as_str())
        .collect();
    let mut counts = AttentionCounts::default();
    for notice in &state.notifications {
        if !live.contains(notice.session_id.as_str())
            || !session_ids.contains(&notice.session_id)
            || !notice_pending(notice, now)
        {
            continue;
        }
        match notice.state {
            AgentState::WaitingInput => counts.input += 1,
            AgentState::WaitingPermission => counts.permission += 1,
            AgentState::Failed => counts.failed += 1,
            _ => {}
        }
    }
    counts
}

/// Per-owner presence support for a session. Single-daemon states carry
/// capabilities on the snapshot; multi-owner states carry them per owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerSupport {
    pub supported: bool,
    pub available: bool,
}

#[cfg(test)]
#[must_use]
pub fn owner_support(state: &State, session: &Session) -> OwnerSupport {
    match state
        .generations
        .iter()
        .find(|health| health.owner.id == session.generation)
    {
        Some(health) => OwnerSupport {
            supported: health
                .capabilities
                .iter()
                .any(|c| c == AGENT_PRESENCE_CAPABILITY),
            available: health.error.is_none(),
        },
        None if state.generations.is_empty() => OwnerSupport {
            supported: state
                .capabilities
                .iter()
                .any(|c| c == AGENT_PRESENCE_CAPABILITY),
            available: true,
        },
        None => OwnerSupport {
            supported: false,
            available: false,
        },
    }
}

/// Complete presentation for one terminal session.
#[derive(Clone, Debug)]
pub struct AgentPresentation {
    /// Stable brand glyph, e.g. `AgentCodex` or `AgentMultiple`.
    pub brand_icon: Option<&'static str>,
    /// Brand text, e.g. `Codex` or `3 agents`.
    pub brand_label: Option<String>,
    /// Verified live agent kinds for this terminal.
    pub detected_kinds: Vec<String>,
    /// A fresh verified observation exists (even when it reports no agents).
    pub verified: bool,
    /// True when at least one agent process is verified live.
    pub live: bool,
    /// Hook lifecycle, when any hook event was recorded.
    pub lifecycle: Option<AgentState>,
    /// Lifecycle label, or "Status unavailable" when unknown.
    pub status_label: String,
    pub status_icon: &'static str,
    pub spin: bool,
    pub unread: usize,
    pub attention: AttentionCounts,
    /// Latest non-dismissed notice sentence. Absent when there is no hook
    /// lifecycle, or when the text only repeats the state label.
    pub notice_preview: Option<String>,
    /// Deferred tooltip inputs. Never contain command arguments.
    diagnostics: Option<Diagnostics>,
}

#[derive(Clone, Debug)]
struct Diagnostics {
    session: Session,
    presence: Option<agents::TerminalPresence>,
    detected: Vec<agents::DetectedAgent>,
    hook: Option<terminator_core::Agent>,
    support: OwnerSupport,
    hook_linked: bool,
}

impl AgentPresentation {
    pub fn diagnostics(&self, now: u64) -> String {
        let Some(d) = &self.diagnostics else {
            return "Unknown session".into();
        };
        diagnostics_text(
            &d.session,
            d.presence.as_ref(),
            &d.detected,
            d.hook.as_ref(),
            d.support,
            self.verified,
            d.hook_linked,
            &self.status_label,
            self.unread,
            now,
        )
    }
}

/// Rebuilt on snapshot/local mutation, freshness changes, and snooze expiry.
/// Time alone does not rebuild stable presentations on every frame.
///
/// Indexes are built once per reconciliation. Unchanged sessions keep their
/// `Arc`. Lookup does not scan sessions, agents, or notifications.
#[derive(Default)]
pub struct PresentationCache {
    entries: std::collections::HashMap<String, CacheEntry>,
    attention: std::collections::HashMap<String, AttentionCounts>,
    fresh: Option<bool>,
    expires: u64,
    /// Address, length, and capacity of the collections that feed the index.
    /// Lookup compares this in constant time. A replaced snapshot cannot keep
    /// a warm empty cache. In-place edits still go through `reconcile`.
    inputs: InputStamp,
    ready: bool,
    missing: Option<std::sync::Arc<AgentPresentation>>,
    indexes_built: u64,
    notifications_visited: u64,
    presentations_rebuilt: u64,
    presentations_reused: u64,
}

struct CacheEntry {
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
            .map(|entry| entry.presentation.clone())
            .unwrap_or_else(|| self.missing())
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
                counts.input += item.input;
                counts.permission += item.permission;
                counts.failed += item.failed;
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
        self.missing
            .get_or_insert_with(|| std::sync::Arc::new(unknown_presentation()))
            .clone()
    }

    fn rebuild(&mut self, state: &State, now: u64, fresh: Option<bool>) {
        self.indexes_built += 1;
        self.notifications_visited += state.notifications.len() as u64;
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
                    AgentState::WaitingInput => fold.attention.input += 1,
                    AgentState::WaitingPermission => fold.attention.permission += 1,
                    AgentState::Failed => fold.attention.failed += 1,
                    _ => {}
                }
            }
            if !notice.read && notice_pending(notice, now) {
                fold.unread += 1;
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
                    let notice = &state.notifications[index];
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
                self.presentations_reused += 1;
                continue;
            }
            let presentation = presentation.expect("changed session builds a presentation");
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
            self.presentations_rebuilt += 1;
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
struct InputStamp {
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
struct SessionView<'a> {
    session: &'a Session,
    support: OwnerSupport,
    presence: Option<&'a agents::TerminalPresence>,
    hook: Option<&'a terminator_core::Agent>,
    attention: AttentionCounts,
    unread: usize,
    fresh: Option<bool>,
    presence_current: bool,
    notice_preview: &'a Option<String>,
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

fn ago(timestamp: u64, now: u64) -> String {
    let age = now.saturating_sub(timestamp);
    if age < 90 {
        format!("{age}s ago")
    } else if age < 5400 {
        format!("{}m ago", age / 60)
    } else {
        format!("{}h ago", age / 3600)
    }
}

/// Present one terminal session. Callers must pass the current time; the
/// model never reads the clock itself so tests stay deterministic.
#[cfg(test)]
#[must_use]
pub fn present_session(state: &State, session_id: &str, now: u64) -> AgentPresentation {
    let mut cache = PresentationCache::default();
    (*cache.get(state, session_id, now, None)).clone()
}

fn unknown_presentation() -> AgentPresentation {
    AgentPresentation {
        brand_icon: None,
        brand_label: None,
        detected_kinds: Vec::new(),
        verified: false,
        live: false,
        lifecycle: None,
        status_label: "Status unavailable".into(),
        status_icon: attention_status_icon(AgentState::Unknown),
        spin: false,
        unread: 0,
        attention: AttentionCounts::default(),
        notice_preview: None,
        diagnostics: None,
    }
}

fn build_presentation(view: &SessionView<'_>, now: u64) -> AgentPresentation {
    let SessionView {
        session,
        support,
        presence,
        hook,
        attention,
        unread,
        fresh,
        notice_preview,
        ..
    } = *view;
    let verified = session.lifecycle.live()
        && support.supported
        && support.available
        && presence.is_some_and(|item| item.outcome == PresenceOutcome::Verified)
        && fresh.unwrap_or_else(|| agents::presence_verified(presence, now));
    let detected: Vec<_> = if verified {
        presence.map(|item| item.agents.clone()).unwrap_or_default()
    } else {
        Vec::new()
    };
    let detected_kinds: Vec<String> = detected.iter().map(|agent| agent.kind.clone()).collect();
    // A fresh verified-empty observation means the agent exited and the
    // terminal is a plain shell again. Hook history stays for the inbox,
    // but terminal chrome must not keep the brand or lifecycle status.
    let plain_shell = verified && detected.is_empty();
    let (brand_icon, brand_label) = if !detected.is_empty() {
        match agents::preferred_agent(&detected) {
            Some(agent) => (
                Some(agents::icon_key(&agent.kind)),
                Some(agents::display_name(&agent.kind).to_string()),
            ),
            None => (
                Some(agents::MULTIPLE_ICON),
                Some(format!("{} agents", detected.len())),
            ),
        }
    } else if let Some(hook) = hook
        && !plain_shell
    {
        (
            Some(agents::icon_key(&hook.kind)),
            Some(agents::display_name(&hook.kind).to_string()),
        )
    } else {
        (None, None)
    };
    let lifecycle = if plain_shell {
        None
    } else {
        hook.map(|agent| agent.state)
    };
    let status_label = match lifecycle {
        Some(status) if status != AgentState::Unknown => status.label().to_string(),
        _ => "Status unavailable".to_string(),
    };
    let status_icon = attention_status_icon(lifecycle.unwrap_or(AgentState::Unknown));
    let hook_linked = match (hook, hook.and_then(|agent| agent.process.as_ref())) {
        (Some(current), Some(identity)) => detected
            .iter()
            .any(|agent| agent.kind == current.kind && agent.process == *identity),
        _ => false,
    };
    AgentPresentation {
        brand_icon,
        brand_label,
        detected_kinds,
        verified,
        live: !detected.is_empty(),
        lifecycle,
        status_label,
        status_icon,
        spin: lifecycle == Some(AgentState::Running),
        unread,
        attention,
        notice_preview: notice_preview.clone(),
        diagnostics: Some(Diagnostics {
            session: session.clone(),
            presence: presence.cloned(),
            detected,
            hook: hook.cloned(),
            support,
            hook_linked,
        }),
    }
}

/// Plain-text sentence from a hook summary. Markdown markers are dropped.
pub(crate) fn notice_preview(markdown: &str) -> String {
    use pulldown_cmark::{Event, Parser, TagEnd};
    let mut text = String::new();
    for event in Parser::new(markdown) {
        match event {
            Event::Text(value) | Event::Code(value) => text.push_str(&value),
            Event::SoftBreak
            | Event::HardBreak
            | Event::End(
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item | TagEnd::CodeBlock,
            ) => text.push(' '),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[allow(clippy::too_many_arguments)]
fn diagnostics_text(
    session: &Session,
    presence: Option<&terminator_core::agents::TerminalPresence>,
    detected: &[terminator_core::agents::DetectedAgent],
    hook: Option<&terminator_core::Agent>,
    support: OwnerSupport,
    verified: bool,
    hook_linked: bool,
    status_label: &str,
    unread: usize,
    now: u64,
) -> String {
    let mut lines = Vec::new();
    if !session.lifecycle.live() {
        lines.push("Session ended".into());
    } else if detected.is_empty() {
        if verified {
            lines.push(format!(
                "No live agent detected (verified {})",
                ago(presence.map(|p| p.observed_at).unwrap_or(now), now)
            ));
        } else if !support.available {
            lines.push("Presence unverified: session owner unavailable".into());
        } else if !support.supported {
            lines.push("Presence unverified: unsupported daemon".into());
        } else if presence.is_some_and(|p| p.outcome == PresenceOutcome::Unavailable) {
            lines.push("Presence unverified: detection unavailable".into());
        } else if let Some(observed) = presence.map(|p| p.observed_at) {
            lines.push(format!(
                "Presence unverified: stale observation ({})",
                ago(observed, now)
            ));
        } else {
            lines.push("Presence unverified: no observation yet".into());
        }
    } else {
        let names: Vec<_> = detected
            .iter()
            .map(|a| agents::display_name(&a.kind))
            .collect();
        let age = ago(presence.map(|p| p.observed_at).unwrap_or(now), now);
        if detected.len() == 1 && agents::preferred_agent(detected).is_some() {
            let how = if detected[0].foreground {
                "foreground"
            } else {
                "only agent"
            };
            lines.push(format!("{} live (verified {age}; {how})", names[0]));
        } else {
            lines.push(format!("{} live (verified {age})", names.join(", ")));
        }
    }
    lines.push(if detected.is_empty() {
        match hook {
            Some(_) if !verified => "Identity source: hook event".into(),
            _ => "Identity source: none".into(),
        }
    } else {
        "Identity source: process inspection".into()
    });
    match hook {
        Some(h) => lines.push(format!(
            "Hook ({}): {} · {} · {}",
            agents::display_name(&h.kind),
            h.state.label(),
            if hook_linked {
                "linked to live process"
            } else {
                "last reported"
            },
            ago(h.updated, now),
        )),
        None => lines.push("Hook: none reported".into()),
    }
    lines.push(format!("Status: {status_label}"));
    if unread > 0 {
        lines.push(format!("Unread: {unread}"));
    }
    lines.push(session.cwd.display().to_string());
    lines.join("\n")
}

/// Sessions with verified live agents, for the All-live view.
#[cfg(test)]
#[must_use]
pub fn live_sessions(state: &State, now: u64) -> Vec<(&Session, Vec<String>)> {
    state
        .sessions
        .iter()
        .filter(|s| s.lifecycle.live())
        .filter_map(|s| {
            let presentation = present_session(state, &s.id, now);
            if presentation.live {
                Some((s, presentation.detected_kinds))
            } else {
                None
            }
        })
        .collect()
}

/// Live sessions with hook records but no verified live agent. Covers older
/// daemons, unavailable owners, stale observations, and detection failures.
/// A verified-empty observation means the agent exited: the terminal is a
/// plain shell and is excluded.
#[cfg(test)]
#[must_use]
pub fn unverified_sessions(state: &State, now: u64) -> Vec<&Session> {
    state
        .sessions
        .iter()
        .filter(|s| {
            if !s.lifecycle.live() {
                return false;
            }
            let presentation = present_session(state, &s.id, now);
            !presentation.live && !presentation.verified && presentation.lifecycle.is_some()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminator_core::{
        Agent, Lifecycle, Notification, SessionKind, agents::ProcessIdentity, now,
    };

    fn session(id: &str) -> Session {
        Session {
            review: false,
            id: id.into(),
            project_id: "p".into(),
            label: id.into(),
            cwd: "/tmp".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle: Lifecycle::Running,
            created: 1,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "g".into(),
            pid: Some(100),
            truncated: false,
            cwd_confirmed: true,
        }
    }

    fn capable(state: &mut State) {
        state.capabilities.push(AGENT_PRESENCE_CAPABILITY.into());
    }

    fn observe(state: &mut State, kinds: &[&str], at: u64) {
        state.presence.push(agents::TerminalPresence {
            session_id: "s".into(),
            generation: "g".into(),
            agents: kinds
                .iter()
                .enumerate()
                .map(|(i, kind)| agents::DetectedAgent {
                    kind: (*kind).into(),
                    process: ProcessIdentity {
                        pid: 200 + i as u32,
                        start_time: 500,
                    },
                    foreground: i == 0,
                })
                .collect(),
            outcome: PresenceOutcome::Verified,
            observed_at: at,
        });
    }

    fn hook(state: &mut State, kind: &str, lifecycle: AgentState, linked: bool) {
        state.agents.push(Agent {
            invocation_id: format!("agent-{kind}"),
            session_id: "s".into(),
            kind: kind.into(),
            provider_session_id: None,
            state: lifecycle,
            sequence: None,
            updated: now() - 3,
            resume: None,
            process: linked.then_some(ProcessIdentity {
                pid: 200,
                start_time: 500,
            }),
        });
    }

    #[test]
    fn cache_reuses_presentations_and_expires_snoozes_and_snapshot_health() {
        let mut state = State::default();
        state.capabilities.push(AGENT_PRESENCE_CAPABILITY.into());
        state.sessions.push(session("s"));
        state.presence.push(agents::TerminalPresence {
            session_id: "s".into(),
            generation: "g".into(),
            agents: vec![],
            outcome: PresenceOutcome::Verified,
            observed_at: 1,
        });
        let mut cache = PresentationCache::default();
        let first = cache.get(&state, "s", 100, Some(true));
        assert!(
            first.verified,
            "successful unchanged snapshots keep presence fresh"
        );
        assert!(std::sync::Arc::ptr_eq(
            &first,
            &cache.get(&state, "s", 200, Some(true))
        ));
        assert!(!cache.get(&state, "s", 200, Some(false)).verified);
        assert!(cache.get(&state, "s", 201, Some(true)).verified);
        // A responsive daemon can explicitly expire a stalled inspector's data.
        state.presence[0].outcome = PresenceOutcome::Unavailable;
        cache.clear();
        assert!(!cache.get(&state, "s", 202, Some(true)).verified);
        state.presence[0].outcome = PresenceOutcome::Verified;
        state.presence[0].observed_at = 203;
        cache.clear();
        assert!(cache.get(&state, "s", 203, Some(true)).verified);
        state.notifications.push(Notification {
            id: "n".into(),
            session_id: "s".into(),
            invocation_id: "i".into(),
            request_id: None,
            state: AgentState::WaitingInput,
            summary: String::new(),
            details: String::new(),
            created: 100,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 300,
        });
        cache.clear();
        assert_eq!(cache.get(&state, "s", 299, Some(true)).attention.input, 0);
        assert_eq!(cache.get(&state, "s", 300, Some(true)).attention.input, 1);
        state.sessions[0].lifecycle = Lifecycle::Ended;
        cache.clear();
        let ended = cache.get(&state, "s", 301, Some(true));
        assert!(!ended.verified);
        assert!(ended.attention.is_empty());
    }

    #[test]
    fn verified_detection_pairs_brand_with_hook_status() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        observe(&mut state, &["codex"], now());
        hook(&mut state, "codex", AgentState::Running, true);
        let presented = present_session(&state, "s", now());
        assert_eq!(presented.brand_icon, Some("AgentCodex"));
        assert_eq!(presented.brand_label.as_deref(), Some("Codex"));
        assert!(presented.verified && presented.live);
        assert_eq!(presented.status_label, "Working");
        assert_eq!(presented.status_icon, "LoaderCircle");
        assert!(presented.spin);
        assert!(presented.diagnostics(now()).contains("process inspection"));
        assert!(
            presented
                .diagnostics(now())
                .contains("linked to live process")
        );
    }

    #[test]
    fn verified_presence_without_hooks_never_claims_working_or_idle() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        observe(&mut state, &["pi"], now());
        let presented = present_session(&state, "s", now());
        assert!(presented.live);
        assert_eq!(presented.brand_label.as_deref(), Some("Pi"));
        assert_eq!(presented.status_label, "Status unavailable");
        assert!(!presented.spin);
        assert!(presented.diagnostics(now()).contains("Hook: none reported"));
    }

    #[test]
    fn hook_only_state_on_old_daemons_is_labeled_unverified() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        hook(&mut state, "my-agent", AgentState::WaitingInput, false);
        let presented = present_session(&state, "s", now());
        assert!(!presented.verified && !presented.live);
        assert_eq!(presented.brand_icon, Some(agents::GENERIC_ICON));
        assert_eq!(presented.status_label, "Needs input");
        assert!(presented.diagnostics(now()).contains("unsupported daemon"));
        assert!(presented.diagnostics(now()).contains("last reported"));
    }

    #[test]
    fn multiple_agents_without_foreground_use_the_count_indicator() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        state.presence.push(agents::TerminalPresence {
            session_id: "s".into(),
            generation: "g".into(),
            agents: ["codex", "claude"]
                .into_iter()
                .enumerate()
                .map(|(i, kind)| agents::DetectedAgent {
                    kind: kind.into(),
                    process: ProcessIdentity {
                        pid: 200 + i as u32,
                        start_time: 500,
                    },
                    foreground: false,
                })
                .collect(),
            outcome: PresenceOutcome::Verified,
            observed_at: now(),
        });
        let presented = present_session(&state, "s", now());
        assert!(presented.live);
        assert_eq!(presented.brand_icon, Some(agents::MULTIPLE_ICON));
        assert_eq!(presented.brand_label.as_deref(), Some("2 agents"));
        assert!(
            presented.diagnostics(now()).contains("Codex")
                && presented.diagnostics(now()).contains("Claude")
        );
    }

    #[test]
    fn stale_failed_and_unavailable_observations_stay_unverified() {
        for (outcome, age, needle) in [
            (PresenceOutcome::Verified, 30, "stale observation"),
            (PresenceOutcome::Unavailable, 0, "detection unavailable"),
        ] {
            let mut state = State::default();
            state.sessions.push(session("s"));
            capable(&mut state);
            state.presence.push(agents::TerminalPresence {
                session_id: "s".into(),
                generation: "g".into(),
                agents: vec![agents::DetectedAgent {
                    kind: "codex".into(),
                    process: ProcessIdentity {
                        pid: 200,
                        start_time: 500,
                    },
                    foreground: true,
                }],
                outcome,
                observed_at: now().saturating_sub(age),
            });
            hook(&mut state, "codex", AgentState::Running, true);
            let presented = present_session(&state, "s", now());
            assert!(!presented.verified && !presented.live, "{needle}");
            assert!(
                presented.diagnostics(now()).contains("last reported")
                    || presented.diagnostics(now()).contains("Hook: none reported"),
                "{}",
                presented.diagnostics(now())
            );
            assert!(
                presented.diagnostics(now()).contains(needle),
                "{}",
                presented.diagnostics(now())
            );
        }
    }

    #[test]
    fn muse_subagent_tool_after_stop_shows_completed() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        state.agents.push(Agent {
            invocation_id: "lead".into(),
            session_id: "s".into(),
            kind: "muse".into(),
            provider_session_id: Some("01a0e846-a59d-7da0-a7c8-3de6bf83174b".into()),
            state: AgentState::Completed,
            sequence: None,
            updated: 10,
            resume: None,
            process: None,
        });
        state.agents.push(Agent {
            invocation_id: "tool".into(),
            session_id: "s".into(),
            kind: "muse".into(),
            provider_session_id: Some("ee1080b7-a350-4c2b-844b-ae1786b1b3c0".into()),
            state: AgentState::Running,
            sequence: None,
            updated: 50,
            resume: None,
            process: None,
        });
        let presented = present_session(&state, "s", 100);
        assert_eq!(presented.status_label, "Completed");
        assert!(!presented.spin);
    }

    #[test]
    fn unknown_lifecycle_shows_status_unavailable() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        hook(&mut state, "codex", AgentState::Unknown, false);
        let presented = present_session(&state, "s", now());
        assert_eq!(presented.status_label, "Status unavailable");
        assert_eq!(presented.status_icon, "CircleQuestion");
    }

    #[test]
    fn attention_counts_keep_input_permission_and_failure_distinct() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        for (i, lifecycle) in [
            AgentState::WaitingInput,
            AgentState::WaitingInput,
            AgentState::WaitingPermission,
            AgentState::Failed,
            AgentState::Completed,
        ]
        .into_iter()
        .enumerate()
        {
            state.notifications.push(Notification {
                id: format!("n{i}"),
                session_id: "s".into(),
                invocation_id: "a".into(),
                request_id: None,
                state: lifecycle,
                summary: String::new(),
                details: String::new(),
                created: i as u64,
                read: false,
                dismissed: false,
                resolved: false,
                snoozed_until: 0,
            });
        }
        // Dismissed, resolved, and snoozed notices never count.
        state.notifications[4].resolved = true;
        state.notifications.push(Notification {
            id: "snoozed".into(),
            session_id: "s".into(),
            invocation_id: "a".into(),
            request_id: None,
            state: AgentState::WaitingInput,
            summary: String::new(),
            details: String::new(),
            created: 9,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: now() + 600,
        });
        let counts = tab_attention(&state, &["s".into(), "other".into()], now());
        assert_eq!(
            counts,
            AttentionCounts {
                input: 2,
                permission: 1,
                failed: 1,
            }
        );
        assert_eq!(present_session(&state, "s", now()).unread, 4);
        assert!(tab_attention(&state, &["other".into()], now()).is_empty());
    }

    #[test]
    fn live_and_unverified_session_lists_partition_hook_activity() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        let mut ended = session("ended");
        ended.lifecycle = Lifecycle::Ended;
        state.sessions.push(ended);
        state.sessions.push(session("plain"));
        capable(&mut state);
        observe(&mut state, &["codex"], now());
        hook(&mut state, "codex", AgentState::Running, true);
        state.agents.push(Agent {
            invocation_id: "hook-only".into(),
            session_id: "plain".into(),
            kind: "claude".into(),
            provider_session_id: None,
            state: AgentState::WaitingInput,
            sequence: None,
            updated: now(),
            resume: None,
            process: None,
        });
        let live = live_sessions(&state, now());
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].0.id, "s");
        let unverified = unverified_sessions(&state, now());
        assert_eq!(unverified.len(), 1);
        assert_eq!(unverified[0].id, "plain");
    }

    fn notice(id: &str, session: &str, lifecycle: AgentState, created: u64) -> Notification {
        Notification {
            id: id.into(),
            session_id: session.into(),
            invocation_id: "a".into(),
            request_id: None,
            state: lifecycle,
            summary: format!("note {id}"),
            details: String::new(),
            created,
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 0,
        }
    }

    #[test]
    fn indexed_reconcile_scans_notifications_once_and_reuses_arcs() {
        let mut state = State::default();
        let sessions = 8usize;
        let notices = 20usize;
        for index in 0..sessions {
            state.sessions.push(session(&format!("s{index}")));
        }
        for index in 0..notices {
            state.notifications.push(notice(
                &format!("n{index}"),
                &format!("s{}", index % sessions),
                AgentState::WaitingInput,
                index as u64,
            ));
        }
        let mut ended = session("ended");
        ended.lifecycle = Lifecycle::Ended;
        state.sessions.push(ended);
        state.notifications.push(notice(
            "ended-note",
            "ended",
            AgentState::WaitingPermission,
            100,
        ));
        let mut cache = PresentationCache::default();
        cache.reconcile(&state, 50, Some(true));
        assert_eq!(cache.indexes_built, 1);
        assert_eq!(cache.notifications_visited, (notices + 1) as u64);
        assert_eq!(cache.presentations_rebuilt, (sessions + 1) as u64);
        let kept = cache.get(&state, "s0", 50, Some(true));
        let stable = cache.get(&state, "s1", 50, Some(true));
        for index in 0..sessions {
            let _ = cache.get(&state, &format!("s{index}"), 50, Some(true));
        }
        assert_eq!(cache.indexes_built, 1);
        assert_eq!(cache.notifications_visited, (notices + 1) as u64);
        let ids = vec!["s0".into(), "s0".into(), "s1".into(), "ended".into()];
        assert_eq!(
            cache.attention(&state, &ids, 50, Some(true)),
            tab_attention(&state, &ids, 50)
        );
        assert!(cache.attention(&state, &ids, 50, Some(true)).input > 0);
        assert_eq!(cache.indexes_built, 1);
        state.projects.push(terminator_core::Project {
            id: "p".into(),
            name: "One".into(),
            path: "/tmp".into(),
            layout: serde_json::json!({}),
        });
        state.sessions[0].label = "renamed".into();
        cache.reconcile(&state, 50, Some(true));
        assert!(!std::sync::Arc::ptr_eq(
            &kept,
            &cache.get(&state, "s0", 50, Some(true))
        ));
        assert!(std::sync::Arc::ptr_eq(
            &stable,
            &cache.get(&state, "s1", 50, Some(true))
        ));
        let other = cache.get(&state, "s1", 50, Some(true));
        state.sessions.retain(|item| item.id != "s0");
        cache.reconcile(&state, 50, Some(true));
        assert_eq!(
            cache.get(&state, "s0", 50, Some(true)).diagnostics(50),
            "Unknown session"
        );
        assert!(std::sync::Arc::ptr_eq(
            &other,
            &cache.get(&state, "s1", 50, Some(true))
        ));
        assert_eq!(cache.notifications_visited, (notices + 1) as u64 * 3);
    }

    #[test]
    fn one_session_change_leaves_unrelated_arcs_in_place() {
        let mut state = State::default();
        state.sessions.push(session("a"));
        state.sessions.push(session("b"));
        capable(&mut state);
        state.presence.push(agents::TerminalPresence {
            session_id: "a".into(),
            generation: "g".into(),
            agents: vec![],
            outcome: PresenceOutcome::Verified,
            observed_at: 1,
        });
        state
            .notifications
            .push(notice("na", "a", AgentState::Failed, 1));
        state
            .notifications
            .push(notice("nb", "b", AgentState::WaitingInput, 2));
        let mut cache = PresentationCache::default();
        cache.reconcile(&state, 10, Some(true));
        let left = cache.get(&state, "a", 10, Some(true));
        let right = cache.get(&state, "b", 10, Some(true));
        state.presence[0].observed_at = 9;
        state.notifications[0].read = true;
        cache.reconcile(&state, 10, Some(true));
        let next = cache.get(&state, "a", 10, Some(true));
        assert!(!std::sync::Arc::ptr_eq(&left, &next));
        assert!(next.verified);
        assert_eq!(next.unread, 0);
        assert_eq!(next.attention.failed, 1);
        assert!(std::sync::Arc::ptr_eq(
            &right,
            &cache.get(&state, "b", 10, Some(true))
        ));
        assert_eq!(cache.presentations_rebuilt, 3);
        assert_eq!(cache.presentations_reused, 1);
    }

    #[test]
    fn changed_generation_drops_stale_verification() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        state
            .generations
            .push(terminator_core::generations::Health {
                owner: terminator_core::generations::Generation {
                    id: "g".into(),
                    data: "/tmp".into(),
                    runtime: "/tmp".into(),
                    version: "0".into(),
                    build: "0".into(),
                    protocol: 1,
                    catalog: 1,
                    status: terminator_core::generations::Status::Active,
                    pid: None,
                },
                revision: 1,
                live_sessions: 1,
                error: None,
                capabilities: vec![AGENT_PRESENCE_CAPABILITY.into()],
                helper: None,
            });
        observe(&mut state, &["codex"], 10);
        let mut cache = PresentationCache::default();
        cache.reconcile(&state, 10, Some(true));
        let first = cache.get(&state, "s", 10, Some(true));
        assert!(first.verified);
        assert_eq!(
            owner_support(&state, &state.sessions[0]),
            OwnerSupport {
                supported: true,
                available: true,
            }
        );
        state.sessions[0].generation = "missing".into();
        cache.reconcile(&state, 10, Some(true));
        let next = cache.get(&state, "s", 10, Some(true));
        assert!(!std::sync::Arc::ptr_eq(&first, &next));
        assert!(!next.verified && !next.live);
        assert_eq!(
            owner_support(&state, &state.sessions[0]),
            OwnerSupport {
                supported: false,
                available: false,
            }
        );
    }

    #[test]
    fn fresh_connection_ignores_observation_age_until_the_timestamp_changes() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        observe(&mut state, &["codex"], 100);
        let mut cache = PresentationCache::default();
        let fresh = cache.get(&state, "s", 105, Some(true));
        assert!(fresh.verified);
        let later = cache.get(&state, "s", 10_000, Some(true));
        assert!(std::sync::Arc::ptr_eq(&fresh, &later));
        assert!(later.verified);
        assert_eq!(cache.indexes_built, 1);
    }

    #[test]
    fn legacy_expiry_changes_verification_at_the_deadline() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        observe(&mut state, &["codex"], 100);
        let mut cache = PresentationCache::default();
        let current = cache.get(&state, "s", 105, None);
        assert!(current.verified);
        assert!(std::sync::Arc::ptr_eq(
            &current,
            &cache.get(&state, "s", 105, None)
        ));
        assert_eq!(cache.indexes_built, 1);
        let expired = cache.get(&state, "s", 106, None);
        assert!(!expired.verified);
        assert_eq!(cache.indexes_built, 2);
    }

    #[test]
    fn due_snoozes_and_reads_update_in_one_pass() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        state.sessions.push(session("other"));
        let mut early = notice("early", "s", AgentState::WaitingInput, 1);
        early.snoozed_until = 20;
        let mut later = notice("later", "s", AgentState::WaitingPermission, 2);
        later.snoozed_until = 30;
        state.notifications.push(early);
        state.notifications.push(later);
        hook(&mut state, "codex", AgentState::Running, false);
        let mut cache = PresentationCache::default();
        cache.reconcile(&state, 10, Some(true));
        assert!(cache.get(&state, "s", 10, Some(true)).attention.is_empty());
        assert_eq!(
            cache
                .get(&state, "s", 10, Some(true))
                .notice_preview
                .as_deref(),
            Some("note later")
        );
        let other = cache.get(&state, "other", 10, Some(true));
        let built = cache.indexes_built;
        let at_deadline = cache.get(&state, "s", 30, Some(true));
        assert_eq!(cache.indexes_built, built + 1);
        assert_eq!(
            at_deadline.attention,
            AttentionCounts {
                input: 1,
                permission: 1,
                failed: 0,
            }
        );
        assert!(std::sync::Arc::ptr_eq(
            &other,
            &cache.get(&state, "other", 30, Some(true))
        ));
        state.notifications[0].read = true;
        state.notifications[1].dismissed = true;
        cache.reconcile(&state, 30, Some(true));
        let updated = cache.get(&state, "s", 30, Some(true));
        assert_eq!(updated.unread, 0);
        assert_eq!(updated.attention.input, 1);
        assert_eq!(updated.attention.permission, 0);
        assert_eq!(updated.notice_preview.as_deref(), Some("note early"));
    }

    #[test]
    fn empty_and_unknown_lookups_do_not_rebuild() {
        let state = State::default();
        let mut cache = PresentationCache::default();
        let missing = cache.get(&state, "missing", 10, Some(true));
        let again = cache.get(&state, "other", 10, Some(true));
        assert!(std::sync::Arc::ptr_eq(&missing, &again));
        assert_eq!(cache.indexes_built, 1);
        assert_eq!(cache.notifications_visited, 0);
        assert_eq!(missing.diagnostics(10), "Unknown session");
        let mut state = State::default();
        state.sessions.push(session("s"));
        cache.reconcile(&state, 10, Some(true));
        let _ = cache.get(&state, "nope", 10, Some(true));
        let _ = cache.get(&state, "nope", 11, Some(true));
        assert_eq!(cache.indexes_built, 2);
    }

    #[test]
    fn replaced_collections_refresh_a_warm_cache_on_lookup() {
        let mut cache = PresentationCache::default();
        let empty = State::default();
        cache.reconcile(&empty, 10, Some(true));
        assert_eq!(cache.indexes_built, 1);
        let mut state = State::default();
        state.sessions.push(session("s"));
        hook(&mut state, "codex", AgentState::Running, false);
        let presented = cache.get(&state, "s", 10, Some(true));
        assert_eq!(presented.brand_icon, Some("AgentCodex"));
        assert_eq!(presented.status_icon, "LoaderCircle");
        assert_eq!(cache.indexes_built, 2);
        let again = cache.get(&state, "s", 10, Some(true));
        assert!(std::sync::Arc::ptr_eq(&presented, &again));
        assert_eq!(cache.indexes_built, 2);
    }

    #[test]
    fn verified_empty_presence_returns_terminal_to_plain_shell() {
        let mut state = State::default();
        state.sessions.push(session("s"));
        capable(&mut state);
        observe(&mut state, &[], now());
        hook(&mut state, "codex", AgentState::Running, false);
        let presented = present_session(&state, "s", now());
        assert!(presented.verified);
        assert!(!presented.live);
        assert_eq!(presented.brand_icon, None);
        assert_eq!(presented.brand_label, None);
        assert_eq!(presented.lifecycle, None);
        assert_eq!(presented.status_label, "Status unavailable");
        assert!(!presented.spin);
        assert!(
            presented
                .diagnostics(now())
                .contains("No live agent detected")
        );
        assert!(
            presented
                .diagnostics(now())
                .contains("Identity source: none")
        );
        assert!(unverified_sessions(&state, now()).is_empty());
        assert!(live_sessions(&state, now()).is_empty());
    }
}
