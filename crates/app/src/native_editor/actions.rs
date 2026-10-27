//! Reformat (Cmd/Ctrl+Alt+L) and code actions (Alt+Enter) through the
//! language server. Edits apply to open buffers as one undo step each;
//! actions that arrive without an edit are resolved on pick.

use super::*;

/// The Alt+Enter popup.
pub(crate) struct ActionsPopup {
    pub path: PathBuf,
    /// Generation of the in-flight request (`None` once answered).
    pub generation: Option<u64>,
    pub items: Vec<native_lsp::CodeActionItem>,
    pub selected: usize,
    pub anchor: Option<egui::Rect>,
}

/// Apply LSP text edits (UTF-16 ranges) to a buffer, last first so earlier
/// ranges stay valid, as one undo step. Returns whether anything changed.
pub(crate) fn apply_text_edits(buffer: &mut Doc, edits: &[native_lsp::RenameTextEdit]) -> bool {
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
    let mut ranges: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|edit| {
            (
                char_at(buffer, edit.range.start),
                char_at(buffer, edit.range.end),
                edit.new_text.as_str(),
            )
        })
        .collect();
    ranges.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    if ranges.is_empty() {
        return false;
    }
    buffer.begin_undo_group();
    for (from, to, text) in ranges {
        let (from, to) = (from.min(to), from.max(to));
        buffer.delete(from..to);
        buffer.insert(from, text);
    }
    buffer.end_undo_group();
    true
}

/// What the popup asked for this frame.
pub(crate) enum ActionPick {
    None,
    Close,
    Pick(usize),
}

/// Keys and clicks for the actions popup, painted at the caret.
pub(crate) fn actions_ui(ui: &egui::Ui, popup: &mut ActionsPopup) -> ActionPick {
    if popup.items.is_empty() {
        return ActionPick::None;
    }
    popup.selected = popup.selected.min(popup.items.len().saturating_sub(1));
    let mut pick = ActionPick::None;
    ui.input_mut(|input| {
        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
            popup.selected = popup
                .selected
                .saturating_add(1)
                .checked_rem(popup.items.len())
                .unwrap_or(0);
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
            popup.selected = popup
                .selected
                .checked_sub(1)
                .unwrap_or(popup.items.len().saturating_sub(1));
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
            pick = ActionPick::Pick(popup.selected);
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            pick = ActionPick::Close;
        }
    });
    let Some(anchor) = popup.anchor else {
        return pick;
    };
    egui::Area::new(egui::Id::unique(("native-code-actions", &popup.path)))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor.left_bottom())
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(480.0);
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for (index, item) in popup.items.iter().enumerate() {
                            let mut text = egui::RichText::new(&item.title);
                            if item.preferred {
                                text = text.strong();
                            }
                            if ui.selectable_label(index == popup.selected, text).clicked() {
                                pick = ActionPick::Pick(index);
                            }
                        }
                    });
            });
        });
    pick
}

impl App {
    /// Apply a workspace edit's text edits to the files open in native
    /// editors; edits to files that are not open are reported, not lost
    /// silently.
    pub(crate) fn apply_workspace_edit(&mut self, files: Vec<native_lsp::RenameFileEdit>) {
        let mut skipped = 0usize;
        for file in files {
            let Some(path) = native_lsp::uri_to_path(&file.uri) else {
                skipped = skipped.saturating_add(1);
                continue;
            };
            match self
                .native_docs
                .get_mut(&path)
                .and_then(|doc| doc.doc.as_mut())
            {
                Some(buffer) => {
                    apply_text_edits(buffer, &file.edits);
                }
                None => skipped = skipped.saturating_add(1),
            }
        }
        if skipped > 0 {
            self.error = Some(format!(
                "The action also edits {skipped} file{} that {} not open; open {} and retry.",
                if skipped == 1 { "" } else { "s" },
                if skipped == 1 { "is" } else { "are" },
                if skipped == 1 { "it" } else { "them" },
            ));
        }
    }

    /// Apply a picked action: its edit now, or ask the server to resolve it.
    pub(crate) fn run_code_action(&mut self, path: &Path, item: &native_lsp::CodeActionItem) {
        match native_lsp::code_action_edits(&item.raw) {
            Ok(files) => self.apply_workspace_edit(files),
            Err(_) if item.raw.get("data").is_some() => {
                self.resolve_attempts = 0;
                if !self.lsp.request_resolve(path, &item.raw) {
                    self.error = Some("The language server is not ready".into());
                }
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Formatting, action lists, and resolved actions that arrived.
    pub(crate) fn drain_edit_answers(&mut self) {
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
        if let Some((path, generation, items)) = self.lsp.edit_answers.actions.take()
            && let Some(popup) = self.code_actions.as_mut()
            && popup.path == path
            && popup.generation == Some(generation)
        {
            if items.is_empty() {
                self.code_actions = None;
                self.info = Some("No code actions here".into());
            } else {
                popup.items = items;
                popup.generation = None;
            }
        }
        if let Some((path, action)) = self.lsp.edit_answers.resolved.take() {
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

    fn edit(line: u32, from: u32, to: u32, text: &str) -> native_lsp::RenameTextEdit {
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
            &[edit(0, 6, 6, " "), edit(1, 0, 0, "    ")],
        ));
        assert_eq!(buffer.text(), "fn a() {\n    x\n}\n");
        engine.undo(&mut buffer);
        assert_eq!(buffer.text(), "fn a(){\nx\n}\n");
        assert!(!apply_text_edits(&mut buffer, &[]));
    }
}
