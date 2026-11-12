use base64::{Engine, engine::general_purpose::STANDARD as B64};
use std::{
    io::Read,
    sync::{
        Arc, Mutex,
        mpsc::{self, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};
use terminator_core::*;

use super::super::super::shared::{HistoryJob, Runtime, Shared, relock};
use crate::terminal_events;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Stores a live session's window title (`terminal-title-v1`); an empty
/// title clears it. Display only: no notice, lifecycle or persist call.
pub(crate) fn publish_title(state: &mut State, sid: &str, title: String) {
    let live = state
        .sessions
        .iter()
        .any(|s| s.id == sid && s.lifecycle.live());
    let changed = if !live || title.is_empty() {
        state.terminal_titles.remove(sid).is_some()
    } else if state.terminal_titles.get(sid) != Some(&title) {
        state.terminal_titles.insert(sid.to_owned(), title);
        true
    } else {
        false
    };
    if changed {
        state.revision = state.revision.saturating_add(1);
    }
}

/// Pending query replies per session; more are dropped, never waited for.
const REPLY_QUEUE: usize = 64;

fn enqueue_replies(queue: &SyncSender<String>, replies: impl IntoIterator<Item = String>) {
    for reply in replies {
        // Full (writer blocked on a stalled PTY) or closed: drop the reply.
        let _ = queue.try_send(reply);
    }
}

/// Longest wait for a slow attach client before one output frame is given
/// up. The relay's 3 s write timeout disconnects a stuck client sooner.
const SUBSCRIBER_STALL: Duration = Duration::from_secs(10);

/// Waits (without any lock) for a full subscriber queue to drain, so heavy
/// output slows the PTY reader instead of detaching the pane. Returns `false`
/// when the client went away or the stall limit passed.
fn send_with_backpressure(tx: &SyncSender<Response>, frame: Response, limit: Duration) -> bool {
    let start = Instant::now();
    let mut frame = frame;
    loop {
        match tx.try_send(frame) {
            Ok(()) => return true,
            Err(TrySendError::Disconnected(_)) => return false,
            Err(TrySendError::Full(back)) => {
                if start.elapsed() >= limit {
                    return false;
                }
                frame = back;
                thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

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

    /// Writes terminal query replies (DA/DSR/OSC) on their own thread. The
    /// reader must never block on PTY input: a program that does not read
    /// stdin would then block on output and freeze the session. The thread
    /// ends when the reader drops the queue.
    fn spawn_reply_writer(self: &Arc<Self>, runtime: &Arc<Mutex<Runtime>>) -> SyncSender<String> {
        let shared = Arc::clone(self);
        let runtime = Arc::clone(runtime);
        let (tx, rx) = mpsc::sync_channel::<String>(REPLY_QUEUE);
        thread::spawn(move || {
            while let Ok(reply) = rx.recv() {
                let _ = shared.forward_input(&runtime, reply.as_bytes());
            }
        });
        tx
    }

    fn pump_output(
        self: &Arc<Self>,
        sid: &str,
        runtime: &Arc<Mutex<Runtime>>,
        mut reader: Box<dyn Read + Send>,
    ) {
        let replies = self.spawn_reply_writer(runtime);
        pump_guarded(
            &mut reader,
            |data| enqueue_replies(&replies, self.apply_output(sid, runtime, data)),
            || {
                self.degraded(&format!(
                    "Terminal output parser failed for session {sid}; output is still saved to history"
                ));
            },
            |data| self.queue_history(sid, data),
        );
    }

    /// Feeds output to the parser and subscribers, raises the notices the
    /// output carried, and returns the terminal query replies to send.
    fn apply_output(
        &self,
        sid: &str,
        runtime: &Arc<Mutex<Runtime>>,
        data: &[u8],
    ) -> std::collections::VecDeque<String> {
        let mut rt = relock(runtime);
        let parsed = rt.osc_limit.filter(data);
        terminal_events::process(&mut rt.parser, &parsed);
        let _finished = rt.commands.process(&parsed, now_ms());
        let replies = std::mem::take(&mut rt.parser.callbacks_mut().replies);
        let notices = std::mem::take(&mut rt.parser.callbacks_mut().notices);
        let title = rt.parser.callbacks_mut().take_title(Instant::now(), false);
        let frame = Response::Data(B64.encode(data));
        let mut blocked = Vec::new();
        rt.subscribers
            .retain(|tx| match tx.try_send(frame.clone()) {
                Ok(()) => true,
                Err(TrySendError::Full(frame)) => {
                    blocked.push((tx.clone(), frame));
                    true
                }
                Err(TrySendError::Disconnected(_)) => false,
            });
        drop(rt);
        for (tx, frame) in blocked {
            send_with_backpressure(&tx, frame, SUBSCRIBER_STALL);
        }
        for notice in notices {
            self.raise_terminal_notice(sid, &notice.title, &notice.body);
        }
        if let Some(title) = title {
            publish_title(&mut relock(&self.state), sid, title);
        }
        replies
    }

    /// Publishes throttled title changes the PTY reader has not sent yet, so
    /// the final title shows even when output stops right after it.
    pub(crate) fn flush_terminal_titles(&self) {
        let runtimes: Vec<(String, Arc<Mutex<Runtime>>)> = relock(&self.sessions)
            .iter()
            .map(|(id, rt)| (id.clone(), Arc::clone(rt)))
            .collect();
        let titles: Vec<(String, String)> = runtimes
            .into_iter()
            .filter_map(|(id, rt)| {
                let title = relock(&rt)
                    .parser
                    .callbacks_mut()
                    .take_title(Instant::now(), true)?;
                Some((id, title))
            })
            .collect();
        if titles.is_empty() {
            return;
        }
        let mut state = relock(&self.state);
        for (id, title) in titles {
            publish_title(&mut state, &id, title);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_replies_are_dropped_instead_of_blocking_the_reader() {
        let (tx, rx) = mpsc::sync_channel::<String>(2);
        let replies = (0..10).map(|i| format!("\x1b[{i};1R"));
        enqueue_replies(&tx, replies);
        assert_eq!(rx.try_iter().count(), 2);
    }

    #[test]
    fn published_title_bumps_revision_only_on_change_and_only_for_live_sessions() {
        let mut state: State = serde_json::from_value(serde_json::json!({
            "sessions": [{
                "id": "s", "project_id": "p", "label": "zsh", "cwd": "/",
                "kind": "shell", "file": null, "lifecycle": "running", "created": 0,
                "exit_code": null, "rows": 24, "cols": 80, "generation": "g",
                "pid": 1, "truncated": false, "cwd_confirmed": true
            }]
        }))
        .unwrap();
        publish_title(&mut state, "s", "vim".into());
        assert_eq!(state.revision, 1);
        publish_title(&mut state, "s", "vim".into());
        assert_eq!(state.revision, 1);
        publish_title(&mut state, "gone", "x".into());
        assert!(!state.terminal_titles.contains_key("gone"));
        publish_title(&mut state, "s", String::new());
        assert!(state.terminal_titles.is_empty());
        assert_eq!(state.revision, 2);
        assert!(state.notifications.is_empty() && state.terminal_notices.is_empty());
    }

    #[test]
    fn full_subscriber_queue_waits_for_the_client_instead_of_dropping_output() {
        let (tx, rx) = mpsc::sync_channel::<Response>(1);
        tx.try_send(Response::Data("a".into())).unwrap();
        let reader = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            rx.iter().take(2).count()
        });
        assert!(send_with_backpressure(
            &tx,
            Response::Data("b".into()),
            Duration::from_secs(5)
        ));
        drop(tx);
        assert_eq!(reader.join().unwrap(), 2);
    }

    #[test]
    fn backpressure_stops_when_the_client_disconnects() {
        let (tx, rx) = mpsc::sync_channel::<Response>(1);
        tx.try_send(Response::Ok).unwrap();
        drop(rx);
        assert!(!send_with_backpressure(
            &tx,
            Response::Ok,
            Duration::from_secs(5)
        ));
    }
}

/// Read the PTY until it ends: `apply` each chunk, then `after` it. A panic
/// in `apply` must not end the reader: nothing would drain the PTY, so the
/// program would block on output and the session would hang without a
/// word. The first panic is reported; reading goes on.
fn pump_guarded(
    reader: &mut dyn Read,
    mut apply: impl FnMut(&[u8]),
    mut report_panic: impl FnMut(),
    mut after: impl FnMut(&[u8]),
) {
    let mut bytes = [0u8; 8192];
    let mut panicked = false;
    while let Ok(n) = reader.read(&mut bytes) {
        if n == 0 {
            break;
        }
        let Some(data) = bytes.get(..n) else {
            break;
        };
        let applied = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| apply(data)));
        if applied.is_err() && !panicked {
            panicked = true;
            report_panic();
        }
        after(data);
    }
}

#[cfg(test)]
mod pump_tests {
    use super::pump_guarded;

    /// Hands out one queued chunk per read.
    struct Chunks(std::collections::VecDeque<&'static [u8]>);

    impl std::io::Read for Chunks {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let Some(chunk) = self.0.pop_front() else {
                return Ok(0);
            };
            let n = chunk.len().min(buf.len());
            buf.get_mut(..n)
                .unwrap_or_default()
                .copy_from_slice(chunk.get(..n).unwrap_or_default());
            Ok(n)
        }
    }

    #[test]
    fn a_parser_panic_is_reported_once_and_the_pty_keeps_draining() {
        let mut reader = Chunks([b"one".as_slice(), b"two", b"three"].into());
        let (mut reports, mut drained) = (0, Vec::new());
        pump_guarded(
            &mut reader,
            |data| assert_ne!(data, b"one".as_slice(), "parser bug"),
            || reports += 1,
            |data| drained.push(data.to_vec()),
        );
        let mut reader = Chunks([b"one".as_slice(), b"one"].into());
        let mut again = 0;
        pump_guarded(&mut reader, |_| panic!("parser bug"), || again += 1, |_| {});
        assert_eq!(reports, 1);
        assert_eq!(
            drained,
            [b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
        );
        assert_eq!(again, 1, "reported once, not per chunk");
    }
}
