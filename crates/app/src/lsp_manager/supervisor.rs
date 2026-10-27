use super::*;

impl LspManager {
    /// Whether [`ensure_server`](Self::ensure_server) may try a spawn now.
    pub(super) fn spawn_allowed(&self, language: &'static str) -> bool {
        !self.unavailable.contains_key(language)
            && self.failures.get(language).copied().unwrap_or(0) < MAX_CRASHES
            && self
                .cooled
                .get(language)
                .is_none_or(|at| at.elapsed() >= SPAWN_COOLDOWN)
    }

    /// Manual recovery for one buffer: forget crash backoff and cached
    /// answers, then stop every server for its language. The next frame
    /// re-submits the open text, which respawns the server and makes it
    /// index again. A binary installed after startup is found this time.
    pub fn restart(&mut self, path: &Path) {
        let Some(language) = self.docs.get(path).map(|doc| doc.server.language) else {
            return;
        };
        self.restart_servers(Some(language));
    }

    /// [`restart`](Self::restart) for every language, used by the
    /// Problems panel where no single editor is implied.
    pub fn restart_all(&mut self) {
        self.restart_servers(None);
    }

    pub(super) fn restart_servers(&mut self, language: Option<&'static str>) {
        if let Some(language) = language {
            self.unavailable.remove(language);
            self.failures.remove(language);
            self.cooled.remove(language);
        } else {
            self.unavailable.clear();
            self.failures.clear();
            self.cooled.clear();
        }
        let keys: Vec<ServerKey> = self
            .servers
            .keys()
            .filter(|key| language.is_none_or(|wanted| key.language == wanted))
            .cloned()
            .collect();
        for key in keys {
            self.shutdown_server(&key);
        }
        self.symbols.clear();
        self.symbols_rev.clear();
        self.symbols_retry.clear();
        self.inlay.clear();
        self.inlay_retry.clear();
        self.search_revision = self.search_revision.wrapping_add(1);
    }

    /// A running server for `key`, spawning one when needed. `None`
    /// means unavailable (binary missing, spawn failed, crash loop):
    /// the reason lands in `unavailable` for hover feedback.
    pub(super) fn ensure_server(&mut self, key: &ServerKey) -> Option<&mut Server> {
        if self.servers.contains_key(key) {
            return self.servers.get_mut(key);
        }
        if self.unavailable.contains_key(key.language) {
            return None;
        }
        // Crash loops back off, then give up with a reason.
        if self.failures.get(key.language).copied().unwrap_or(0) >= MAX_CRASHES {
            self.unavailable.insert(
                key.language,
                format!("{} keeps crashing; restart the app to retry", key.language),
            );
            return None;
        }
        if self
            .cooled
            .get(key.language)
            .is_some_and(|at| at.elapsed() < SPAWN_COOLDOWN)
        {
            return None;
        }
        let Some(path) = self.resolve_server(key.language) else {
            self.unavailable
                .insert(key.language, self.missing_server_note(key.language));
            return None;
        };
        if self.services.is_some() {
            return self.spawn_supervised(key, &path);
        }
        match spawn_server_direct(key, &path, self.pump_tx.clone()) {
            Ok(server) => {
                self.servers.insert(key.clone(), server);
                self.servers.get_mut(key)
            }
            Err(note) => {
                self.note_failure(key.language);
                self.unavailable.insert(key.language, note);
                None
            }
        }
    }

