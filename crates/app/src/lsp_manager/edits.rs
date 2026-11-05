use super::*;

impl LspManager {
    /// Ask for completions at `position`; `trigger` says why. Returns the
    /// request's generation (a later request supersedes it), or `None`
    /// when no ready server tracks the file.
    pub fn request_completion(
        &mut self,
        path: &Path,
        position: L::Position,
        trigger: L::CompletionTrigger,
    ) -> Option<u64> {
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
        let _ = server.inbox.send(L::completion(
            &L::RequestId::Number(number),
            &uri,
            position,
            trigger,
        ));
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
    pub(super) fn send_for(
        &mut self,
        path: &Path,
        build: impl FnOnce(&L::RequestId, &url::Url) -> Vec<u8>,
        pending: Pending,
    ) -> bool {
        self.send_in((Slot::Main, path), build, pending)
    }

    /// [`send_for`](Self::send_for) on one slot's server.
    pub(super) fn send_in(
        &mut self,
        (slot, path): (Slot, &Path),
        build: impl FnOnce(&L::RequestId, &url::Url) -> Vec<u8>,
        pending: Pending,
    ) -> bool {
        let Some(doc) = self.docs_of(slot).get(path) else {
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

    /// Indentation format requests for `path` send (the buffer's own,
    /// from editor settings or detected from its text).
    pub fn set_format_options(&mut self, path: &Path, options: L::FormatOptions) {
        if self.format_options.get(path) != Some(&options) {
            self.format_options.insert(path.to_owned(), options);
        }
    }

    pub(super) fn format_options_for(&self, path: &Path) -> L::FormatOptions {
        self.format_options.get(path).copied().unwrap_or_default()
    }

    /// Format the whole document as of buffer `revision`.
    pub fn request_format(&mut self, path: &Path, revision: u64) -> bool {
        let options = self.format_options_for(path);
        self.send_for(
            path,
            |id, uri| L::formatting(id, uri, options),
            Pending::Format {
                path: path.to_owned(),
                revision,
            },
        )
    }

    /// Code actions for `range` with the diagnostics on it.
    pub fn request_code_actions(&mut self, path: &Path, range: L::Range) -> Option<u64> {
        let touching = |items: Option<&Vec<L::Diagnostic>>| -> Vec<L::Diagnostic> {
            items
                .into_iter()
                .flatten()
                .filter(|item| {
                    item.range.start.line <= range.end.line
                        && range.start.line <= item.range.end.line
                })
                .cloned()
                .collect()
        };
        let diagnostics = touching(self.diagnostics.get(path));
        let spelling = touching(self.spell_diagnostics.get(path));
        self.completion_generation = self.completion_generation.wrapping_add(1);
        let generation = self.completion_generation;
        fn build(
            range: L::Range,
            diagnostics: &[L::Diagnostic],
        ) -> impl FnOnce(&L::RequestId, &url::Url) -> Vec<u8> + '_ {
            move |id, uri| {
                L::code_action(L::CodeActionInput {
                    id,
                    uri,
                    range,
                    diagnostics,
                })
            }
        }
        let pending = |spell| Pending::CodeActions {
            path: path.to_owned(),
            generation,
            spell,
        };
        let main = self.send_for(path, build(range, &diagnostics), pending(false));
        // Ask the spell checker only where it found something: the
        // popup then never waits on it for a clean line.
        let spell = !spelling.is_empty()
            && self.send_in((Slot::Spell, path), build(range, &spelling), pending(true));
        let remaining = usize::from(main).saturating_add(usize::from(spell));
        if remaining == 0 {
            return None;
        }
        self.action_wait = Some(ActionWait {
            path: path.to_owned(),
            generation,
            remaining,
            items: Vec::new(),
        });
        Some(generation)
    }

    /// Collect one server's code actions; publish the merged list once
    /// every asked server answered. Spell fixes lead on a typo.
    pub(super) fn on_code_actions(
        &mut self,
        (path, generation): (PathBuf, u64),
        spell: bool,
        items: Vec<L::CodeActionItem>,
    ) {
        let Some(wait) = self
            .action_wait
            .as_mut()
            .filter(|wait| wait.path == path && wait.generation == generation)
        else {
            return;
        };
        if spell {
            wait.items.splice(0..0, items);
        } else {
            wait.items.extend(items);
        }
        wait.remaining = wait.remaining.saturating_sub(1);
        if wait.remaining == 0
            && let Some(wait) = self.action_wait.take()
        {
            self.edit_answers.actions = Some((wait.path, wait.generation, wait.items));
        }
    }

    /// Run a bare command action (for example "add to dictionary") on
    /// the server that offered it. False when that server is not ready.
    pub fn execute_action_command(&mut self, path: &Path, item: &L::CodeActionItem) -> bool {
        let Some(command) = L::action_command(&item.raw) else {
            return false;
        };
        let slot = if item.spell { Slot::Spell } else { Slot::Main };
        self.send_in(
            (slot, path),
            |id, _| L::execute_command(id, &command),
            Pending::Command,
        )
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
