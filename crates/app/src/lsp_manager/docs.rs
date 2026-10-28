use super::*;

pub(crate) struct SubmitTextInput<'a> {
    pub path: &'a Path,
    pub language: Language,
    pub revision: u64,
    pub text: &'a str,
}

impl LspManager {
    /// Per-frame sync for one loaded buffer. Returns true when the
    /// caller should clone the text and hand it to [`submit_text`]:
    /// unknown buffers, unopened buffers with a ready (or missing)
    /// server, and edits older than the debounce. Unsupported languages
    /// untrack (and close) silently.
    pub fn sync_doc(&mut self, path: &Path, language: Language, revision: u64) -> bool {
        if L::server_language(language).is_none() {
            self.close_path(path);
            return false;
        }
        let Some(doc) = self.docs.get_mut(path) else {
            return true;
        };
        if doc.observed != revision {
            doc.observed = revision;
            doc.last_change = Instant::now();
        }
        if doc.opened {
            return doc.submitted != revision && doc.last_change.elapsed() >= SYNC_DEBOUNCE;
        }
        // Open as soon as a server can take it; a starting server waits, a
        // dead one respawns in submit. A missing, crash-looping, or cooling
        // server would refuse the respawn, so do not clone the buffer and
        // walk the project root every frame until it could start again.
        let key = doc.server.clone();
        match self.servers.get(&key) {
            Some(server) => server.ready,
            None => self.spawn_allowed(key.language),
        }
    }

    /// Open or refresh one buffer on its server. Always records the
    /// revision (latest text wins). Text submitted while the server is
    /// still starting parks on the doc and flushes as `didOpen` when the
    /// handshake completes, so a slow start neither blocks typing nor
    /// drops the open.
    pub fn submit_text(&mut self, input: SubmitTextInput<'_>) {
        let SubmitTextInput {
            path,
            language,
            revision,
            text,
        } = input;
        let Some(language_id) = L::server_language(language) else {
            return;
        };
        let Some(protocol_id) = L::language_id_for_path(path, language) else {
            return;
        };
        let root = project_root(path, language);
        let key = ServerKey {
            language: language_id,
            root,
        };
        let Some(uri) = L::file_uri(path) else {
            return;
        };
        let now = Instant::now();
        let doc = self.docs.entry(path.to_owned()).or_insert_with(|| OpenDoc {
            uri,
            version: 0,
            server: key.clone(),
            language_id: protocol_id,
            submitted: revision,
            observed: revision,
            last_change: now,
            opened: false,
            pending_text: None,
        });
        if doc.server != key {
            doc.server = key.clone();
            doc.opened = false;
        }
        doc.language_id = protocol_id;
        let was_submitted = doc.submitted;
        doc.submitted = revision;
        doc.observed = revision;
        if !self.servers.get(&key).is_some_and(|server| server.ready) {
            let _ = self.ensure_server(&key);
            // Park the latest text for the handshake flush, but only
            // while a server is actually starting: an unavailable server
            // would pile a copy per frame with nobody to flush it.
            if self.servers.contains_key(&key)
                && let Some(doc) = self.docs.get_mut(path)
            {
                doc.pending_text = Some(text.to_owned());
            }
            return;
        }
        let inbox = self.servers.get(&key).map(|server| server.inbox.clone());
        let Some(inbox) = inbox else {
            return;
        };
        let Some(doc) = self.docs.get_mut(path) else {
            return;
        };
        // A parked open is already flushed or superseded; either way the
        // live text below is what the server must hold.
        doc.pending_text = None;
        if !doc.opened {
            doc.version = doc.version.saturating_add(1);
            let bytes = L::did_open(L::DidOpenInput {
                uri: &doc.uri,
                language_id: doc.language_id,
                version: doc.version,
                text,
            });
            let _ = inbox.send(bytes);
            doc.opened = true;
        } else if was_submitted != revision {
            doc.version = doc.version.saturating_add(1);
            let bytes = L::did_change(&doc.uri, doc.version, text);
            let _ = inbox.send(bytes);
        }
    }

    pub(super) fn close_path(&mut self, path: &Path) {
        if let Some(doc) = self.docs.remove(path)
            && doc.opened
            && let Some(server) = self.servers.get(&doc.server)
        {
            let _ = server.inbox.send(L::did_close(&doc.uri));
        }
        self.diagnostics.remove(path);
        let had_symbols = self.symbols.remove(path).is_some();
        self.symbols_rev.remove(path);
        self.symbols_retry.remove(path);
        self.inlay.remove(path);
        self.inlay_retry.remove(path);
        if had_symbols {
            self.search_revision = self.search_revision.wrapping_add(1);
        }
        if self.hover.as_ref().is_some_and(|popup| popup.path == path) {
            self.hover = None;
        }
        if self
            .references
            .as_ref()
            .is_some_and(|result| result.from == path)
        {
            self.references = None;
        }
        if self
            .rename
            .as_ref()
            .is_some_and(|result| result.from == path)
        {
            self.rename = None;
        }
    }

    /// Forget closed tabs: `didClose` open buffers, drop their state,
    /// and shut down servers nobody needs anymore.
    pub fn prune(&mut self, live: &std::collections::HashSet<PathBuf>) {
        let gone: Vec<PathBuf> = self
            .docs
            .keys()
            .filter(|path| !live.contains(*path))
            .cloned()
            .collect();
        for path in &gone {
            self.close_path(path);
        }
        let needed: std::collections::HashSet<ServerKey> =
            self.docs.values().map(|doc| doc.server.clone()).collect();
        let idle: Vec<ServerKey> = self
            .servers
            .keys()
            .filter(|key| !needed.contains(*key))
            .cloned()
            .collect();
        for key in &idle {
            self.shutdown_server(key);
        }
    }
}
