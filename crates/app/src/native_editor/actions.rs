//! Reformat (Cmd/Ctrl+Alt+L) and code actions (Alt+Enter) through the
//! language server. Edits apply to open buffers as one undo step each;
//! actions that arrive without an edit are resolved on pick. The popup
//! lives in `action_popup`.

pub(crate) use super::action_popup::{ActionPick, ActionsPopup, BuiltinAction, actions_ui};
use super::*;

/// Apply LSP text edits (UTF-16 ranges) to a buffer, last first so earlier
/// ranges stay valid, as one undo step. Edits at the same position apply
/// in reverse array order, so their texts land in array order (LSP).
/// Returns whether anything changed.
pub(crate) fn apply_text_edits(buffer: &mut Doc, edits: &[native_lsp::RenameTextEdit]) -> bool {
    let mut ranges: Vec<_> = edit_ranges(buffer, edits).into_iter().enumerate().collect();
    ranges.sort_by(|(i, a), (j, b)| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(j.cmp(i)));
    if ranges.is_empty() {
        return false;
    }
    buffer.begin_undo_group();
    for (_, (from, to, text)) in ranges {
        buffer.delete(from..to);
        buffer.insert(from, text);
    }
    buffer.end_undo_group();
    true
}

/// Apply edits (on-type formatting) and keep the caret on the same text:
/// edits that end before it shift it by their length change.
pub(crate) fn apply_edits_keep_caret(
    buffer: &mut Doc,
    engine: &mut VimEngine,
    edits: &[native_lsp::RenameTextEdit],
) {
    let caret = engine.cursor();
    let at = buffer.char_at_line_col(caret.line, caret.col);
    let moved = edit_ranges(buffer, edits)
        .into_iter()
        .filter(|(_, to, _)| *to <= at)
        .fold(at, |caret, (from, to, text)| {
            caret
                .saturating_sub(to.saturating_sub(from))
                .saturating_add(text.chars().count())
        });
    if apply_text_edits(buffer, edits) {
        let (line, col) = buffer.line_col_at(moved.min(buffer.len_chars()));
        let _ = engine.place_cursor(buffer, line, col);
    }
}

/// Edits as ordered buffer char ranges `(from, to, text)`.
fn edit_ranges<'a>(
    buffer: &Doc,
    edits: &'a [native_lsp::RenameTextEdit],
) -> Vec<(usize, usize, &'a str)> {
    let char_at = |buffer: &Doc, position: native_lsp::Position| {
        let line = usize::try_from(position.line).unwrap_or(usize::MAX);
        if line >= buffer.line_count() {
            return buffer.len_chars();
        }
        let text = buffer.line_text(line);
        buffer.char_at_line_col(
            line,
            native_lsp::utf16_to_char_col(&text, position.character),
        )
    };
    edits
        .iter()
        .map(|edit| {
            let (a, b) = (
                char_at(buffer, edit.range.start),
                char_at(buffer, edit.range.end),
            );
            (a.min(b), a.max(b), edit.new_text.as_str())
        })
        .collect()
}

impl App {
    /// Apply a workspace edit's text edits to the files open in native
    /// editors; edits to files that are not open are reported, not lost
    /// silently.
    pub(crate) fn apply_workspace_edit(&mut self, files: Vec<native_lsp::RenameFileEdit>) {
        if let Some(stale) = files.iter().find(|file| self.edit_is_stale(file)) {
            self.error = Some(format!(
                "{} changed since the action was computed. Try again.",
                native_lsp::uri_to_path(&stale.uri)
                    .map_or_else(|| stale.uri.clone(), |path| path.display().to_string())
            ));
            return;
        }
        let mut closed = Vec::new();
        for file in files {
            let open = native_lsp::uri_to_path(&file.uri)
                .and_then(|path| self.native_docs.get_mut(&path))
                .and_then(|doc| doc.doc.as_mut());
            match open {
                Some(buffer) => {
                    apply_text_edits(buffer, &file.edits);
                }
                None => closed.push(file),
            }
        }
        if closed.is_empty() {
            return;
        }
        // Files that are not open are edited on disk, with the same
        // checks and rollback as a rename (inside the project only).
        let Some(root) = self.selected_project().map(|project| project.path.clone()) else {
            self.error = Some("The action edits files outside an open project".into());
            return;
        };
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let service = self.services.clone();
        let submitted = self.services.handle().submit(
            OperationContext::new(
                "code-action-apply",
                root.display().to_string(),
                Policy::OrderedMutation,
            ),
            CancellationToken::new(),
            async move {
                let token = CancellationToken::new();
                let result = service
                    .fs()
                    .run(&token, move || {
                        let prepared = super::rename::prepare_rename(&root, closed)?;
                        super::rename::apply_rename(prepared)
                    })
                    .await;
                let _ = tx.send(result.map_err(|error| format!("{error:#}")));
                Ok(Vec::new())
            },
        );
        match submitted {
            Ok(_) => self.action_apply = Some(rx),
            Err(error) => self.error = Some(format!("Could not apply the action: {error}")),
        }
    }

