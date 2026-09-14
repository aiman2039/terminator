use crate::Update;
use anyhow::{Result, ensure};
use std::{
    fs,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use terminator_core::{
    Paths, read_frame,
    ui_control::{Envelope, Response},
    write_frame,
};
pub struct Server {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub fn spawn(
    paths: Paths,
    updates: mpsc::Sender<Update>,
    ctx: eframe::egui::Context,
) -> Result<Server> {
    let socket = paths.runtime.join("gui.sock");
    let _ = fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let running = stop.clone();
    let active = Arc::new(AtomicUsize::new(0));
    let thread = thread::spawn(move || {
        while !running.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    if active.fetch_add(1, Ordering::Relaxed) >= 16 {
                        active.fetch_sub(1, Ordering::Relaxed);
                        continue;
                    }
                    let active = active.clone();
                    let paths = paths.clone();
                    let updates = updates.clone();
                    let ctx = ctx.clone();
                    thread::spawn(move || {
                        let result = (|| -> Result<serde_json::Value> {
                            stream.set_nonblocking(false)?;
                            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                            stream.set_write_timeout(Some(Duration::from_secs(3)))?;
                            let envelope: Envelope = read_frame(&mut stream)?;
                            ensure!(
                                envelope.version == 1 && envelope.auth == paths.token()?,
                                "GUI authentication failed"
                            );
                            envelope.request.validate()?;
                            // Observational GUI requests must remain available
                            // during a daemon outage and report the GUI's actual
                            // state, rather than causing a reconnect themselves.
                            if !matches!(
                                envelope.request,
                                terminator_core::ui_control::Request::Snapshot
                                    | terminator_core::ui_control::Request::Ping
                            ) && let terminator_core::Response::State(state) =
                                terminator_core::rpc(&paths, terminator_core::Request::Snapshot)?
                            {
                                updates.send(Update::State(state))?;
                            }
                            let (tx, rx) = mpsc::sync_channel(1);
                            let deadline = Instant::now() + Duration::from_secs(5);
                            updates.send(Update::UiRequest(envelope.request, tx, deadline))?;
                            ctx.request_repaint();
                            rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))?
                                .map_err(anyhow::Error::msg)
                        })();
                        let response = match result {
                            Ok(value) => Response {
                                result: Some(value),
                                error: None,
                            },
                            Err(error) => Response {
                                result: None,
                                error: Some(error.to_string()),
                            },
                        };
                        let _ = write_frame(&mut stream, &response);
                        active.fetch_sub(1, Ordering::Relaxed);
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(25))
                }
                Err(_) => break,
            }
        }
        let _ = fs::remove_file(socket);
    });
    Ok(Server {
        stop,
        thread: Some(thread),
    })
}
