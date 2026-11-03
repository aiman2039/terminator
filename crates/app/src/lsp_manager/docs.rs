use super::*;

/// Which document map a server slot uses.
#[derive(Clone, Copy)]
pub(super) enum Slot {
    Main,
    Spell,
}

struct SlotSubmit<'a> {
    slot: Slot,
    path: &'a Path,
    key: ServerKey,
    protocol_id: &'static str,
    uri: url::Url,
    revision: u64,
    text: &'a str,
}

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
        let main = if L::server_language(language).is_some() {
            self.sync_slot(Slot::Main, path, revision)
        } else {
            self.close_path(path);
            false
        };
        let spell = if self.spell_enabled {
            self.sync_slot(Slot::Spell, path, revision)
        } else {
            false
        };
        main || spell
    }

    pub(super) fn docs_of(&mut self, slot: Slot) -> &mut HashMap<PathBuf, OpenDoc> {
        match slot {
            Slot::Main => &mut self.docs,
            Slot::Spell => &mut self.spell_docs,
        }
    }

    /// [`sync_doc`](Self::sync_doc) for one server slot.
    fn sync_slot(&mut self, slot: Slot, path: &Path, revision: u64) -> bool {
        let Some(doc) = self.docs_of(slot).get_mut(path) else {
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
            None => self.spawn_allowed(key.language) && self.key_trusted(&key),
        }
    }

    /// Servers that run no project code (the spell checker) need no trust.
    pub(super) fn key_trusted(&mut self, key: &ServerKey) -> bool {
        !L::family_needs_trust(key.language) || self.root_trusted(&key.root)
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
        // The spell slot keeps its own debounce: completion, hover, and
        // goto push live text often, and harper re-lints the whole file.
        if self.spell_enabled && self.sync_slot(Slot::Spell, path, revision) {
            self.submit_spell(path, language, (revision, text));
        }
        let Some(language_id) = L::server_language(language) else {
            return;
        };
        let Some(protocol_id) = L::language_id_for_path(path, language) else {
            return;
        };
        let key = self.server_key(path, language, language_id);
        let Some(uri) = L::file_uri(path) else {
            return;
        };
        self.submit_slot(SlotSubmit {
            slot: Slot::Main,
            path,
            key,
            protocol_id,
            uri,
            revision,
            text,
        });
    }

    /// The spell half of [`submit_text`](Self::submit_text).
    fn submit_spell(&mut self, path: &Path, language: Language, (revision, text): (u64, &str)) {
        let Some(uri) = L::file_uri(path) else {
            return;
        };
        // Reuse a known root (spell, then language doc) before walking up.
        let root = match self.spell_docs.get(path).or_else(|| self.docs.get(path)) {
            Some(doc) => doc.server.root.clone(),
            None => project_root(path, Language::Plain),
        };
        self.submit_slot(SlotSubmit {
            slot: Slot::Spell,
            path,
            key: ServerKey {
                language: L::SPELL_SERVER,
                root,
            },
            protocol_id: L::spell_language_id(path, language),
            uri,
            revision,
            text,
        });
    }

    fn submit_slot(&mut self, input: SlotSubmit<'_>) {
        let SlotSubmit {
            slot,
            path,
            key,
            protocol_id,
            uri,
            revision,
            text,
        } = input;
        let now = Instant::now();
        let doc = self
            .docs_of(slot)
            .entry(path.to_owned())
            .or_insert_with(|| OpenDoc {
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
                && let Some(doc) = self.docs_of(slot).get_mut(path)
            {
                doc.pending_text = Some(text.to_owned());
            }
            return;
        }
        let inbox = self.servers.get(&key).map(|server| server.inbox.clone());
        let Some(inbox) = inbox else {
            return;
        };
        let Some(doc) = self.docs_of(slot).get_mut(path) else {
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

    /// Server for one document. A tracked doc keeps its root: walking up
    /// to 20 ancestors per sync stalls typing on slow (network) disks.
    fn server_key(&self, path: &Path, language: Language, language_id: &'static str) -> ServerKey {
        let root = match self.docs.get(path) {
            Some(doc) if doc.server.language == language_id => doc.server.root.clone(),
            _ => project_root(path, language),
        };
        ServerKey {
            language: language_id,
            root,
        }
    }

    /// Close one spell document and drop its findings.
    pub(super) fn close_spell(&mut self, path: &Path) {
        self.close_doc(Slot::Spell, path);
        self.spell_diagnostics.remove(path);
    }

    /// Untrack one slot's document, `didClose` it when it was open.
    fn close_doc(&mut self, slot: Slot, path: &Path) {
        if let Some(doc) = self.docs_of(slot).remove(path)
            && doc.opened
            && let Some(server) = self.servers.get(&doc.server)
        {
            let _ = server.inbox.send(L::did_close(&doc.uri));
        }
    }

    /// Every tracked document, language and spell, for per-server sweeps.
    pub(super) fn all_docs_mut(&mut self) -> impl Iterator<Item = &mut OpenDoc> {
        self.docs.values_mut().chain(self.spell_docs.values_mut())
    }

    pub(super) fn close_path(&mut self, path: &Path) {
        self.close_doc(Slot::Main, path);
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
        for slot in [Slot::Main, Slot::Spell] {
            let gone: Vec<PathBuf> = self
                .docs_of(slot)
                .keys()
                .filter(|path| !live.contains(*path))
                .cloned()
                .collect();
            for path in &gone {
                match slot {
                    Slot::Main => self.close_path(path),
                    Slot::Spell => self.close_spell(path),
                }
            }
        }
        let needed: std::collections::HashSet<ServerKey> = self
            .docs
            .values()
            .chain(self.spell_docs.values())
            .map(|doc| doc.server.clone())
            .collect();
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
