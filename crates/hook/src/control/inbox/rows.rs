//! Inbox rows from the daemon snapshot: hook-reported agent state and
//! notices only, never terminal text.
use terminator_core::*;

/// Attention order: a lower rank shows first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Rank {
    Waiting,
    Failed,
    Working,
    DoneUnread,
    Idle,
}

impl Rank {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Waiting => "WAIT",
            Self::Failed => "FAIL",
            Self::Working => "WORK",
            Self::DoneUnread => "DONE",
            Self::Idle => "idle",
        }
    }
}

/// The notice a row shows, and how to dismiss it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Notice {
    /// An agent notification (`Request::Notice`): dismiss and snooze.
    Agent { id: String, text: String },
    /// A terminal message (`Request::DismissTerminalNotice`): dismiss only.
    Terminal { id: String, text: String },
}

impl Notice {
    pub(super) fn text(&self) -> &str {
        match self {
            Self::Agent { text, .. } | Self::Terminal { text, .. } => text,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Row {
    pub(super) session: String,
    pub(super) project: String,
    pub(super) label: String,
    pub(super) rank: Rank,
    pub(super) state: Option<AgentState>,
    /// Seconds since the last hook report (or since the session started).
    pub(super) age: u64,
    pub(super) notice: Option<Notice>,
}

fn pending(n: &Notification, now: u64) -> bool {
    !n.dismissed && !n.resolved && n.snoozed_until <= now
}

fn one_line(summary: &str, details: &str) -> String {
    let text = if details.trim().is_empty() {
        summary.to_owned()
    } else {
        format!("{summary}: {details}")
    };
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn notice_for(state: &State, session: &str, now: u64) -> Option<Notice> {
    let agent = state
        .notifications
        .iter()
        .filter(|n| n.session_id == session && pending(n, now))
        .max_by_key(|n| n.created)
        .map(|n| Notice::Agent {
            id: n.id.clone(),
            text: one_line(&n.summary, &n.details),
        });
    agent.or_else(|| {
        state
            .terminal_notices
            .iter()
            .filter(|n| n.session_id == session && !n.dismissed)
            .max_by_key(|n| n.created)
            .map(|n| Notice::Terminal {
                id: n.id.clone(),
                text: one_line(&n.title, &n.body),
            })
    })
}

/// A session whose notifications were all dismissed or snoozed leaves the
/// attention groups until a newer notification arrives.
fn quieted(state: &State, session: &str, now: u64) -> bool {
    let mut own = state
        .notifications
        .iter()
        .filter(|n| n.session_id == session)
        .peekable();
    own.peek().is_some() && own.all(|n| !pending(n, now))
}

fn rank(state: &State, session: &str, agent: Option<AgentState>, now: u64) -> Rank {
    let rank = match agent {
        Some(AgentState::WaitingInput | AgentState::WaitingPermission) => Rank::Waiting,
        Some(AgentState::Failed) => Rank::Failed,
        Some(AgentState::Running) => Rank::Working,
        Some(AgentState::Completed)
            if state
                .notifications
                .iter()
                .any(|n| n.session_id == session && !n.read && pending(n, now)) =>
        {
            Rank::DoneUnread
        }
        _ => Rank::Idle,
    };
    if matches!(rank, Rank::Waiting | Rank::Failed | Rank::DoneUnread)
        && quieted(state, session, now)
    {
        return Rank::Idle;
    }
    rank
}

/// Live shell sessions plus ended ones with a pending notice, sorted by
/// attention, then newest report first.
pub(super) fn rows(state: &State, now: u64) -> Vec<Row> {
    let mut rows: Vec<Row> = state
        .sessions
        .iter()
        .filter(|s| s.kind == SessionKind::Shell && !s.review)
        .filter_map(|s| {
            let agent = state
                .agents
                .iter()
                .filter(|a| a.session_id == s.id)
                .max_by_key(|a| (a.updated, a.sequence));
            let notice = notice_for(state, &s.id, now);
            if !s.lifecycle.live() && notice.is_none() {
                return None;
            }
            let state_now = agent.map(|a| a.state);
            Some(Row {
                session: s.id.clone(),
                project: project_name(state, &s.project_id),
                label: s.label.clone(),
                rank: rank(state, &s.id, state_now, now),
                state: state_now,
                age: now.saturating_sub(agent.map_or(s.created, |a| a.updated)),
                notice,
            })
        })
        .collect();
    rows.sort_by(|a, b| a.rank.cmp(&b.rank).then(a.age.cmp(&b.age)));
    rows
}

fn project_name(state: &State, id: &str) -> String {
    state
        .projects
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.name.clone())
        .or_else(|| {
            state
                .remote_projects
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.name.clone())
        })
        .unwrap_or_else(|| id.to_owned())
}

/// `42s`, `5m`, `3h`, `2d`.
pub(super) fn age(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    }
}
