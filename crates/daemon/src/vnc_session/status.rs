//! Publishes a session's connection status to its runtime and to the
//! state record that clients read, with a typed reason on failure.
use super::*;
use terminator_core::vnc::VncErrorKind;

pub(super) fn mark_connected(
    shared: &Arc<Shared>,
    runtime: &Arc<Mutex<VncRuntime>>,
    session_id: &str,
) {
    {
        let mut rt = relock(runtime);
        rt.status = VncSessionStatus::Connected;
        rt.error = None;
        rt.revision = rt.revision.saturating_add(1);
    }
    let mut state = relock(&shared.state);
    if let Some(rec) = state.vnc_sessions.iter_mut().find(|s| s.id == session_id) {
        rec.status = VncSessionStatus::Connected;
        rec.retry_attempt = 0;
        rec.retry_in_seconds = 0;
        rec.error = None;
        rec.error_kind = None;
        // Only a connection that worked counts as the host's last use.
        let profile_id = rec.profile_id.clone();
        if let Some(profile) =
            profile_id.and_then(|pid| state.vnc_profiles.iter_mut().find(|p| p.id == pid))
        {
            profile.last_connected = terminator_core::now();
        }
        state.revision = state.revision.saturating_add(1);
    }
}

pub(super) struct MarkFailedInput<'a> {
    pub(super) shared: &'a Arc<Shared>,
    pub(super) runtime: &'a Arc<Mutex<VncRuntime>>,
    pub(super) session_id: &'a str,
    pub(super) message: String,
    pub(super) kind: VncErrorKind,
}

pub(super) fn mark_failed(input: MarkFailedInput<'_>) {
    let MarkFailedInput {
        shared,
        runtime,
        session_id,
        message,
        kind,
    } = input;
    {
        let mut rt = relock(runtime);
        rt.status = VncSessionStatus::Failed;
        rt.error = Some(message.clone());
        rt.input_tx = None;
        rt.revision = rt.revision.saturating_add(1);
    }
    let mut state = relock(&shared.state);
    if let Some(rec) = state.vnc_sessions.iter_mut().find(|s| s.id == session_id) {
        rec.status = VncSessionStatus::Failed;
        rec.error = Some(message);
        rec.error_kind = Some(kind);
        state.revision = state.revision.saturating_add(1);
    }
}

pub(super) struct MarkDisconnectedInput<'a> {
    pub(super) shared: &'a Arc<Shared>,
    pub(super) runtime: &'a Arc<Mutex<VncRuntime>>,
    pub(super) session_id: &'a str,
    pub(super) reason: Option<String>,
    pub(super) kind: Option<VncErrorKind>,
}

pub(super) fn mark_disconnected(input: MarkDisconnectedInput<'_>) {
    let MarkDisconnectedInput {
        shared,
        runtime,
        session_id,
        reason,
        kind,
    } = input;
    {
        let mut rt = relock(runtime);
        if rt.status == VncSessionStatus::Failed {
            return;
        }
        rt.status = VncSessionStatus::Disconnected;
        rt.error.clone_from(&reason);
        rt.input_tx = None;
        rt.revision = rt.revision.saturating_add(1);
    }
    let mut state = relock(&shared.state);
    if let Some(rec) = state.vnc_sessions.iter_mut().find(|s| s.id == session_id)
        && rec.status != VncSessionStatus::Failed
    {
        rec.status = VncSessionStatus::Disconnected;
        rec.error = reason;
        rec.error_kind = kind;
        state.revision = state.revision.saturating_add(1);
    }
}
