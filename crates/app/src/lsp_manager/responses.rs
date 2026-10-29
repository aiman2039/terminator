use super::*;

pub(super) struct OnResponseInput<'a> {
    pub(super) key: &'a ServerKey,
    pub(super) id: L::RequestId,
    pub(super) result: Option<serde_json::Value>,
    pub(super) error: Option<L::ResponseError>,
}

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
                self.on_response(OnResponseInput {
                    key,
                    id,
                    result,
                    error,
                });
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

    pub(super) fn on_response(&mut self, input: OnResponseInput<'_>) {
        let OnResponseInput {
            key,
            id,
            result,
            error,
        } = input;
        let L::RequestId::Number(number) = id else {
            return;
        };
        // Init handshake first: only the starting server owns init ids.
        if self
            .servers
            .get(key)
            .is_some_and(|server| server.init_id == Some(number))
        {
            self.on_initialize(key, result, error);
            return;
        }
        let pending = self
            .servers
            .get_mut(key)
            .and_then(|server| server.pending.remove(&number));
        let Some(pending) = pending else {
            return;
        };
        self.on_answer(key, pending, Answer { result, error });
    }

    fn on_initialize(
        &mut self,
        key: &ServerKey,
        result: Option<serde_json::Value>,
        error: Option<L::ResponseError>,
    ) {
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
        server.capabilities = result
            .as_ref()
            .and_then(|result| result.get("capabilities"))
            .cloned()
            .unwrap_or_default();
        let _ = server.inbox.send(L::initialized());
        let inbox = server.inbox.clone();
        // Buffers submitted while starting open now, in order behind
        // `initialized`: a single pre-ready submit is enough, and no
        // further frame is needed to get its `didOpen` out.
        for doc in self.docs.values_mut().filter(|doc| doc.server == *key) {
            if let Some(text) = doc.pending_text.take() {
                doc.version = doc.version.saturating_add(1);
                let _ = inbox.send(L::did_open(L::DidOpenInput {
                    uri: &doc.uri,
                    language_id: doc.language_id,
                    version: doc.version,
                    text: &text,
                }));
                doc.opened = true;
            } else {
                doc.opened = false;
            }
        }
        self.failures.remove(key.language);
    }

    fn on_answer(&mut self, key: &ServerKey, pending: Pending, answer: Answer) {
        match pending {
            Pending::Hover { path, cursor } => self.on_hover(path, cursor, answer.value()),
            Pending::Symbols { path, revision } => self.on_symbols(path, revision, answer.value()),
            Pending::InlayHints { path, revision } => self.on_inlay_hints(path, revision, answer),
            Pending::WorkspaceSymbols { generation } => {
                self.on_workspace_symbols(key, generation, answer.value());
            }
            Pending::Definition { path } => self.on_definition(path, answer.value()),
            // The editor matches answers to its popup by generation.
            Pending::Completion { path, generation } => {
                self.completion = Some(CompletionResults {
                    path,
                    generation,
                    items: L::parse_completion(&answer.value()),
                });
            }
            Pending::Format { path, revision } => {
                self.edit_answers.format =
                    Some((path, revision, L::parse_text_edits(&answer.value())));
            }
            Pending::OnTypeFormat { path, revision } => {
                self.edit_answers.on_type =
                    Some((path, revision, L::parse_text_edits(&answer.value())));
            }
            Pending::DocumentHighlight { path, revision } => {
                self.on_document_highlight(path, revision, &answer.value());
            }
            Pending::CodeActions { path, generation } => {
                self.edit_answers.actions =
                    Some((path, generation, L::parse_code_actions(&answer.value())));
            }
            Pending::Resolve { path } => {
                if let Some(action) = answer.result {
                    self.edit_answers.resolved = Some((path, action));
                }
            }
            Pending::Signature { path, generation } => {
                self.signature = Some((path, generation, L::parse_signature_help(&answer.value())));
            }
            Pending::Hierarchy { generation, node } => {
                self.on_hierarchy(generation, node, answer.value());
            }
            Pending::References { path } => {
                self.references = Some(ReferenceResults {
                    from: path,
                    targets: L::parse_references(&answer.value()),
                });
            }
            Pending::Rename {
                path,
                new_name,
                revision,
            } => self.on_rename(RenameAnswer {
                path,
                new_name,
                revision,
                answer,
            }),
        }
    }
}