    /// A file edit whose server version no longer matches the open
    /// buffer's synced version.
    fn edit_is_stale(&self, file: &native_lsp::RenameFileEdit) -> bool {
        let Some(version) = file.version else {
            return false;
        };
        native_lsp::uri_to_path(&file.uri)
            .filter(|path| self.native_docs.contains_key(path))
            .is_some_and(|path| self.lsp.document_version(&path) != Some(version))
    }

    /// The open buffer at `path` is still at `revision`.
    fn buffer_at_revision(&self, path: &Path, revision: u64) -> bool {
        self.native_docs
            .get(path)
            .and_then(NativeDoc::buffer)
            .is_some_and(|buffer| buffer.revision() == revision)
    }

    /// Apply a picked action: its edit now, or ask the server to resolve it.
    /// `revision` is the buffer revision the actions were listed for.
    pub(crate) fn run_code_action(
        &mut self,
        path: &Path,
        item: &native_lsp::CodeActionItem,
        revision: u64,
    ) {
        if !self.buffer_at_revision(path, revision) {
            self.info = Some("The file changed since the actions were listed; try again".into());
            return;
        }
        match native_lsp::code_action_edits(&item.raw) {
            Ok(files) => self.apply_workspace_edit(files),
            // "Add to dictionary", "Ignore": the server updates its state
            // and republishes the underlines.
            Err(_) if item.spell && native_lsp::action_command(&item.raw).is_some() => {
                if !self.lsp.execute_action_command(path, item) {
                    self.error = Some("The spell checker is not ready".into());
                }
            }
            Err(_) if item.raw.get("data").is_some() => {
                self.resolve_attempts = 0;
                self.resolve_revision = revision;
                if !self.lsp.request_resolve(path, &item.raw) {
                    self.error = Some("The language server is not ready".into());
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Formatting, action lists, and resolved actions that arrived.
    pub(crate) fn drain_edit_answers(&mut self) {
        if let Some(rx) = self.action_apply.as_mut()
            && let Ok(result) = rx.try_recv()
        {
            self.action_apply = None;
            match result {
                Ok(paths) => {
                    self.info = Some(format!(
                        "The action also updated {} file{} on disk",
                        paths.len(),
                        if paths.len() == 1 { "" } else { "s" }
                    ));
                }
                Err(error) => self.error = Some(format!("Code action not applied: {error}")),
            }
        }
        if let Some((path, revision, edits)) = self.lsp.edit_answers.format.take()
            && let Some(buffer) = self
                .native_docs
                .get_mut(&path)
                .and_then(|doc| doc.doc.as_mut())
        {
            if buffer.revision() == revision {
                apply_text_edits(buffer, &edits);
            } else {
                self.info = Some("The file changed while formatting; format again".into());
            }
        }
        // On-type edits for older text are dropped: typing moved on.
        if let Some((path, revision, edits)) = self.lsp.edit_answers.on_type.take()
            && let Some(doc) = self.native_docs.get_mut(&path)
            && let Some(buffer) = doc.doc.as_mut()
            && buffer.revision() == revision
        {
            apply_edits_keep_caret(buffer, &mut doc.engine, &edits);
        }
        if let Some((path, generation, items)) = self.lsp.edit_answers.actions.take()
            && let Some(popup) = self.code_actions.as_mut()
            && popup.path == path
            && popup.generation == Some(generation)
        {
            if items.is_empty() && popup.builtins.is_empty() {
                self.code_actions = None;
                self.info = Some("No code actions here".into());
            } else {
                popup.items = items;
                popup.generation = None;
            }
        }
        if let Some((path, action)) = self.lsp.edit_answers.resolved.take() {
            if !self.buffer_at_revision(&path, self.resolve_revision) {
                self.info = Some("The file changed before the action resolved; try again".into());
                return;
            }
            match native_lsp::code_action_edits(&action) {
                Ok(files) => self.apply_workspace_edit(files),
                // Servers answer an edit-less action while their analysis
                // catches up; ask again a few times before giving up.
                Err(_) if self.resolve_attempts < 3 && action.get("data").is_some() => {
                    self.resolve_attempts = self.resolve_attempts.saturating_add(1);
                    self.lsp.request_resolve(&path, &action);
                }
                Err(error) => self.error = Some(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EditInput<'a> {
        line: u32,
        from: u32,
        to: u32,
        text: &'a str,
    }

    fn edit(input: EditInput<'_>) -> native_lsp::RenameTextEdit {
        let EditInput {
            line,
            from,
            to,
            text,
        } = input;
        native_lsp::RenameTextEdit {
            range: native_lsp::Range {
                start: native_lsp::Position {
                    line,
                    character: from,
                },
                end: native_lsp::Position {
                    line,
                    character: to,
                },
            },
            new_text: text.into(),
        }
    }

    #[test]
    fn text_edits_apply_back_to_front_as_one_undo_step() {
        let mut buffer = Doc::new("fn a(){\nx\n}\n");
        let mut engine = VimEngine::new();
        assert!(apply_text_edits(
            &mut buffer,
            &[
                edit(EditInput {
                    line: 0,
                    from: 6,
                    to: 6,
                    text: " "
                }),
                edit(EditInput {
                    line: 1,
                    from: 0,
                    to: 0,
                    text: "    "
                })
            ],
        ));
        assert_eq!(buffer.text(), "fn a() {\n    x\n}\n");
        engine.undo(&mut buffer);
        assert_eq!(buffer.text(), "fn a(){\nx\n}\n");
        assert!(!apply_text_edits(&mut buffer, &[]));
    }

    #[test]
    fn on_type_edits_keep_the_caret_on_the_typed_text() {
        let mut buffer = Doc::new("        }\n");
        let mut engine = VimEngine::new();
        let _ = engine.place_cursor(&mut buffer, 0, 8);
        apply_edits_keep_caret(
            &mut buffer,
            &mut engine,
            &[edit(EditInput {
                line: 0,
                from: 0,
                to: 8,
                text: "    ",
            })],
        );
        assert_eq!(buffer.text(), "    }\n");
        assert_eq!(engine.cursor().col, 4);
    }

    #[test]
    fn inserts_at_one_position_keep_their_array_order() {
        let mut buffer = Doc::new("x");
        let at = |text| {
            edit(EditInput {
                line: 0,
                from: 0,
                to: 0,
                text,
            })
        };
        assert!(apply_text_edits(&mut buffer, &[at("A"), at("B")]));
        assert_eq!(buffer.text(), "ABx");
    }

    #[test]
    fn a_code_action_picked_after_an_edit_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let path = dir.path().join("a.rs");
        let mut native = NativeDoc::new(path.clone(), false);
        native.doc = Some(Doc::new("let a = 1;\n"));
        let listed = native.doc.as_ref().map_or(0, Doc::revision);
        native.doc.as_mut().unwrap().insert(0, "// typed\n");
        app.native_docs.insert(path.clone(), native);
        let uri = native_lsp::file_uri(&path).unwrap().to_string();
        let item = native_lsp::CodeActionItem {
            title: "Rename".into(),
            kind: None,
            preferred: false,
            spell: false,
            raw: serde_json::json!({"edit": {"changes": {uri: [{
                "range": {"start": {"line": 0, "character": 4},
                          "end": {"line": 0, "character": 5}},
                "newText": "b"
            }]}}}),
        };
        app.run_code_action(&path, &item, listed);
        let text = app.native_docs[&path].buffer().unwrap().text();
        assert_eq!(text, "// typed\nlet a = 1;\n");
        // At the listed revision the same action applies.
        let current = app.native_docs[&path].buffer().unwrap().revision();
        app.run_code_action(&path, &item, current);
        let text = app.native_docs[&path].buffer().unwrap().text();
        assert_eq!(text, "// tbped\nlet a = 1;\n");
    }
}
