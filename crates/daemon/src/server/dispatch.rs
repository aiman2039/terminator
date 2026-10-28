use anyhow::{Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use portable_pty::PtySize;
use std::{
    io::Read,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};
use terminator_core::*;

use super::shared::{Disconnect, Runtime, Shared, relock};
use crate::presence;

/// Drain the notification waker socketpair after `poll` reports it readable.
/// The read end is non-blocking, so this stops at the first `WouldBlock`.
pub(crate) fn drain_wake(stream: &mut transport::Stream) {
    let mut buf = [0u8; 64];
    while let Ok(read) = stream.read(&mut buf) {
        if read == 0 {
            break;
        }
    }
}

/// Raw snapshot bytes per attach frame. Base64 and the JSON envelope expand
/// this by about a third, which keeps every frame well under `MAX_FRAME`.
const SNAPSHOT_CHUNK: usize = MAX_FRAME / 2;

/// Tell the client why the connection is refused before closing it, so it
/// does not see a bare EOF and guess at the cause.
fn reject(stream: &mut transport::Stream, error: anyhow::Error) -> anyhow::Error {
    let _ = write_frame(stream, &Response::Error(format!("{error:#}")));
    error
}

/// A registered attach: its runtime, subscriber stream, and initial snapshot.
type Attached = (Arc<Mutex<Runtime>>, mpsc::Receiver<Response>, Vec<u8>);

struct AttachSetupInput<'a> {
    shared: &'a Shared,
    session: &'a str,
    rows: u16,
    cols: u16,
}

/// Validate an attach, resize the terminal, and register the subscriber.
/// The snapshot is captured under the runtime lock with the subscriber so no
/// output is lost between them, but it is written after the lock is released.
fn attach_setup(input: AttachSetupInput<'_>) -> Result<Attached> {
    let AttachSetupInput {
        shared,
        session,
        rows,
        cols,
    } = input;
    anyhow::ensure!(
        rows > 0 && cols > 0 && rows <= 500 && cols <= 1000,
        "Invalid terminal dimensions"
    );
    let rt = shared.runtime(session)?;
    let (tx, rx) = mpsc::sync_channel(128);
    let snapshot = {
        let mut r = relock(&rt);
        r.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        r.parser.screen_mut().set_size(rows, cols);
        let snapshot = crate::terminal_events::attachment_snapshot(&r.parser);
        r.subscribers.push(tx);
        snapshot
    };
    Ok((rt, rx, snapshot))
}

/// Split an attach snapshot into `Data` frames that each fit `MAX_FRAME`.
/// An empty snapshot still yields one frame: clients read it as the attach
/// acknowledgement.
fn snapshot_frames(snapshot: &[u8]) -> Vec<Response> {
    if snapshot.is_empty() {
        return vec![Response::Data(String::new())];
    }
    snapshot
        .chunks(SNAPSHOT_CHUNK)
        .map(|chunk| Response::Data(B64.encode(chunk)))
        .collect()
}

fn authenticated(env: &Envelope, shared: &Shared) -> bool {
    if env.auth == shared.auth {
        return true;
    }
    match &env.request {
        Request::Hook(e) => shared
            .runtime(&e.terminal_session_id)
            .is_ok_and(|r| relock(&r).token == env.auth),
        Request::ShellCommand { session }
        | Request::ShellPrompt { session, .. }
        | Request::Cwd { session, .. }
        | Request::TerminalNotify { session, .. } => shared
            .runtime(session)
            .is_ok_and(|r| relock(&r).token == env.auth),
        _ => false,
    }
}
fn prepare_snapshot(
    env: &Envelope,
    shared: &Shared,
    stream: &mut transport::Stream,
) -> Result<bool> {
    if !matches!(env.request, Request::Snapshot) {
        return Ok(false);
    }
    let executable = std::env::current_exe().ok();
    let helper = &shared.helper.executable;
    let available = executable_available(helper);
    let mut state = relock(&shared.state);
    if state.daemon_executable != executable
        || state.attachment_helper_executable.as_ref() != Some(helper)
        || state.attachment_helper_available != Some(available)
    {
        state.daemon_executable = executable;
        state.attachment_helper_executable = Some(helper.clone());
        state.attachment_helper_available = Some(available);
        state.revision = state.revision.saturating_add(1);
    }
    presence::expire_observations(&mut state, now());
    if let Some(hint) = &env.snapshot_hint
        && hint == &state.snapshot_hint()
    {
        write_frame(stream, &Response::Unchanged)?;
        return Ok(true);
    }
    Ok(false)
}
struct ServeAttachInput {
    session: String,
    rows: u16,
    cols: u16,
}
fn serve_attach(
    stream: &mut transport::Stream,
    shared: &Arc<Shared>,
    input: ServeAttachInput,
) -> Result<()> {
    let ServeAttachInput {
        session,
        rows,
        cols,
    } = input;
    let _disconnect = Disconnect(stream.try_clone()?);
    let (rt, rx, snapshot) = match attach_setup(AttachSetupInput {
        shared,
        session: &session,
        rows,
        cols,
    }) {
        Ok(setup) => setup,
        Err(error) => return Err(reject(stream, error)),
    };
    for frame in snapshot_frames(&snapshot) {
        write_frame(stream, &frame)?;
    }
    stream.set_read_timeout(None)?;
    AttachedInput {
        shared: Arc::clone(shared),
        rt: Arc::clone(&rt),
        session,
    }
    .spawn(stream)?;
    relay_attach_output(stream, &rx, &rt)
}

