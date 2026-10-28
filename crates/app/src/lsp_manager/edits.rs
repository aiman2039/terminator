use super::*;

impl LspManager {
    /// Ask for completions at `position`. Returns the request's generation
    /// (a later request supersedes it), or `None` when no ready server
    /// tracks the file.
    pub fn request_completion(&mut self, path: &Path, position: L::Position) -> Option<u64> {
        let doc = self.docs.get(path)?;
        let (key, uri) = (doc.server.clone(), doc.uri.clone());
        let server = self.servers.get_mut(&key)?;
        if !server.ready {
            return None;
        }
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return None;
        };
        self.completion_generation = self.completion_generation.wrapping_add(1);
        let generation = self.completion_generation;
        let _ = server
            .inbox
            .send(L::completion(&L::RequestId::Number(number), &uri, position));
        server.pending.insert(
            number,
            Pending::Completion {
                path: path.to_owned(),
                generation,
            },
        );
        Some(generation)
    }

    /// Ask for signature help at `position`; generations share the
    /// completion counter, so the newest request of either kind wins.
    pub fn request_signature_help(&mut self, path: &Path, position: L::Position) -> Option<u64> {
        let doc = self.docs.get(path)?;
        let (key, uri) = (doc.server.clone(), doc.uri.clone());
        let server = self.servers.get_mut(&key)?;
        if !server.ready {
            return None;
        }
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return None;
        };
        self.completion_generation = self.completion_generation.wrapping_add(1);
        let generation = self.completion_generation;
        let _ = server.inbox.send(L::signature_help(
            &L::RequestId::Number(number),
            &uri,
            position,
        ));
        server.pending.insert(
            number,
            Pending::Signature {
                path: path.to_owned(),
                generation,
            },
        );
        Some(generation)
    }

    /// Send one request for `path` built from its URI, recording what the
    /// answer is for. False when no ready server tracks the file.
    fn send_for(
        &mut self,
        path: &Path,
        build: impl FnOnce(&L::RequestId, &url::Url) -> Vec<u8>,
        pending: Pending,
    ) -> bool {
        let Some(doc) = self.docs.get(path) else {
            return false;
        };
        let (key, uri) = (doc.server.clone(), doc.uri.clone());
        let Some(server) = self.servers.get_mut(&key) else {
            return false;
        };
        if !server.ready {
            return false;
        }
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return false;
        };
        let _ = server
            .inbox
            .send(build(&L::RequestId::Number(number), &uri));
        server.pending.insert(number, pending);
        true
    }

    /// Format the whole document as of buffer `revision`.
    pub fn request_format(&mut self, path: &Path, revision: u64) -> bool {
        self.send_for(
            path,
            L::formatting,
            Pending::Format {
                path: path.to_owned(),
                revision,
            },
        )
    }

    /// Code actions for `range` with the diagnostics on it.
    pub fn request_code_actions(&mut self, path: &Path, range: L::Range) -> Option<u64> {
        let diagnostics: Vec<L::Diagnostic> = self
            .problems_for(path)
            .into_iter()
            .filter(|item| {
                item.range.start.line <= range.end.line && range.start.line <= item.range.end.line
            })
            .collect();
        self.completion_generation = self.completion_generation.wrapping_add(1);
        let generation = self.completion_generation;
        self.send_for(
            path,
            |id, uri| {
                L::code_action(L::CodeActionInput {
                    id,
                    uri,
                    range,
                    diagnostics: &diagnostics,
                })
            },
            Pending::CodeActions {
                path: path.to_owned(),
                generation,
            },
        )
        .then_some(generation)
    }

    /// Resolve an action that arrived without its edit.
    pub fn request_resolve(&mut self, path: &Path, action: &serde_json::Value) -> bool {
        self.send_for(
            path,
            |id, _| L::code_action_resolve(id, action),
            Pending::Resolve {
                path: path.to_owned(),
            },
        )
    }

    /// Latest signature-help answer, if one arrived.
    pub fn take_signature(&mut self) -> Option<(PathBuf, u64, Option<L::SignatureHelp>)> {
        self.signature.take()
    }

    /// Latest completion answer, if one arrived for the newest request.
    pub fn take_completion(&mut self) -> Option<CompletionResults> {
        self.completion.take()
    }

    pub fn take_references(&mut self) -> Option<ReferenceResults> {
        self.references.take()
    }

    pub fn take_rename(&mut self) -> Option<RenameResults> {
        self.rename.take()
    }
}
