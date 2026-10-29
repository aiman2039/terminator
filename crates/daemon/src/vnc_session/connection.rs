//! One connection attempt. Retry policy and credential lookup live elsewhere.
use super::*;
use super::{input, quality, ssh_steps, transport};
use std::time::Duration;
use terminator_core::vnc::{Transport, VncErrorKind, VncOptions};

/// Why an attempt ended. `Network` may be retried; `Permanent` may not.
/// The kind tells viewers which fix to offer.
pub(super) enum Failure {
    Network(VncErrorKind, String),
    Permanent(VncErrorKind, String),
    Cancelled,
}
impl Failure {
    /// A lost connection to a running session.
    fn dropped(error: impl std::fmt::Display) -> Self {
        Self::Network(VncErrorKind::Unreachable, error.to_string())
    }
}
pub(super) struct ConnectionSpec {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: zeroize::Zeroizing<String>,
    pub shared: bool,
    pub options: VncOptions,
}
pub(super) struct RunInput<'a> {
    pub shared: &'a Arc<Shared>,
    pub runtime: &'a Arc<Mutex<VncRuntime>>,
    pub sid: &'a str,
    pub spec: &'a ConnectionSpec,
    pub input_rx: &'a mut mpsc::Receiver<VncInputEvent>,
}
async fn connect_client(
    shared: &Arc<Shared>,
    spec: &ConnectionSpec,
    input_rx: &mut mpsc::Receiver<VncInputEvent>,
) -> Result<vnc::VncClient, Failure> {
    let control = shared.ssh_control(&spec.options.ssh);
    if spec.options.transport == Transport::Ssh {
        let started = tokio::select! {
            result=ssh_steps::run_start_command(&spec.options, control.as_deref())=>result,
            _=input_rx.recv()=>return Err(Failure::Cancelled),
        };
        started.map_err(|(kind, message)| Failure::Permanent(kind, message))?;
    }
    let connected = tokio::select! {
        result=transport::connect(transport::ConnectInput { host: &spec.host, port: spec.port, options: &spec.options, control: control.as_deref() })=>result,
        _=input_rx.recv()=>return Err(Failure::Cancelled),
    };
    let transport::Connected { stream, ssh_stderr } = connected.map_err(|e| {
        if spec.options.transport == Transport::Ssh {
            Failure::Permanent(VncErrorKind::Other, format!("SSH connection failed: {e}"))
        } else {
            Failure::Network(VncErrorKind::Unreachable, format!("Connection failed: {e}"))
        }
    })?;
    let policy = transport::security(&spec.host, &spec.options).map_err(|e| {
        Failure::Permanent(VncErrorKind::Tls, format!("TLS configuration failed: {e}"))
    })?;
    let secret = zeroize::Zeroizing::new(spec.password.to_string());
    // Asked only when the server wants a VNC password: an empty one is
    // never sent, so the viewer can prompt instead of failing on auth.
    let auth = async move {
        if secret.is_empty() {
            Err(vnc::VncError::NoPassword)
        } else {
            Ok(secret.to_string())
        }
    };
    let mut connector = VncConnector::new(stream)
        .set_security(policy)
        .set_username(spec.username.clone())
        .set_auth_method(auth)
        .allow_shared(spec.shared)
        .set_pixel_format(PixelFormat::rgba());
    for encoding in quality::encodings(spec.options.quality, false) {
        connector = connector.add_encoding(encoding);
    }
    let state = connector
        .build()
        .map_err(|e| Failure::Permanent(VncErrorKind::Other, format!("VNC setup failed: {e}")))?;
    let handshake = tokio::time::timeout(Duration::from_secs(15), state.try_start());
    let outcome = tokio::select! {
        result=handshake=>result,
        item=input_rx.recv()=>{let _=item;return Err(Failure::Cancelled);}
    };
    let state = match outcome {
        Err(_) => {
            return Err(Failure::Network(
                VncErrorKind::Unreachable,
                "VNC handshake timed out".into(),
            ));
        }
        Ok(Err(error)) => {
            return Err(handshake_failure(error, spec.options.transport, ssh_stderr).await);
        }
        Ok(Ok(state)) => state,
    };
    let client = state
        .finish()
        .map_err(|e| Failure::Permanent(VncErrorKind::Other, format!("VNC setup failed: {e}")))?;
    Ok(client)
}
struct TypeTextInput<'a> {
    client: &'a vnc::VncClient,
    shared: &'a Arc<Shared>,
    runtime: &'a Arc<Mutex<VncRuntime>>,
    sid: &'a str,
    text: &'a str,
    input_rx: &'a mut mpsc::Receiver<VncInputEvent>,
}

