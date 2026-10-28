use super::*;

impl LspManager {
    pub fn document_version(&self, path: &Path) -> Option<i32> {
        self.docs
            .get(path)
            .filter(|doc| doc.opened)
            .map(|doc| doc.version)
    }

    pub fn invalidate_paths(&mut self, paths: &[PathBuf]) {
        for path in paths {
            self.close_path(path);
        }
    }

    /// Ask for the document outline of a tracked buffer. Cheap to call
    /// every frame: it only sends when the submitted revision moved past
    /// the cached answer and no request for the file is in flight.
    pub fn request_symbols(&mut self, path: &Path) {
        let Some(doc) = self.docs.get(path) else {
            return;
        };
        let (key, uri, revision) = (doc.server.clone(), doc.uri.clone(), doc.submitted);
        if self.symbols_rev.get(path) == Some(&revision) {
            // An empty outline usually means the server is still indexing.
            // Keep asking on a slow cadence until symbols arrive instead
            // of caching the empty answer until the next edit.
            let empty = self
                .symbols
                .get(path)
                .is_none_or(|symbols| symbols.is_empty());
            if !empty
                || self
                    .symbols_retry
                    .get(path)
                    .is_some_and(|at| at.elapsed() < SYMBOLS_RETRY)
            {
                return;
            }
        }
        let Some(server) = self.servers.get_mut(&key) else {
            return;
        };
        if !server.ready
            || server
                .pending
                .values()
                .any(|pending| matches!(pending, Pending::Symbols { path: p, .. } if p == path))
        {
            return;
        }
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return;
        };
        let _ = server
            .inbox
            .send(L::document_symbol(&L::RequestId::Number(number), &uri));
        server.pending.insert(
            number,
            Pending::Symbols {
                path: path.to_owned(),
                revision,
            },
        );
    }

    /// Ask for inlay hints over a tracked buffer's `lines` lines, once per
    /// submitted revision (like [`Self::request_symbols`]); empty or failed
    /// answers are retried on a slow cadence while the server indexes.
    pub fn request_inlay_hints(&mut self, path: &Path, lines: usize) {
        let Some(doc) = self.docs.get(path) else {
            return;
        };
        let (key, uri, revision) = (doc.server.clone(), doc.uri.clone(), doc.submitted);
        if self
            .inlay_retry
            .get(path)
            .is_some_and(|at| at.elapsed() < SYMBOLS_RETRY)
            || self
                .inlay
                .get(path)
                .is_some_and(|(answered, hints)| *answered == revision && !hints.is_empty())
        {
            return;
        }
        let Some(server) = self.servers.get_mut(&key) else {
            return;
        };
        if !server.ready
            || server
                .pending
                .values()
                .any(|pending| matches!(pending, Pending::InlayHints { path: p, .. } if p == path))
        {
            return;
        }
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return;
        };
        let lines = u32::try_from(lines).unwrap_or(u32::MAX);
        let _ = server
            .inbox
            .send(L::inlay_hint(&L::RequestId::Number(number), &uri, lines));
        server.pending.insert(
            number,
            Pending::InlayHints {
                path: path.to_owned(),
                revision,
            },
        );
    }

    /// Latest inlay hints for `path` and the buffer revision they describe.
    #[must_use]
    pub fn inlay_hints(&self, path: &Path) -> Option<(u64, std::sync::Arc<[L::InlayHint]>)> {
        self.inlay.get(path).cloned()
    }

    /// Seed a cached outline, standing in for a server answer.
    #[cfg(test)]
    pub fn set_symbols_for_test(&mut self, path: &Path, symbols: Vec<L::Symbol>) {
        self.symbols.insert(path.to_owned(), symbols.into());
    }

    /// Cached outline for `path`, in document order (depth-tagged).
    /// Shared, so per-frame callers clone a pointer, not the outline.
    #[must_use]
    pub fn symbols_for(&self, path: &Path) -> std::sync::Arc<[L::Symbol]> {
        self.symbols
            .get(path)
            .cloned()
            .unwrap_or_else(|| std::sync::Arc::from(Vec::new()))
    }

    /// Request fresh document outlines for every open native editor. The
    /// per-document cache and pending guard keep repeated calls cheap.
    pub fn request_open_document_symbols(&mut self) {
        let paths: Vec<PathBuf> = self.docs.keys().cloned().collect();
        for path in paths {
            self.request_symbols(&path);
        }
    }

    /// Cached document symbols from all open native buffers.
    pub fn all_document_symbols(&self) -> Vec<(PathBuf, L::Symbol)> {
        self.symbols
            .iter()
            .flat_map(|(path, symbols)| {
                symbols.iter().cloned().map(|symbol| (path.clone(), symbol))
            })
            .collect()
    }

    pub fn clear_workspace_symbols(&mut self) {
        if self.workspace_symbol_query.is_empty()
            && self.workspace_symbols.is_empty()
            && self.workspace_symbol_root.is_none()
        {
            return;
        }
        self.workspace_symbol_query.clear();
        self.workspace_symbol_root = None;
        self.workspace_symbol_generation = self.workspace_symbol_generation.wrapping_add(1);
        self.workspace_symbols.clear();
        self.search_revision = self.search_revision.wrapping_add(1);
        for server in self.servers.values_mut() {
            server
                .pending
                .retain(|_, pending| !matches!(pending, Pending::WorkspaceSymbols { .. }));
        }
    }

    /// Ask each ready project server for matching symbols once per query.
    /// `root` keeps the request on servers for that project. A later query or
    /// a different root discards old pending replies by generation.
    pub fn request_scoped_symbols(&mut self, query: &str, root: Option<&Path>) {
        let query = query.trim();
        if query.is_empty() {
            self.clear_workspace_symbols();
            return;
        }
        let root_changed = match (&self.workspace_symbol_root, root) {
            (None, None) => false,
            (Some(current), Some(next)) => current.as_path() != next,
            _ => true,
        };
        if self.workspace_symbol_query != query || root_changed {
            self.clear_workspace_symbols();
            query.clone_into(&mut self.workspace_symbol_query);
            self.workspace_symbol_root = root.map(Path::to_path_buf);
        }
        let generation = self.workspace_symbol_generation;
        for (key, server) in &mut self.servers {
            if self.workspace_symbol_root.as_ref().is_some_and(|root| {
                key.root != *root && !key.root.starts_with(root) && !root.starts_with(&key.root)
            }) {
                continue;
            }
            if !server.ready || self.workspace_symbols.contains_key(key)
                || server.pending.values().any(|pending| matches!(pending, Pending::WorkspaceSymbols { generation: pending_generation } if *pending_generation == generation))
            {
                continue;
            }
            let L::RequestId::Number(number) = server.ids.next_id() else {
                continue;
            };
            if server
                .inbox
                .send(L::workspace_symbol(&L::RequestId::Number(number), query))
                .is_ok()
            {
                server
                    .pending
                    .insert(number, Pending::WorkspaceSymbols { generation });
            }
        }
    }

    pub fn workspace_symbols(&self) -> Vec<L::WorkspaceSymbolHit> {
        self.workspace_symbols
            .values()
            .flat_map(|hits| hits.iter().cloned())
            .collect()
    }

    pub fn workspace_symbols_pending(&self) -> bool {
        self.servers.values().any(|server| server.pending.values().any(|pending| matches!(pending, Pending::WorkspaceSymbols { generation } if *generation == self.workspace_symbol_generation)))
    }

    pub fn search_revision(&self) -> u64 {
        self.search_revision
    }
}
