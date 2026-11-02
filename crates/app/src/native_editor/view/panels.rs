//! Panels and popups a native view draws under its source view.
use super::*;

/// The problems panel must not push the source view off screen; the first
/// five stay clickable and the rest are counted.
const LISTED_PROBLEMS: usize = 5;

impl ViewPass<'_> {
    /// Problems panel: click targets under the header. Clicks place the
    /// cursor directly; the buffer is borrowed here.
    pub(super) fn problems_panel(&mut self, ui: &mut egui::Ui) {
        let problems = &self.frame.problems;
        if problems.is_empty() {
            return;
        }
        let summary = Self::problems_summary(problems);
        ui.horizontal(|ui| {
            ui.weak(summary);
        });
        let jumps = self.problem_rows(ui);
        for (line, col) in jumps {
            if let Some(live) = self.doc.doc.as_mut() {
                self.doc.engine.place_cursor(live, line, col);
            }
        }
    }

    fn problems_summary(problems: &[native_lsp::Diagnostic]) -> String {
        let errors = problems
            .iter()
            .filter(|problem| problem.severity == Some(native_lsp::Severity::Error))
            .count();
        let warnings = problems
            .iter()
            .filter(|problem| problem.severity == Some(native_lsp::Severity::Warning))
            .count();
        let mut summary = format!(
            "{} problem{}",
            problems.len(),
            if problems.len() == 1 { "" } else { "s" }
        );
        if errors > 0 {
            summary.push_str(&format!(
                " · {errors} error{}",
                if errors == 1 { "" } else { "s" }
            ));
        }
        if warnings > 0 {
            summary.push_str(&format!(
                " · {warnings} warning{}",
                if warnings == 1 { "" } else { "s" }
            ));
        }
        summary
    }

    /// One button per listed problem; returns the clicked positions.
    fn problem_rows(&self, ui: &mut egui::Ui) -> Vec<(usize, usize)> {
        let problems = &self.frame.problems;
        let mut jumps: Vec<(usize, usize)> = Vec::new();
        for problem in problems.iter().take(LISTED_PROBLEMS) {
            let line = usize::try_from(problem.range.start.line).unwrap_or(usize::MAX);
            let row = self
                .doc
                .doc
                .as_ref()
                .map(|live| live.line_text(line))
                .unwrap_or_default();
            let col = native_lsp::utf16_to_char_col(&row, problem.range.start.character);
            let tag = match problem.severity {
                Some(native_lsp::Severity::Error) => "E",
                Some(native_lsp::Severity::Warning) => "W",
                _ => "·",
            };
            let label = format!(
                "{tag} {}: {}",
                line.saturating_add(1),
                problem.message.lines().next().unwrap_or("")
            );
            if ui.small_button(label).clicked() {
                jumps.push((line, col));
            }
        }
        if problems.len() > LISTED_PROBLEMS {
            ui.weak(format!(
                "+{} more",
                problems.len().saturating_sub(LISTED_PROBLEMS)
            ));
        }
        jumps
    }

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
        if let Some(shown) = self.popups.signature.as_mut() {
            if self.requests.caret_rect.is_some() {
                shown.anchor = self.requests.caret_rect;
            }
            complete::signature_ui(ui, shown);
        }
    }

    pub(super) fn completion_after_frame(&mut self, ui: &mut egui::Ui, revision_before: u64) {
        let Some(live) = self.doc.doc.as_mut() else {
            return;
        };
        self.requests.completion_request = complete::after_frame(complete::AfterFrameInput {
            ui,
            popup: &mut self.popups.completion,
            buffer: live,
            engine: &mut self.doc.engine,
            revision_before,
            manual: self.requests.complete_now,
            caret_rect: self.requests.caret_rect,
            server: native_lsp::server_language(self.frame.language).is_some(),
        });
    }
}
