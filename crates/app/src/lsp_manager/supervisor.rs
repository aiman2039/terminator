use super::process::{RunServerActorInput, run_server_actor, spawn_server_direct};
use super::*;

impl LspManager {
    /// Apply Settings → Languages server paths. Saving restarts
    /// only the servers whose path changed, so the new binary is used
    /// without restarting the app. Unchanged settings do nothing.
    pub fn set_server_overrides(&mut self, overrides: &std::collections::BTreeMap<String, String>) {
        // User-defined servers share the map (`custom:` keys); a change
        // there restarts the servers they replace or add.
        let normalized: std::collections::BTreeMap<String, String> = overrides
            .iter()
            .map(|(language, value)| (language.clone(), value.trim().to_owned()))
            .filter(|(_, value)| !value.is_empty())
            .collect();
        if normalized == self.overrides {
            return;
        }
        let before_custom = L::custom_families();
        L::set_custom_servers(&L::parse_custom_servers(&normalized));
        let after_custom = L::custom_families();
        let mut changed: Vec<&'static str> = Vec::new();
        for language in L::SERVER_FAMILIES.iter().map(|family| family.id) {
            let before = self.overrides.get(language);
            let after = normalized.get(language);
            if before != after {
                changed.push(language);
            }
        }
        if before_custom != after_custom {
            // A user server can take a built-in family's files: restart
            // every language server so documents re-key on the next sync.
            changed.extend(self.servers.keys().map(|key| key.language));
            changed.extend(before_custom.iter().map(|family| family.id));
        }
        changed.sort_unstable();
        changed.dedup();
        self.overrides = normalized;
        for &language in &changed {
            self.unavailable.remove(language);
            self.failures.remove(language);
            self.cooled.remove(language);
            let keys: Vec<ServerKey> = self
                .servers
                .keys()
                .filter(|key| key.language == language)
                .cloned()
                .collect();
            for key in keys {
                self.shutdown_server(&key);
            }
        }
        if !changed.is_empty() {
            self.symbols.clear();
            self.symbols_rev.clear();
            self.symbols_retry.clear();
            self.inlay.clear();
            self.inlay_retry.clear();
            self.hover = hover::HoverCache::default();
            self.search_revision = self.search_revision.wrapping_add(1);
        }
    }

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
        // Never run project code (build scripts, proc macros) unasked.
        if !self.key_trusted(key) {
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
                self.init_started.insert(key.clone(), Instant::now());
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
        if let Some(custom) = self.path_override(language) {
            return terminator_core::find_executable(custom);
        }
        L::family_candidates(language)
            .iter()
            .find_map(|candidate| terminator_core::find_executable(candidate))
    }

    /// The Settings path for a built-in family. User-defined servers
    /// (`custom:` keys) hold their whole definition there, not a path.
    fn path_override(&self, language: &'static str) -> Option<&str> {
        if language.starts_with(L::CUSTOM_PREFIX) {
            return None;
        }
        self.overrides
            .get(language)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
    }

    /// Why no executable was found, naming the Settings row that fixes it
    /// and how to install the server.
    pub(super) fn missing_server_note(&self, language: &'static str) -> String {
        if let Some(custom) = self.path_override(language) {
            return format!(
                "Language server not found: {custom}. Fix the path in Settings → Languages."
            );
        }
        let candidates = L::family_candidates(language);
        let install = L::family_install(language);
        let hint = if install.is_empty() {
            String::new()
        } else {
            format!(" Install: {install}")
        };
        format!(
            "No language server on PATH ({}); install it or set a path in Settings → Languages.{hint}",
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
                capabilities: serde_json::Value::Null,
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
                run_server_actor(RunServerActorInput {
                    key: actor_key,
                    binary,
                    root,
                    incoming,
                    pump,
                    cancel: actor_cancel,
                })
                .await?;
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
        self.init_started.insert(key.clone(), Instant::now());
        self.servers.get_mut(key)
    }

    /// Stop servers that left `initialize` unanswered past
    /// [`INIT_TIMEOUT`] and show why, like a failed `initialize`.
    pub(super) fn expire_silent_starts(&mut self) {
        let servers = &self.servers;
        self.init_started.retain(|key, _| {
            servers
                .get(key)
                .is_some_and(|server| server.init_id.is_some())
        });
        let silent: Vec<ServerKey> = self
            .init_started
            .iter()
            .filter(|(_, at)| at.elapsed() >= INIT_TIMEOUT)
            .map(|(key, _)| key.clone())
            .collect();
        for key in silent {
            self.init_started.remove(&key);
            self.shutdown_server(&key);
            self.note_failure(key.language);
            self.unavailable.insert(
                key.language,
                format!(
                    "Language server failed to start: {} did not answer within {} s. Restart it to try again.",
                    key.language,
                    INIT_TIMEOUT.as_secs()
                ),
            );
        }
    }

    /// A supervised spawn that failed inside the actor: same outcome as a
    /// direct spawn error, reported asynchronously. Parked text survives
    /// for the next respawn's handshake flush.
    pub(super) fn on_spawn_failed(&mut self, key: &ServerKey, note: &str) {
        if self.servers.remove(key).is_some() {
            self.note_failure(key.language);
            self.unavailable.insert(key.language, note.to_owned());
            for doc in self.all_docs_mut().filter(|doc| doc.server == *key) {
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
        let Some(server) = self.servers.remove(key) else {
            return;
        };
        self.fail_pending(key, server.pending);
        self.workspace_symbols.remove(key);
        self.search_revision = self.search_revision.wrapping_add(1);
        self.note_failure(key.language);
        // Docs re-open (and the server respawns, cooldown permitting)
        // on the next sync; pending answers are dead with their ids.
        for doc in self.all_docs_mut().filter(|doc| doc.server == *key) {
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
            self.fail_pending(key, server.pending);
        }
        self.workspace_symbols.remove(key);
        self.search_revision = self.search_revision.wrapping_add(1);
        for doc in self.all_docs_mut().filter(|doc| doc.server == *key) {
            doc.opened = false;
        }
    }
}
