use super::*;

impl LspManager {
    pub(super) fn on_body(&mut self, key: &ServerKey, body: &[u8]) {
        let Some(message) = L::parse_message(body) else {
            return;
        };
        match message {
            L::LspMessage::Notification { method, params } => {
                if method == "textDocument/publishDiagnostics"
                    && let Some((uri, items)) = L::parse_diagnostics(&params)
                    && let Some(path) = L::uri_to_path(&uri)
                    && self.docs.contains_key(&path)
                {
                    self.diagnostics.insert(path, items);
                }
            }
            L::LspMessage::Response { id, result, error } => {
                self.on_response(key, id, result, error);
            }
            // Answer server-to-client requests with null (progress and
            // registration prompts), like the probe verified: unanswered
            // requests can stall analysis.
            L::LspMessage::Request { id, .. } => {
                let reply = match &id {
                    L::RequestId::Number(number) => {
                        serde_json::json!({"jsonrpc":"2.0","id":number,"result":null})
                    }
                    L::RequestId::Text(text) => {
                        serde_json::json!({"jsonrpc":"2.0","id":text,"result":null})
                    }
                };
                if let Some(server) = self.servers.get(key) {
                    let _ = server.inbox.send(L::frame(reply.to_string().as_bytes()));
                }
            }
        }
    }

    pub(super) fn on_response(
        &mut self,
        key: &ServerKey,
        id: L::RequestId,
        result: Option<serde_json::Value>,
        error: Option<L::ResponseError>,
    ) {
        let L::RequestId::Number(number) = id else {
            return;
        };
        // Init handshake first: only the starting server owns init ids.
        if self
            .servers
            .get(key)
            .is_some_and(|server| server.init_id == Some(number))
        {
            // A failed `initialize` (for example TypeScript's server with
            // no usable `typescript` package) must not count as ready.
            if let Some(error) = error {
                self.shutdown_server(key);
                self.note_failure(key.language);
                self.unavailable.insert(
                    key.language,
                    format!("Language server failed to start: {}", error.message),
                );
                return;
            }
            let Some(server) = self.servers.get_mut(key) else {
                return;
            };
            server.init_id = None;
            server.ready = true;
            let _ = server.inbox.send(L::initialized());
            let inbox = server.inbox.clone();
            // Buffers submitted while starting open now, in order behind
            // `initialized`: a single pre-ready submit is enough, and no
            // further frame is needed to get its `didOpen` out.
            for doc in self.docs.values_mut().filter(|doc| doc.server == *key) {
                if let Some(text) = doc.pending_text.take() {
                    doc.version = doc.version.saturating_add(1);
                    let _ = inbox.send(L::did_open(&doc.uri, doc.language_id, doc.version, &text));
                    doc.opened = true;
                } else {
                    doc.opened = false;
                }
            }
            self.failures.remove(key.language);
            return;
        }
        let pending = self
            .servers
            .get_mut(key)
            .and_then(|server| server.pending.remove(&number));
        match pending {
            Some(Pending::Hover { path, cursor }) => {
                let result = result.unwrap_or(serde_json::Value::Null);
                if let Some(hover) = L::parse_hover(&result) {
                    self.hover = Some(HoverPopup {
                        path,
                        cursor_line: cursor.0,
                        cursor_col: cursor.1,
                        text: hover.text,
                        markdown: hover.markdown,
                        open: true,
                        status: false,
                    });
                }
                // A null answer clears a status note (the request it
                // hung off is done); real answers are never cleared.
                else if self
                    .hover
                    .as_ref()
                    .is_some_and(|popup| popup.status && popup.path == path)
                {
                    self.hover = None;
                }
            }
            Some(Pending::Symbols { path, revision }) => {
                let result = result.unwrap_or(serde_json::Value::Null);
                let parsed = L::parse_document_symbols(&result);
                if parsed.is_empty() {
                    self.symbols_retry.insert(path.clone(), Instant::now());
                } else {
                    self.symbols_retry.remove(&path);
                }
                // Re-answers after unrelated edits are common; only a
                // changed outline invalidates the palette's cached results.
                let changed = self
                    .symbols
                    .get(&path)
                    .is_none_or(|old| old.as_ref() != parsed.as_slice());
                if changed {
                    self.symbols.insert(path.clone(), parsed.into());
                    self.search_revision = self.search_revision.wrapping_add(1);
                }
                self.symbols_rev.insert(path, revision);
            }
            Some(Pending::InlayHints { path, revision }) => {
                // A failed answer ("content modified" while indexing) keeps
                // the previous hints; it and an empty answer are asked
                // again after a pause.
                let hints = error
                    .is_none()
                    .then(|| L::parse_inlay_hints(&result.unwrap_or(serde_json::Value::Null)));
                if hints.as_ref().is_none_or(Vec::is_empty) {
                    self.inlay_retry.insert(path.clone(), Instant::now());
                } else {
                    self.inlay_retry.remove(&path);
                }
                if let Some(hints) = hints {
                    self.inlay.insert(path, (revision, hints.into()));
                }
            }
            Some(Pending::WorkspaceSymbols { generation }) => {
                if generation == self.workspace_symbol_generation {
                    let hits =
                        L::parse_workspace_symbols(&result.unwrap_or(serde_json::Value::Null));
                    self.workspace_symbols
                        .insert(key.clone(), hits.into_iter().take(200).collect());
                    self.search_revision = self.search_revision.wrapping_add(1);
                }
            }
            Some(Pending::Definition { path }) => {
                let result = result.unwrap_or(serde_json::Value::Null);
                let targets = L::parse_definition(&result);
                if !targets.is_empty() {
                    self.goto = Some(GotoJump {
                        from: path,
                        targets,
                    });
                }
            }
            // The editor matches answers to its popup by generation.
            Some(Pending::Completion { path, generation }) => {
                let result = result.unwrap_or(serde_json::Value::Null);
                self.completion = Some(CompletionResults {
                    path,
                    generation,
                    items: L::parse_completion(&result),
                });
            }
            Some(Pending::Format { path, revision }) => {
                let result = result.unwrap_or(serde_json::Value::Null);
                self.edit_answers.format = Some((path, revision, L::parse_text_edits(&result)));
            }
            Some(Pending::CodeActions { path, generation }) => {
                let result = result.unwrap_or(serde_json::Value::Null);
                self.edit_answers.actions =
                    Some((path, generation, L::parse_code_actions(&result)));
            }
            Some(Pending::Resolve { path }) => {
                if let Some(action) = result {
                    self.edit_answers.resolved = Some((path, action));
                }
            }
            Some(Pending::Signature { path, generation }) => {
                let result = result.unwrap_or(serde_json::Value::Null);
                self.signature = Some((path, generation, L::parse_signature_help(&result)));
            }
            Some(Pending::References { path }) => {
                let result = result.unwrap_or(serde_json::Value::Null);
                self.references = Some(ReferenceResults {
                    from: path,
                    targets: L::parse_references(&result),
                });
            }
            Some(Pending::Rename {
                path,
                new_name,
                revision,
            }) => {
                let stale = self
                    .docs
                    .get(&path)
                    .is_none_or(|doc| doc.observed != revision);
                let edits = if stale {
                    Err("The file changed while the rename was running. Try again.".into())
                } else if let Some(error) = error {
                    Err(error.message)
                } else {
                    L::parse_rename(&result.unwrap_or(serde_json::Value::Null))
                };
                self.rename = Some(RenameResults {
                    from: path,
                    new_name,
                    edits,
                });
            }
            None => {}
        }
    }
}