async fn type_text(input: TypeTextInput<'_>) -> Result<(), Failure> {
    let TypeTextInput {
        client,
        shared,
        runtime,
        sid,
        text,
        input_rx,
    } = input;
    for keycode in text.chars().filter_map(input::keysym) {
        if input_rx.is_closed() {
            return Err(Failure::Cancelled);
        }
        for down in [true, false] {
            if let Err(e) = client
                .input(X11Event::KeyEvent(ClientKeyEvent { keycode, down }))
                .await
            {
                return Err(Failure::dropped(e));
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        while let Ok(Some(event)) = client.poll_event().await {
            if !apply_vnc_event(ApplyVncEventInput {
                shared,
                runtime,
                session_id: sid,
                event,
            }) {
                return Err(Failure::Permanent(
                    VncErrorKind::Protocol,
                    "VNC protocol update was rejected".into(),
                ));
            }
        }
    }
    Ok(())
}
pub(super) async fn run(input: RunInput<'_>) -> Result<(), Failure> {
    let RunInput {
        shared,
        runtime,
        sid,
        spec,
        input_rx,
    } = input;
    let client = connect_client(shared, spec, input_rx).await?;
    mark_connected(shared, runtime, sid);
    let mut tick = tokio::time::interval(REFRESH_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut resize: Option<tokio::task::JoinHandle<()>> = None;
    let mut last_request = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    let connected_at = Instant::now();
    let mut adaptive = quality::Adaptive::default();
    let result = 'connection: loop {
        tokio::select! {
            event=client.recv_event()=>match event {
                Ok(event)=>{
                    if !apply_vnc_event(ApplyVncEventInput { shared, runtime, session_id: sid, event }){break Err(Failure::Permanent(VncErrorKind::Protocol,relock(runtime).error.clone().unwrap_or_else(||"VNC protocol update was rejected".into())));}
                    while let Ok(Some(event))=client.poll_event().await {
                        if !apply_vnc_event(ApplyVncEventInput { shared, runtime, session_id: sid, event }){break;}
                    }
                    if let Err(e)=request_refresh(&client,runtime,spec.options.quality,&mut last_request).await{break Err(e);}
                }
                Err(e)=>break Err(Failure::dropped(e)),
            },
            item=input_rx.recv()=>{
                let Some(first)=item else{break Err(Failure::Cancelled)};
                // Take everything already queued so stale pointer moves can be
                // dropped instead of replayed one by one behind the cursor.
                let mut batch=vec![first];
                while let Ok(event)=input_rx.try_recv(){batch.push(event);}
                for event in input::coalesce_pointer_moves(batch) {
                match event {
                    VncInputEvent::Resize{width,height}=>{
                        if resize.as_ref().is_some_and(|task|!task.is_finished()){continue;}
                        let client=client.clone();let runtime=Arc::clone(runtime);
                        resize=Some(tokio::spawn(async move{
                            if let Err(e)=client.resize_desktop(width,height).await {let mut rt=relock(&runtime);rt.error=Some(format!("Remote resize failed: {e}"));rt.revision=rt.revision.saturating_add(1);}
                        }));
                    }
                    VncInputEvent::TypeText(text)=>{
                        if let Err(e)=type_text(TypeTextInput { client: &client, shared, runtime, sid, text: &text, input_rx }).await{break 'connection Err(e);}
                    }
                    event=>{
                        let printable_release=matches!(&event,VncInputEvent::Key{keycode,down:false} if *keycode<=255||*keycode>=0x0100_0000);
                        if let Some(event)=input::to_x11_event(event)
                            && let Err(e)=client.input(event).await{break 'connection Err(Failure::dropped(e));}
                        if printable_release{tokio::time::sleep(Duration::from_millis(10)).await;}
                    },
                }
                }
            },
            _=tick.tick()=>{
                if detach_due(shared,runtime,connected_at){
                    // Nobody has looked at it for a long time: free the
                    // connection like an explicit Disconnect would.
                    let _=shared.vnc_disconnect(sid);
                    break Err(Failure::Cancelled);
                }
                if let Err(e)=request_refresh(&client,runtime,spec.options.quality,&mut last_request).await{break Err(e);}
                let update_ms=relock(runtime).stats.snapshot().update_ms;
                if let Some(encodings)=adaptive.check(spec.options.quality,update_ms)
                    && let Err(e)=client.input(X11Event::SetEncodings(encodings)).await{break Err(Failure::dropped(e));}
            }
        }
    };
    if let Some(task) = resize {
        task.abort();
        let _ = task.await;
    }
    let _ = client.close().await;
    result
}

/// How long since a viewer last asked for a frame, counting from this
/// connection's start at most.
fn unwatched_for(runtime: &Arc<Mutex<VncRuntime>>, connected_at: Instant) -> Duration {
    let since_connect = connected_at.elapsed();
    let since_poll = relock(runtime)
        .last_poll
        .map_or(since_connect, |polled| polled.elapsed());
    since_connect.min(since_poll)
}

/// True when no viewer has watched for longer than the Settings timeout.
/// Settings are read only once nobody has watched for a minute.
fn detach_due(shared: &Shared, runtime: &Arc<Mutex<VncRuntime>>, connected_at: Instant) -> bool {
    let unwatched = unwatched_for(runtime, connected_at);
    unwatched >= Duration::from_mins(1)
        && detach_limit(relock(&shared.state).settings.vnc_detached_timeout_minutes)
            .is_some_and(|limit| unwatched >= limit)
}

/// The Settings value as a time limit; 0 keeps detached sessions.
fn detach_limit(minutes: u32) -> Option<Duration> {
    (minutes > 0).then(|| Duration::from_secs(u64::from(minutes).saturating_mul(60)))
}

/// How often the loop checks whether a new update request is due.
const REFRESH_TICK: Duration = Duration::from_millis(8);
/// Ask again when an update request got no complete answer this long.
const REFRESH_RESEND: Duration = Duration::from_secs(1);

/// Pipelined incremental updates, as other viewers do: ask for the next
/// update as soon as the previous one ended (no sooner than the quality's
/// minimum interval), resend after [`REFRESH_RESEND`] without an answer,
/// and stop while no viewer is watching.
async fn request_refresh(
    client: &vnc::VncClient,
    runtime: &Arc<Mutex<VncRuntime>>,
    quality: terminator_core::vnc::Quality,
    last_request: &mut Instant,
) -> Result<(), Failure> {
    let (watched, pending, interval) = {
        let rt = relock(runtime);
        (
            rt.polled_recently(),
            rt.update_pending,
            quality::refresh_ms(quality, rt.last_encoded_bytes),
        )
    };
    let elapsed = last_request.elapsed();
    let due = watched
        && if pending {
            elapsed >= REFRESH_RESEND
        } else {
            elapsed >= Duration::from_millis(interval)
        };
    if !due {
        return Ok(());
    }
    client
        .input(X11Event::Refresh)
        .await
        .map_err(Failure::dropped)?;
    relock(runtime).update_pending = true;
    *last_request = Instant::now();
    Ok(())
}

/// Classify a failed handshake. A missing password asks the viewer for
/// one; a tunnel that wrote OpenSSH errors reports the SSH problem.
async fn handshake_failure(
    error: vnc::VncError,
    transport: Transport,
    ssh_stderr: Option<tokio::task::JoinHandle<Vec<u8>>>,
) -> Failure {
    if matches!(error, vnc::VncError::NoPassword) {
        return Failure::Permanent(
            VncErrorKind::PasswordRequired,
            "This VNC server needs a password".into(),
        );
    }
    if let Some(handle) = ssh_stderr
        && let Some(stderr) = ssh_steps::collected(handle).await
    {
        let (kind, message) = ssh_steps::classify(&stderr);
        return if kind == VncErrorKind::Unreachable {
            Failure::Network(kind, message)
        } else {
            Failure::Permanent(kind, message)
        };
    }
    let network = transport != Transport::Ssh
        && matches!(&error,vnc::VncError::IoError(e) if matches!(e.kind(),std::io::ErrorKind::UnexpectedEof|std::io::ErrorKind::ConnectionReset|std::io::ErrorKind::ConnectionAborted|std::io::ErrorKind::TimedOut|std::io::ErrorKind::BrokenPipe));
    let message = format!("VNC authentication or security negotiation failed: {error}");
    if network {
        Failure::Network(VncErrorKind::Unreachable, message)
    } else {
        Failure::Permanent(VncErrorKind::from_handshake(&error.to_string()), message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_session_without_a_viewer_for_long_is_detached_too_long() {
        let (tx, _rx) = mpsc::channel(1);
        let runtime = Arc::new(Mutex::new(VncRuntime::new(false, tx)));
        let limit = detach_limit(30).unwrap();
        let long_ago = Instant::now().checked_sub(limit).unwrap();
        assert!(
            unwatched_for(&runtime, Instant::now()) < limit,
            "just connected"
        );
        assert!(
            unwatched_for(&runtime, long_ago) >= limit,
            "never watched since"
        );
        relock(&runtime).last_poll = Some(Instant::now());
        assert!(
            unwatched_for(&runtime, long_ago) < limit,
            "a viewer just looked"
        );
    }

    #[test]
    fn a_zero_timeout_keeps_detached_sessions() {
        assert_eq!(detach_limit(0), None);
        assert_eq!(detach_limit(2), Some(Duration::from_mins(2)));
    }
}
