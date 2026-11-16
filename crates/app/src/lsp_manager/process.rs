//! Server processes and their stdio pumps: the supervised actor, the
//! direct test-only spawn, and process-group cleanup.

use super::*;

/// Supervisor actor for one server: spawn the child with piped stdio
/// in `root` and pump both directions. Runs on the supervisor's Tokio
/// runtime, so process startup and `tokio::spawn` never touch the GUI
/// thread. A failed `exec` reports [`PumpEvent::SpawnFailed`]; a live
/// child forwards framed bodies and reports [`PumpEvent::Down`] when it
/// exits. Cancellation (deliberate shutdown) kills the child, reaps it,
/// and stays silent, so prunes never count as crashes.
pub(crate) struct RunServerActorInput {
    pub(crate) key: ServerKey,
    pub(crate) binary: PathBuf,
    pub(crate) root: PathBuf,
    pub(crate) incoming: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    pub(crate) pump: tokio::sync::mpsc::UnboundedSender<PumpEvent>,
    pub(crate) cancel: CancellationToken,
}

pub(super) async fn run_server_actor(input: RunServerActorInput) -> anyhow::Result<()> {
    let RunServerActorInput {
        key,
        binary,
        root,
        incoming,
        pump,
        cancel,
    } = input;
    let mut command = tokio::process::Command::new(&binary);
    command
        .args(L::family_args(key.language))
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    // Its own process group, so its children (`cargo check`, `rustc`)
    // end with it instead of holding the build lock.
    #[cfg(unix)]
    command.process_group(0);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = pump.send(PumpEvent::SpawnFailed {
                key: key.clone(),
                note: format!("Could not start {}: {error:#}", key.language),
            });
            return Ok(());
        }
    };
    let group = child.id();
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let writer = tokio::spawn(ServerPipes::write_stdin(stdin, incoming));
    let reader_pump = pump.clone();
    let reader_key = key.clone();
    let mut reader = tokio::spawn(async move {
        ServerPipes::read_stdout(stdout, &reader_pump, &reader_key).await;
    });
    // The child outlives the pumps on crash: wait for the process
    // itself, then tear the pumps down. A reader that stops (stdout
    // closed, or a frame over the size limits) leaves the server
    // unusable: kill it and report it down.
    let exited = tokio::select! {
        result = child.wait() => {
            let _ = result;
            true
        }
        _ = &mut reader => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            true
        }
        () = cancel.cancelled() => {
            // `shutdown` and `exit` were queued before the cancel: give
            // the server a moment to read them and exit cleanly.
            if tokio::time::timeout(SHUTDOWN_GRACE, child.wait()).await.is_err() {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
            false
        }
    };
    end_process_group(group);
    writer.abort();
    reader.abort();
    if exited && !cancel.is_cancelled() {
        let _ = pump.send(PumpEvent::Down { key });
    }
    Ok(())
}

/// How long a server may take to read `shutdown`/`exit` before SIGKILL.
pub(super) const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// Kill what is left of a server's process group (its build children).
/// The group outlives its leader while a member runs, so its id is not
/// reused yet; an empty group answers ESRCH, which is ignored.
fn end_process_group(group: Option<u32>) {
    #[cfg(unix)]
    if let Some(group) = group {
        let _ = terminator_core::signals::signal_group(
            group,
            terminator_core::signals::ProcSignal::Kill,
        );
    }
    #[cfg(not(unix))]
    let _ = group;
}

/// The two stdio pumps between a server process and the manager.
struct ServerPipes;

impl ServerPipes {
    /// Write each queued frame to the server's stdin until it closes.
    async fn write_stdin(
        mut stdin: Option<tokio::process::ChildStdin>,
        mut incoming: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        use tokio::io::AsyncWriteExt as _;
        while let Some(bytes) = incoming.recv().await {
            let ok = match stdin.as_mut() {
                Some(pipe) => pipe.write_all(&bytes).await.is_ok() && pipe.flush().await.is_ok(),
                None => false,
            };
            if !ok {
                break;
            }
        }
    }

    /// Decode framed bodies from the server's stdout and forward them
    /// until it closes or the manager stops listening.
    async fn read_stdout(
        mut stdout: Option<tokio::process::ChildStdout>,
        pump: &tokio::sync::mpsc::UnboundedSender<PumpEvent>,
        key: &ServerKey,
    ) {
        use tokio::io::AsyncReadExt as _;
        let mut decoder = L::FrameDecoder::default();
        let mut chunk = [0u8; 8192];
        while let Some(pipe) = stdout.as_mut() {
            let n = match pipe.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let bodies = decoder.feed(chunk.get(..n).unwrap_or(&[]));
            if !Self::forward(bodies, pump, key) || decoder.overflowed() {
                break;
            }
        }
    }

    /// Parse each body here, off the GUI thread, and forward what the
    /// manager routes. False once the manager's receiver is gone.
    fn forward(
        bodies: impl IntoIterator<Item = Vec<u8>>,
        pump: &tokio::sync::mpsc::UnboundedSender<PumpEvent>,
        key: &ServerKey,
    ) -> bool {
        bodies
            .into_iter()
            .filter_map(|body| responses::decode_body(key, &body))
            .all(|event| pump.send(event).is_ok())
    }
}

/// Spawn one server with piped stdio in `root`, handshake it, and pump
/// both directions on background tasks. Needs a running tokio runtime;
/// only the no-services test path uses this (tests use `#[tokio::test]`).
/// The GUI path goes through [`LspManager::spawn_supervised`] instead.
pub(super) fn spawn_server_direct(
    key: &ServerKey,
    binary: &Path,
    pump: tokio::sync::mpsc::UnboundedSender<PumpEvent>,
) -> Result<Server, String> {
    let mut child = tokio::process::Command::new(binary)
        .args(L::family_args(key.language))
        .current_dir(&key.root)
        // This path is only for tests. A release launched with `cargo run`
        // holds the shared target's build lock while tests run. Keep the
        // server's cargo check in its fixture instead of waiting on that lock.
        .env_remove("CARGO_TARGET_DIR")
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start {}: {error:#}", key.language))?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let (inbox, incoming) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(ServerPipes::write_stdin(stdin, incoming));
    let reader_pump = pump.clone();
    let reader_key = key.clone();
    tokio::spawn(async move {
        ServerPipes::read_stdout(stdout, &reader_pump, &reader_key).await;
        let _ = pump.send(PumpEvent::Down { key: reader_key });
    });
    let mut server = Server {
        inbox,
        child: Some(child),
        cancel: None,
        ids: L::IdGen::default(),
        pending: HashMap::new(),
        init_id: None,
        capabilities: serde_json::Value::Null,
        ready: false,
    };
    let L::RequestId::Number(init) = server.ids.next_id() else {
        return Err(format!("Could not start {}", key.language));
    };
    let root = L::file_uri(&key.root).ok_or_else(|| format!("No file URI for {}", key.language))?;
    let bytes = L::initialize(&L::RequestId::Number(init), std::process::id(), &root);
    if server.inbox.send(bytes).is_err() {
        return Err(format!("{} exited immediately", key.language));
    }
    server.init_id = Some(init);
    Ok(server)
}