    /// Server executable for one language: the Settings override when
    /// set (absolute path or bare name), else the default candidates,
    /// including the usual GUI-launch directories and `~/.cargo/bin`.
    pub(super) fn resolve_server(&self, language: &'static str) -> Option<PathBuf> {
        if let Some(custom) = self
            .overrides
            .get(language)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return terminator_core::find_executable(custom);
        }
        terminator_native_edit::lsp::server_candidates(language_of(language))
            .iter()
            .find_map(|candidate| terminator_core::find_executable(candidate))
    }

    /// Why no executable was found, naming the Settings row that fixes it.
    pub(super) fn missing_server_note(&self, language: &'static str) -> String {
        if let Some(custom) = self
            .overrides
            .get(language)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return format!(
                "Language server not found: {custom}. Fix the path in Settings → Terminal & Editor."
            );
        }
        let candidates = terminator_native_edit::lsp::server_candidates(language_of(language));
        format!(
            "No language server on PATH ({}); install it or set a path in Settings → Terminal & Editor.",
            candidates.join(", ")
        )
    }

    /// Insert an optimistic entry for `key` and start its process plus
    /// stdio pumps as a supervisor actor. Needs no Tokio runtime on the
    /// calling thread: channels and the submit are plain sends. The
    /// handshake (`initialize`) queues on the inbox at once, so a slow
    /// start behaves exactly like the direct path; a failed `exec`
    /// reports back as [`PumpEvent::SpawnFailed`].
    pub(super) fn spawn_supervised(
        &mut self,
        key: &ServerKey,
        binary: &Path,
    ) -> Option<&mut Server> {
        let services = self.services.clone()?;
        let (inbox, incoming) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let cancel = CancellationToken::new();
        let actor_cancel = cancel.clone();
        self.servers.insert(
            key.clone(),
            Server {
                inbox,
                child: None,
                cancel: Some(cancel),
                ids: L::IdGen::default(),
                pending: HashMap::new(),
                init_id: None,
                ready: false,
            },
        );
        let actor_key = key.clone();
        let root = key.root.clone();
        let binary = binary.to_owned();
        let pump = self.pump_tx.clone();
        // One supervised task per (language, root): the supervisor
        // admits a single active task per key, so keying by language
        // alone would park every second project's server behind the
        // first forever.
        let context = OperationContext::new("lsp", server_resource(key), Policy::ServiceLifetime);
        if services
            .handle()
            .submit(context, actor_cancel.clone(), async move {
                run_server_actor(actor_key, binary, root, incoming, pump, actor_cancel).await?;
                Ok(Vec::new())
            })
            .is_err()
        {
            self.servers.remove(key);
            self.note_failure(key.language);
            self.unavailable
                .insert(key.language, format!("Could not start {}", key.language));
            return None;
        }
        let server = self.servers.get_mut(key)?;
        let L::RequestId::Number(init) = server.ids.next_id() else {
            self.servers.remove(key);
            self.note_failure(key.language);
            return None;
        };
        let root_uri = L::file_uri(&key.root);
        let Some(root_uri) = root_uri else {
            self.servers.remove(key);
            self.note_failure(key.language);
            return None;
        };
        let bytes = L::initialize(&L::RequestId::Number(init), std::process::id(), &root_uri);
        if server.inbox.send(bytes).is_err() {
            self.servers.remove(key);
            self.note_failure(key.language);
            return None;
        }
        server.init_id = Some(init);
        self.servers.get_mut(key)
    }

    /// A supervised spawn that failed inside the actor: same outcome as a
    /// direct spawn error, reported asynchronously. Parked text survives
    /// for the next respawn's handshake flush.
    pub(super) fn on_spawn_failed(&mut self, key: &ServerKey, note: &str) {
        if self.servers.remove(key).is_some() {
            self.note_failure(key.language);
            self.unavailable.insert(key.language, note.to_owned());
            for doc in self.docs.values_mut().filter(|doc| doc.server == *key) {
                doc.opened = false;
            }
        }
    }

    pub(super) fn note_failure(&mut self, language: &'static str) {
        let count = self.failures.get(language).copied().unwrap_or(0);
        self.failures.insert(language, count.saturating_add(1));
        self.cooled.insert(language, Instant::now());
    }

    pub(super) fn on_server_down(&mut self, key: &ServerKey) {
        // Stray exits (a supervised actor reaped just after a deliberate
        // shutdown removed the entry) are not crashes.
        if self.servers.remove(key).is_none() {
            return;
        }
        self.workspace_symbols.remove(key);
        self.search_revision = self.search_revision.wrapping_add(1);
        self.note_failure(key.language);
        // Docs re-open (and the server respawns, cooldown permitting)
        // on the next sync; pending answers are dead with their ids.
        for doc in self.docs.values_mut().filter(|doc| doc.server == *key) {
            doc.opened = false;
        }
    }

    pub(super) fn shutdown_server(&mut self, key: &ServerKey) {
        if let Some(mut server) = self.servers.remove(key) {
            let id = server.ids.next_id();
            let _ = server.inbox.send(L::shutdown(&id));
            let _ = server.inbox.send(L::exit());
            if let Some(mut child) = server.child {
                let _ = child.start_kill();
            }
            if let Some(cancel) = server.cancel {
                cancel.cancel();
            }
        }
        self.workspace_symbols.remove(key);
        self.search_revision = self.search_revision.wrapping_add(1);
        for doc in self.docs.values_mut().filter(|doc| doc.server == *key) {
            doc.opened = false;
        }
    }
}

