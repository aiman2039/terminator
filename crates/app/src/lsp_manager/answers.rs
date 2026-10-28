use super::*;

/// A response's payload: `result` on success, `error` on failure.
pub(super) struct Answer {
    pub(super) result: Option<serde_json::Value>,
    pub(super) error: Option<L::ResponseError>,
}

impl Answer {
    /// The result, with a failed or empty answer read as `null`.
    pub(super) fn value(self) -> serde_json::Value {
        self.result.unwrap_or(serde_json::Value::Null)
    }
}

pub(super) struct RenameAnswer {
    pub(super) path: PathBuf,
    pub(super) new_name: String,
    pub(super) revision: u64,
    pub(super) answer: Answer,
}

impl LspManager {
    pub(super) fn on_hover(
        &mut self,
        path: PathBuf,
        cursor: (usize, usize),
        result: serde_json::Value,
    ) {
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

    pub(super) fn on_symbols(&mut self, path: PathBuf, revision: u64, result: serde_json::Value) {
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

    pub(super) fn on_inlay_hints(&mut self, path: PathBuf, revision: u64, answer: Answer) {
        // A failed answer ("content modified" while indexing) keeps
        // the previous hints; it and an empty answer are asked
        // again after a pause.
        let Answer { result, error } = answer;
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

    pub(super) fn on_workspace_symbols(
        &mut self,
        key: &ServerKey,
        generation: u64,
        result: serde_json::Value,
    ) {
        if generation != self.workspace_symbol_generation {
            return;
        }
        let hits = L::parse_workspace_symbols(&result);
        self.workspace_symbols
            .insert(key.clone(), hits.into_iter().take(200).collect());
        self.search_revision = self.search_revision.wrapping_add(1);
    }

    pub(super) fn on_definition(&mut self, path: PathBuf, result: serde_json::Value) {
        let targets = L::parse_definition(&result);
        if !targets.is_empty() {
            self.goto = Some(GotoJump {
                from: path,
                targets,
            });
        }
    }

    pub(super) fn on_hierarchy(
        &mut self,
        generation: u64,
        node: Option<usize>,
        result: serde_json::Value,
    ) {
        // A failed answer reads as "nothing here" (empty node).
        let items = L::parse_hierarchy_items(&result);
        self.hierarchy_answers.push(match node {
            None => HierarchyAnswer::Roots { generation, items },
            Some(node) => HierarchyAnswer::Children {
                generation,
                node,
                items,
            },
        });
    }

    pub(super) fn on_rename(&mut self, input: RenameAnswer) {
        let RenameAnswer {
            path,
            new_name,
            revision,
            answer,
        } = input;
        let stale = self
            .docs
            .get(&path)
            .is_none_or(|doc| doc.observed != revision);
        let edits = if stale {
            Err("The file changed while the rename was running. Try again.".into())
        } else if let Some(error) = answer.error {
            Err(error.message)
        } else {
            L::parse_rename(&answer.result.unwrap_or(serde_json::Value::Null))
        };
        self.rename = Some(RenameResults {
            from: path,
            new_name,
            edits,
        });
    }
}
