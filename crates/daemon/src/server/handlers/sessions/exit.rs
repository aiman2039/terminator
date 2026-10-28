use portable_pty::Child;
use std::{
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};
use terminator_core::*;

use super::super::super::shared::{Runtime, Shared, relock};
use crate::review;

/// Upper bound on waiting for a PTY reader to drain after its child exits.
const READER_DRAIN: Duration = Duration::from_millis(500);

/// What the exit thread owns until the session's process ends.
pub(super) struct ExitWatch {
    pub child: Box<dyn Child + Send + Sync>,
    pub runtime: Arc<Mutex<Runtime>>,
    pub sid: String,
    pub reader_finished: mpsc::Receiver<()>,
    /// Review snapshots live until the review session ends.
    pub review_files: Option<review::Files>,
}

impl Shared {
    pub(super) fn spawn_exit_watcher(self: &Arc<Self>, watch: ExitWatch) {
        let shared = Arc::clone(self);
        thread::spawn(move || shared.await_exit(watch));
    }

    fn await_exit(&self, watch: ExitWatch) {
        let ExitWatch {
            mut child,
            runtime,
            sid,
            reader_finished,
            review_files,
        } = watch;
        let exit = child.wait();
        // Output still buffered in the PTY (often the error that made a
        // launch fail) must reach subscribers before `End`. Background
        // children may keep the PTY open, so the wait is bounded.
        let _ = reader_finished.recv_timeout(READER_DRAIN);
        Self::end_subscribers(&runtime);
        self.record_exit(&sid, exit.ok().map(|e| e.exit_code()));
        let _ = self.persist();
        if !relock(&self.state).sessions.iter().any(|r| r.id == sid) {
            let _ = self.history_clear(Some(sid.clone()), true);
        }
        relock(&self.sessions).remove(&sid);
        drop(review_files);
    }

    fn end_subscribers(runtime: &Arc<Mutex<Runtime>>) {
        let mut rt = relock(runtime);
        rt.ended = true;
        for tx in rt.subscribers.drain(..) {
            let _ = tx.try_send(Response::End);
        }
    }

    fn record_exit(&self, sid: &str, exit_code: Option<u32>) {
        let mut s = relock(&self.state);
        if let Some(rec) = s.sessions.iter_mut().find(|r| r.id == sid) {
            rec.lifecycle = Lifecycle::Ended;
            rec.exit_code = exit_code;
            rec.pid = None;
        }
        for a in &mut s.agents {
            if a.session_id == sid {
                a.state = AgentState::Stopped;
            }
        }
        s.revision = s.revision.saturating_add(1);
        // Sessions without an agent resume command (plain shells,
        // file editors) are not worth keeping: reopening them
        // restores nothing actionable.
        if !s.session_has_resume(sid) {
            s.sessions.retain(|r| r.id != sid);
            s.agents.retain(|a| a.session_id != sid);
            s.notifications.retain(|n| n.session_id != sid);
            s.terminal_notices.retain(|n| n.session_id != sid);
            s.revision = s.revision.saturating_add(1);
        }
    }
}
