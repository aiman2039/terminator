use base64::{Engine, engine::general_purpose::STANDARD as B64};
use std::{
    io::Read,
    sync::{Arc, Mutex, mpsc},
    thread,
};
use terminator_core::*;

use super::super::super::shared::{HistoryJob, Runtime, Shared, relock};
use crate::terminal_events;

impl Shared {
    /// Starts the PTY reader thread. The returned channel disconnects when
    /// the reader finishes, so the exit thread can deliver the final output
    /// before ending subscriber streams.
    pub(super) fn spawn_output_reader(
        self: &Arc<Self>,
        sid: &str,
        runtime: &Arc<Mutex<Runtime>>,
        reader: Box<dyn Read + Send>,
    ) -> mpsc::Receiver<()> {
        let shared = Arc::clone(self);
        let read_sid = sid.to_owned();
        let read_rt = Arc::clone(runtime);
        let (reader_done, reader_finished) = mpsc::channel::<()>();
        thread::spawn(move || {
            let _reader_done = reader_done;
            shared.pump_output(&read_sid, &read_rt, reader);
        });
        reader_finished
    }

    fn pump_output(
        &self,
        sid: &str,
        runtime: &Arc<Mutex<Runtime>>,
        mut reader: Box<dyn Read + Send>,
    ) {
        let mut bytes = [0u8; 8192];
        while let Ok(n) = reader.read(&mut bytes) {
            if n == 0 {
                break;
            }
            let Some(data) = bytes.get(..n) else {
                break;
            };
            self.apply_output(sid, runtime, data);
            self.queue_history(sid, data);
        }
    }

    /// Feeds output to the parser and subscribers, then answers terminal
    /// queries and raises the notices the output carried.
    fn apply_output(&self, sid: &str, runtime: &Arc<Mutex<Runtime>>, data: &[u8]) {
        let mut rt = relock(runtime);
        terminal_events::process(&mut rt.parser, data);
        let replies = std::mem::take(&mut rt.parser.callbacks_mut().replies);
        let notices = std::mem::take(&mut rt.parser.callbacks_mut().notices);
        let frame = Response::Data(B64.encode(data));
        rt.subscribers.retain(|s| s.try_send(frame.clone()).is_ok());
        drop(rt);
        for reply in replies {
            let _ = self.forward_input(runtime, reply.as_bytes());
        }
        for notice in notices {
            self.raise_terminal_notice(sid, &notice.title, &notice.body);
        }
    }

    fn raise_terminal_notice(&self, sid: &str, title: &str, body: &str) {
        let mut state = relock(&self.state);
        let os = state.settings.terminal_notifications_os;
        let id = state.terminal_notice(sid, title, body).ok().flatten();
        drop(state);
        if os && let Some(id) = id {
            let _ = self.alerts.try_send(id);
        }
    }

    /// Bound memory without dropping bursts. No runtime/state lock is held
    /// while storage applies backpressure to this PTY reader.
    fn queue_history(&self, sid: &str, data: &[u8]) {
        if self
            .history
            .send(HistoryJob::Append(sid.to_owned(), data.to_vec()))
            .is_ok()
        {
            return;
        }
        let mut s = relock(&self.state);
        if let Some(rec) = s.sessions.iter_mut().find(|s| s.id == sid) {
            rec.truncated = true;
        }
        s.degraded = Some(
            "History storage worker stopped; terminal output continues but some history was not saved"
                .into(),
        );
        s.revision = s.revision.saturating_add(1);
    }
}
