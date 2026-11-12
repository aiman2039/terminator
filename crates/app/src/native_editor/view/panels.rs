//! Panels and popups a native view draws under its source view.
use super::*;

impl ViewPass<'_> {
    pub(super) fn actions_popup(&mut self, ui: &egui::Ui) {
        let caret_rect = self.requests.caret_rect;
        let Some(popup) = self.popups.actions.as_mut() else {
            return;
        };
        if caret_rect.is_some() {
            popup.anchor = caret_rect;
        }
        match actions::actions_ui(ui, popup) {
            actions::ActionPick::Pick(index) => {
                self.requests.picked_action = popup
                    .items
                    .get(index)
                    .cloned()
                    .map(|item| (item, popup.revision));
                self.popups.actions = None;
            }
            actions::ActionPick::Builtin(builtin) => {
                self.popups.actions = None;
                self.run_builtin_action(builtin);
            }
            actions::ActionPick::Close => self.popups.actions = None,
            actions::ActionPick::None => {}
        }
    }

    fn run_builtin_action(&mut self, builtin: actions::BuiltinAction) {
        use actions::BuiltinAction;
        match builtin {
            BuiltinAction::GotoImplementation => {
                self.requests.locate_at =
                    Some((self.caret_at(), native_lsp::Locate::Implementation));
            }
            BuiltinAction::FindUsages => self.requests.references_at = Some(self.caret_at()),
            BuiltinAction::Rename => self.requests.rename_at = Some(self.caret_at()),
            BuiltinAction::AddToAgentContext => {
                let caret = self.doc.engine.cursor();
                let (from, to) = self.doc.engine.selection().unwrap_or((caret, caret));
                let (first, last) = (from.line.min(to.line), from.line.max(to.line));
                self.requests.agent_context =
                    Some((first.saturating_add(1), last.saturating_add(1)));
            }
        }
    }

    pub(super) fn gutter_popup(&mut self, ui: &egui::Ui, gutter_hunks: &[gutter::GutterHunk]) {
        let index = self.popups.gutter.as_ref().map(|popup| popup.index);
        let action = gutter::popup_ui(gutter::PopupUiInput {
            ui,
            popup: &mut self.popups.gutter,
            hunks: gutter_hunks,
            deleted: self.frame.change_colors[2],
            can_stage: !self.doc.dirty(),
            commit_lock: self.frame.commit_lock,
        });
        let Some(hunk) = index.and_then(|index| gutter_hunks.get(index)) else {
            return;
        };
        match action {
            gutter::PopupAction::Rollback => {
                if let Some(live) = self.doc.doc.as_mut() {
                    gutter::rollback(live, &mut self.doc.engine, hunk);
                }
            }
            gutter::PopupAction::ShowDiff => self.requests.gutter_diff = true,
            gutter::PopupAction::Stage => {
                self.requests.gutter_stage =
                    self.doc.doc.as_ref().map(|live| (hunk.mark, live.text()));
            }
            gutter::PopupAction::None => {}
        }
    }

    pub(super) fn signature_popup(&mut self, ui: &mut egui::Ui, revision_before: u64) {
        let Some(live) = self.doc.doc.as_ref() else {
            return;
        };
        self.requests.signature_request =
            complete::signature_after_frame(complete::SignatureAfterFrameInput {
                popup: &mut self.popups.signature,
                buffer: live,
                engine: &self.doc.engine,
                revision_before,
                server: native_lsp::server_language(self.frame.language).is_some(),
            });
        // Last frame's completion list above the caret takes that room.
        let list_above = self
            .popups
            .completion
            .as_ref()
            .and_then(|list| list.rect.zip(list.anchor))
            .is_some_and(|(list, caret)| list.bottom() <= caret.top() + 0.5);
        if let Some(shown) = self.popups.signature.as_mut() {
            if self.requests.caret_rect.is_some() {
                shown.anchor = self.requests.caret_rect;
            }
            complete::signature_ui(ui, shown, list_above);
        }
    }

    pub(super) fn completion_after_frame(&mut self, ui: &mut egui::Ui, revision_before: u64) {
        let Some(live) = self.doc.doc.as_mut() else {
            return;
        };
        let after = complete::after_frame(complete::AfterFrameInput {
            ui,
            popup: &mut self.popups.completion,
            buffer: live,
            engine: &mut self.doc.engine,
            revision_before,
            manual: self.requests.complete_now,
            caret_rect: self.requests.caret_rect,
            server: native_lsp::server_language(self.frame.language).is_some(),
            triggers: &self.frame.completion_triggers,
        });
        self.requests.completion_ask = after.ask;
        self.requests.accepted_completion = after.accepted;
    }
}