/// The input half of an attach: client keystrokes and resizes for one session.
struct AttachedInput {
    shared: Arc<Shared>,
    rt: Arc<Mutex<Runtime>>,
    session: String,
}

impl AttachedInput {
    /// Reads input frames on a clone of `stream` until the client stops
    /// sending or input can no longer be delivered.
    fn spawn(self, stream: &transport::Stream) -> Result<()> {
        let mut input_stream = stream.try_clone()?;
        let input_disconnect = Disconnect(input_stream.try_clone()?);
        thread::spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _disconnect = input_disconnect;
                while let Ok(req) = read_frame::<Request>(&mut input_stream) {
                    if !self.apply(req) {
                        break;
                    }
                }
            }));
        });
        Ok(())
    }

    /// Applies one input frame; `false` ends the input stream.
    fn apply(&self, req: Request) -> bool {
        match req {
            Request::Input { data } => match B64.decode(data) {
                Ok(bytes) => self.shared.forward_input(&self.rt, &bytes).is_ok(),
                Err(_) => true,
            },
            Request::Resize { rows, cols }
                if rows > 0 && cols > 0 && rows <= 500 && cols <= 1000 =>
            {
                self.resize(rows, cols);
                true
            }
            _ => false,
        }
    }

    fn resize(&self, rows: u16, cols: u16) {
        let mut r = relock(&self.rt);
        let _ = r.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        r.parser.screen_mut().set_size(rows, cols);
        drop(r);
        let mut s = relock(&self.shared.state);
        if let Some(rec) = s.sessions.iter_mut().find(|s| s.id == self.session) {
            rec.rows = rows;
            rec.cols = cols;
        }
    }
}

/// Forwards subscriber frames until `End`. Once the session is live past
/// the first quiet 2 s, idle periods send `Ok` keepalives.
fn relay_attach_output(
    stream: &mut transport::Stream,
    rx: &mpsc::Receiver<Response>,
    rt: &Arc<Mutex<Runtime>>,
) -> Result<()> {
    while let Ok(frame) = rx.recv_timeout(Duration::from_secs(2)) {
        write_frame(stream, &frame)?;
        if matches!(frame, Response::End) {
            return Ok(());
        }
    }
    if relock(rt).ended {
        return Ok(());
    }
    loop {
        match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(frame) => {
                write_frame(stream, &frame)?;
                if matches!(frame, Response::End) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                write_frame(stream, &Response::Ok)?;
            }
            Err(_) => break,
        }
    }
    Ok(())
}
pub(crate) fn serve(mut stream: transport::Stream, shared: Arc<Shared>) -> Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let env: Envelope = read_frame(&mut stream)?;
    if env.version != PROTOCOL_VERSION {
        return Err(reject(
            &mut stream,
            anyhow!(
                "Unsupported protocol version {} (daemon speaks {PROTOCOL_VERSION}); restart the session service",
                env.version
            ),
        ));
    }
    if !authenticated(&env, &shared) {
        return Err(reject(&mut stream, anyhow!("Authentication failed")));
    }
    if matches!(env.request, Request::VncFrame { .. }) && env.snapshot_chunks {
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    }
    if prepare_snapshot(&env, &shared, &mut stream)? {
        return Ok(());
    }
    if let Request::Attach {
        session,
        rows,
        cols,
    } = env.request
    {
        return serve_attach(
            &mut stream,
            &shared,
            ServeAttachInput {
                session,
                rows,
                cols,
            },
        );
    }
    let ssh_errors = env.ssh_errors;
    let response = shared.handle(env.request).unwrap_or_else(|e| {
        match terminator_core::ssh::SshError::find(e.as_ref()) {
            Some(ssh) if ssh_errors => Response::SshError(terminator_core::ssh::SshError {
                failure: ssh.failure.clone(),
                message: format!("{e:#}"),
            }),
            _ => Response::Error(format!("{e:#}")),
        }
    });
    Ok(snapshot::write_response(
        &mut stream,
        &response,
        env.snapshot_chunks,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decoded(frames: &[Response]) -> Vec<u8> {
        frames
            .iter()
            .flat_map(|frame| match frame {
                Response::Data(data) => B64.decode(data).unwrap(),
                _ => panic!("snapshot frames are data"),
            })
            .collect()
    }

    #[test]
    fn large_snapshots_split_into_frames_that_fit() {
        let snapshot: Vec<u8> = (0..MAX_FRAME + 17)
            .map(|i| u8::try_from(i % 251).unwrap())
            .collect();
        let frames = snapshot_frames(&snapshot);
        assert!(frames.len() > 1);
        for frame in &frames {
            assert!(serde_json::to_vec(frame).unwrap().len() <= MAX_FRAME);
        }
        assert_eq!(decoded(&frames), snapshot);
    }

    #[test]
    fn small_and_empty_snapshots_are_one_frame() {
        assert_eq!(snapshot_frames(b"hi").len(), 1);
        let empty = snapshot_frames(b"");
        assert_eq!(empty.len(), 1);
        assert!(decoded(&empty).is_empty());
    }

    #[test]
    fn rejected_connections_hear_why() {
        let (mut client, mut server) = transport::pair().unwrap();
        let error = reject(&mut server, anyhow!("Authentication failed"));
        assert_eq!(error.to_string(), "Authentication failed");
        let Response::Error(text) = read_frame::<Response>(&mut client).unwrap() else {
            panic!("expected an error frame");
        };
        assert_eq!(text, "Authentication failed");
    }
}
