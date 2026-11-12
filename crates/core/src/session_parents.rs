//! Parent and child terminal sessions (`terminator-hook ctl spawn`). A link
//! is display data only: it never starts, stops or resumes a session and
//! never changes agent lifecycle. Older daemons do not advertise
//! [`CAPABILITY`] and keep no links.
use crate::{Session, State};

/// `Request::SetSessionParent` and `State::session_parents`.
pub const CAPABILITY: &str = "session-parent-v1";

/// Longest session ID a link accepts.
const MAX_ID: usize = 128;
/// Deepest parent chain walked when checking for a cycle.
const MAX_DEPTH: usize = 64;

#[derive(Debug, PartialEq, Eq)]
pub enum LinkError {
    UnknownChild,
    BadParent,
    Cycle,
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnknownChild => "Child must be a live session of this daemon",
            Self::BadParent => "Parent must be another session ID",
            Self::Cycle => "A session cannot be its own ancestor",
        })
    }
}

impl std::error::Error for LinkError {}

/// Records `parent` as the parent of the live session `child`, and drops
/// links of sessions that no longer exist. True when the links changed.
///
/// # Errors
/// Refuses an unknown or ended child, a bad parent ID, or a cycle.
pub fn link(state: &mut State, child: &str, parent: &str) -> Result<bool, LinkError> {
    if parent.is_empty() || parent.len() > MAX_ID || parent == child {
        return Err(LinkError::BadParent);
    }
    if !state
        .sessions
        .iter()
        .any(|s| s.id == child && s.lifecycle.live())
    {
        return Err(LinkError::UnknownChild);
    }
    let mut at = Some(parent);
    for _ in 0..MAX_DEPTH {
        let Some(id) = at else { break };
        if id == child {
            return Err(LinkError::Cycle);
        }
        at = state.session_parents.get(id).map(String::as_str);
    }
    let known: std::collections::BTreeSet<String> =
        state.sessions.iter().map(|s| s.id.clone()).collect();
    let before = state.session_parents.len();
    state.session_parents.retain(|c, _| known.contains(c));
    let replaced = state
        .session_parents
        .insert(child.to_owned(), parent.to_owned());
    let changed = before != state.session_parents.len() || replaced.as_deref() != Some(parent);
    if changed {
        state.revision = state.revision.saturating_add(1);
    }
    Ok(changed)
}

/// The parent of `session`, if linked.
#[must_use]
pub fn parent_of<'a>(state: &'a State, session: &str) -> Option<&'a str> {
    state.session_parents.get(session).map(String::as_str)
}

/// Live children of `parent`, in session order.
#[must_use]
pub fn live_children<'a>(state: &'a State, parent: &str) -> Vec<&'a Session> {
    state
        .sessions
        .iter()
        .filter(|s| s.lifecycle.live() && parent_of(state, &s.id) == Some(parent))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Lifecycle, SessionKind};

    fn session(id: &str, lifecycle: Lifecycle) -> Session {
        Session {
            remote: None,
            review: false,
            id: id.into(),
            project_id: "p".into(),
            label: id.into(),
            cwd: "/p".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle,
            created: 0,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "g".into(),
            pid: None,
            truncated: false,
            cwd_confirmed: true,
        }
    }

    fn state() -> State {
        let mut state = State::default();
        for id in ["a", "b", "c"] {
            state.sessions.push(session(id, Lifecycle::Running));
        }
        state.sessions.push(session("gone", Lifecycle::Ended));
        state
    }

    #[test]
    fn a_child_links_to_its_parent_once() {
        let mut s = state();
        assert_eq!(link(&mut s, "b", "a"), Ok(true));
        assert_eq!(link(&mut s, "b", "a"), Ok(false));
        assert_eq!(parent_of(&s, "b"), Some("a"));
        let children: Vec<&str> = live_children(&s, "a")
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        assert_eq!(children, ["b"]);
    }

    #[test]
    fn links_refuse_cycles_self_and_ended_children() {
        let mut s = state();
        link(&mut s, "b", "a").unwrap();
        link(&mut s, "c", "b").unwrap();
        assert_eq!(link(&mut s, "a", "c"), Err(LinkError::Cycle));
        assert_eq!(link(&mut s, "a", "a"), Err(LinkError::BadParent));
        assert_eq!(link(&mut s, "gone", "a"), Err(LinkError::UnknownChild));
        assert_eq!(link(&mut s, "missing", "a"), Err(LinkError::UnknownChild));
    }

    #[test]
    fn links_of_removed_sessions_are_dropped() {
        let mut s = state();
        link(&mut s, "b", "a").unwrap();
        s.sessions.retain(|x| x.id != "b");
        assert_eq!(link(&mut s, "c", "a"), Ok(true));
        assert_eq!(s.session_parents.len(), 1);
    }
}