/// Supervisor actor for one server: spawn the child with piped stdio
/// in `root` and pump both directions. Runs on the supervisor's Tokio
/// runtime, so process startup and `tokio::spawn` never touch the GUI
/// thread. A failed `exec` reports [`PumpEvent::SpawnFailed`]; a live
/// child forwards framed bodies and reports [`PumpEvent::Down`] when it
/// exits. Cancellation (deliberate shutdown) kills the child, reaps it,
/// and stays silent, so prunes never count as crashes.
async fn run_server_actor(
    key: ServerKey,
    binary: PathBuf,
    root: PathBuf,
    mut incoming: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    pump: tokio::sync::mpsc::UnboundedSender<PumpEvent>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let mut child = match tokio::process::Command::new(&binary)
        .args(L::server_args(language_of(key.language)))
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = pump.send(PumpEvent::SpawnFailed {
                key: key.clone(),
                note: format!("Could not start {}: {error:#}", key.language),
            });
            return Ok(());
        }
    };
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let writer = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt as _;
        let mut stdin = stdin;
        while let Some(bytes) = incoming.recv().await {
            let ok = match stdin.as_mut() {
                Some(pipe) => pipe.write_all(&bytes).await.is_ok() && pipe.flush().await.is_ok(),
                None => false,
            };
            if !ok {
                break;
            }
        }
    });
    let reader_pump = pump.clone();
    let reader_key = key.clone();
    let reader = tokio::spawn(async move {
        use tokio::io::AsyncReadExt as _;
        let mut stdout = stdout;
        let mut decoder = L::FrameDecoder::default();
        let mut chunk = [0u8; 8192];
        while let Some(pipe) = stdout.as_mut() {
            match pipe.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut stop = false;
                    for body in decoder.feed(chunk.get(..n).unwrap_or(&[])) {
                        if reader_pump
                            .send(PumpEvent::Body {
                                key: reader_key.clone(),
                                body,
                            })
                            .is_err()
                        {
                            stop = true;
                            break;
                        }
                    }
                    if stop {
                        break;
                    }
                }
            }
        }
    });
    // The child outlives the pumps on crash: wait for the process
    // itself, then tear the pumps down.
    let exited = tokio::select! {
        result = child.wait() => {
            let _ = result;
            true
        }
        () = cancel.cancelled() => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            false
        }
    };
    writer.abort();
    reader.abort();
    if exited && !cancel.is_cancelled() {
        let _ = pump.send(PumpEvent::Down { key });
    }
    Ok(())
}

/// Spawn one server with piped stdio in `root`, handshake it, and pump
/// both directions on background tasks. Needs a running tokio runtime;
/// only the no-services test path uses this (tests use `#[tokio::test]`).
/// The GUI path goes through [`LspManager::spawn_supervised`] instead.
fn spawn_server_direct(
    key: &ServerKey,
    binary: &Path,
    pump: tokio::sync::mpsc::UnboundedSender<PumpEvent>,
) -> Result<Server, String> {
    let mut child = tokio::process::Command::new(binary)
        .args(L::server_args(language_of(key.language)))
        .current_dir(&key.root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start {}: {error:#}", key.language))?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let (inbox, mut incoming) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt as _;
        let mut stdin = stdin;
        while let Some(bytes) = incoming.recv().await {
            let ok = match stdin.as_mut() {
                Some(pipe) => pipe.write_all(&bytes).await.is_ok() && pipe.flush().await.is_ok(),
                None => false,
            };
            if !ok {
                break;
            }
        }
    });
    let reader_pump = pump.clone();
    let reader_key = key.clone();
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt as _;
        let mut stdout = stdout;
        let mut decoder = L::FrameDecoder::default();
        let mut chunk = [0u8; 8192];
        while let Some(pipe) = stdout.as_mut() {
            match pipe.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut stop = false;
                    for body in decoder.feed(chunk.get(..n).unwrap_or(&[])) {
                        if reader_pump
                            .send(PumpEvent::Body {
                                key: reader_key.clone(),
                                body,
                            })
                            .is_err()
                        {
                            stop = true;
                            break;
                        }
                    }
                    if stop {
                        break;
                    }
                }
            }
        }
        let _ = pump.send(PumpEvent::Down { key: reader_key });
    });
    let mut server = Server {
        inbox,
        child: Some(child),
        cancel: None,
        ids: L::IdGen::default(),
        pending: HashMap::new(),
        init_id: None,
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
