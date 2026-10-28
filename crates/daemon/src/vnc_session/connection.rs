//! One connection attempt. Retry policy and credential lookup live elsewhere.
use super::*;
use super::{input, quality, transport};
use std::time::Duration;
use terminator_core::vnc::{Transport, VncOptions};

pub(super) enum Failure {
    Network(String),
    Permanent(String),
    Cancelled,
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
    let connected = tokio::select! {
        result=transport::connect(&spec.host,spec.port,&spec.options,control.as_deref())=>result,
        _=input_rx.recv()=>return Err(Failure::Cancelled),
    };
    let stream = connected.map_err(|e| {
        if spec.options.transport == Transport::Ssh {
            Failure::Permanent(format!(
                "SSH connection failed: {e}; verify the saved host key and agent/key authentication"
            ))
        } else {
            Failure::Network(format!("Connection failed: {e}"))
        }
    })?;
    let policy = transport::security(&spec.host, &spec.options)
        .map_err(|e| Failure::Permanent(format!("TLS configuration failed: {e}")))?;
    let secret = zeroize::Zeroizing::new(spec.password.to_string());
    let mut connector = VncConnector::new(stream)
        .set_security(policy)
        .set_username(spec.username.clone())
        .set_auth_method(async move { Ok(secret.to_string()) })
        .allow_shared(spec.shared)
        .set_pixel_format(PixelFormat::rgba());
    for encoding in quality::encodings(&spec.options) {
        connector = connector.add_encoding(encoding);
    }
    let state = connector
        .build()
        .map_err(|e| Failure::Permanent(format!("VNC setup failed: {e}")))?;
    let handshake = tokio::time::timeout(Duration::from_secs(15), state.try_start());
    let state = tokio::select! {
        result=handshake=>result.map_err(|_|Failure::Network("VNC handshake timed out".into()))?.map_err(|e|handshake_failure(e,spec.options.transport))?,
        item=input_rx.recv()=>{let _=item;return Err(Failure::Cancelled);}
    };
    let client = state
        .finish()
        .map_err(|e| Failure::Permanent(format!("VNC setup failed: {e}")))?;
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
                return Err(Failure::Network(e.to_string()));
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
        while let Ok(Some(event)) = client.poll_event().await {
            if !apply_vnc_event(shared, runtime, sid, event) {
                return Err(Failure::Permanent(
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
    let mut tick = tokio::time::interval(Duration::from_millis(33));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut resize: Option<tokio::task::JoinHandle<()>> = None;
    let mut last_request = Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now);
    let result = 'connection: loop {
        tokio::select! {
            event=client.recv_event()=>match event {
                Ok(event)=>{
                    if !apply_vnc_event(shared,runtime,sid,event){break Err(Failure::Permanent(relock(runtime).error.clone().unwrap_or_else(||"VNC protocol update was rejected".into())));}
                    while let Ok(Some(event))=client.poll_event().await {
                        if !apply_vnc_event(shared,runtime,sid,event){break;}
                    }
                }
                Err(e)=>break Err(Failure::Network(e.to_string())),
            },
            item=input_rx.recv()=>{
                let Some(event)=item else{break Err(Failure::Cancelled)};
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
                            && let Err(e)=client.input(event).await{break Err(Failure::Network(e.to_string()));}
                        if printable_release{tokio::time::sleep(Duration::from_millis(10)).await;}
                    },
                }
            },
            _=tick.tick()=>{
                let (watched,delay)={let rt=relock(runtime);(rt.polled_recently(),quality::refresh_ms(spec.options.quality,rt.last_encoded_bytes))};
                if watched&&last_request.elapsed()>=Duration::from_millis(delay){
                    if let Err(e)=client.input(X11Event::Refresh).await{break Err(Failure::Network(e.to_string()));}
                    last_request=Instant::now();
                }
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

fn handshake_failure(error: vnc::VncError, transport: Transport) -> Failure {
    let network = transport != Transport::Ssh
        && matches!(&error,vnc::VncError::IoError(e) if matches!(e.kind(),std::io::ErrorKind::UnexpectedEof|std::io::ErrorKind::ConnectionReset|std::io::ErrorKind::ConnectionAborted|std::io::ErrorKind::TimedOut|std::io::ErrorKind::BrokenPipe));
    let message = format!("VNC authentication or security negotiation failed: {error}");
    if network {
        Failure::Network(message)
    } else {
        Failure::Permanent(message)
    }
}
