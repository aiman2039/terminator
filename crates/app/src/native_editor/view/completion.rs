//! The completion popup's requests and answers once the view drew: list
//! requests (with why they were asked), re-requests for an incomplete
//! list, the debounced docs lookup, the docs panel, and auto-import edits
//! that arrive after an item was accepted.
use super::*;
use crate::lsp_manager::{CompletionResults, ResolveFor};

/// While an answer is due, the frame looks for it this often, for up to
/// [`ANSWER_WAIT`]: the language-server pump does not wake the GUI.
const ANSWER_POLL: Duration = Duration::from_millis(30);
const ANSWER_WAIT: Duration = Duration::from_secs(2);

impl App {
    /// Completion asks with the live text: push it past the sync debounce
    /// first so the server sees what was just typed.
    pub(super) fn native_view_completion(
        &mut self,
        (ui, path): (&egui::Ui, &Path),
        language: Language,
        requests: &ViewRequests,
    ) {
        if let Some(ask) = requests.completion_ask
            && self.push_live_text(path, language).is_some()
            && let Some(generation) = self.lsp.request_completion(path, ask.position, ask.trigger)
        {
            self.native_completion_asked(path, (ask, generation), requests.caret_rect);
        }
        if let Some(results) = self.lsp.take_completion() {
            self.native_completion_answered(results);
        }
        for answer in self.lsp.take_resolved_completions(path) {
            match answer.target {
                ResolveFor::Popup { generation, index } => {
                    if let Some(popup) = self.completion.as_mut().filter(|popup| popup.path == path)
                    {
                        popup.merge_resolved((generation, index), answer.item);
                    }
                }
                ResolveFor::Accepted { line, col, before } => {
                    self.native_completion_imports(path, (line, col, before), answer.item);
                }
            }
        }
        self.native_completion_docs(ui, path);
    }

    /// A list request went out. An incomplete list stays on screen until
    /// its update lands; any other request opens a fresh popup.
    fn native_completion_asked(
        &mut self,
        path: &Path,
        (ask, generation): (complete::CompletionAsk, u64),
        anchor: Option<egui::Rect>,
    ) {
        if ask.trigger == native_lsp::CompletionTrigger::Incomplete
            && let Some(popup) = self.completion.as_mut().filter(|popup| popup.path == path)
        {
            popup.generation = Some(generation);
            popup.waiting = Some(Instant::now());
            return;
        }
        self.completion = Some(CompletionPopup::new(
            path,
            generation,
            (ask.line, ask.start),
            anchor,
        ));
    }

    /// Show the answer to the popup's newest request. A complete answer
    /// with no items closes the popup, so the next keystroke asks again.
    fn native_completion_answered(&mut self, results: CompletionResults) {
        let Some(popup) = self.completion.as_mut().filter(|popup| {
            popup.generation == Some(results.generation) && popup.path == results.path
        }) else {
            return;
        };
        popup.adopt(results);
        if popup.items.is_empty() && !popup.incomplete {
            self.completion = None;
        }
    }

    /// Auto-import edits for an item accepted before they were known: one
    /// undo step, applied only while they still fit the text. The carets
    /// stay on their text, so a selected snippet stop stays selected, and
    /// an open popup on the caret's line moves with it.
    fn native_completion_imports(
        &mut self,
        path: &Path,
        at: (usize, usize, u64),
        item: native_lsp::CompletionItem,
    ) {
        if item.additional.is_empty() {
            return;
        }
        let Some(doc) = self.native_docs.get_mut(path) else {
            return;
        };
        let Some(buffer) = doc.doc.as_mut() else {
            return;
        };
        if !complete::imports_fit(buffer, at, &item.additional) {
            self.info = Some(format!(
                "The import for {} was not added: the file changed first",
                item.label
            ));
            return;
        }
        let carets = doc.engine.caret_offsets(buffer);
        let before = doc.engine.cursor();
        actions::apply_text_edits(buffer, &item.additional);
        doc.engine.follow_carets(buffer, carets);
        let after = doc.engine.cursor();
        if let Some(popup) = self
            .completion
            .as_mut()
            .filter(|popup| popup.path == path && popup.line == before.line)
        {
            popup.line = after.line;
            popup.start = popup
                .start
                .saturating_add(after.col)
                .saturating_sub(before.col);
        }
    }

    /// Ask for the selected item's docs once the selection rests, keep
    /// the frame looking for due answers, and draw the docs panel.
    fn native_completion_docs(&mut self, ui: &egui::Ui, path: &Path) {
        let Some(popup) = self.completion.as_mut().filter(|popup| popup.path == path) else {
            return;
        };
        let now = Instant::now();
        match complete::resolve_due(popup, now) {
            complete::Due::Now(index) => {
                let target = ResolveFor::Popup {
                    generation: popup.list,
                    index,
                };
                let item = popup.items.get(index).map(|item| &item.raw);
                if let Some(raw) = item
                    && self.lsp.request_completion_resolve(path, raw, target)
                {
                    popup.waiting = Some(now);
                }
            }
            complete::Due::Later(wait) => ui.ctx().request_repaint_after(wait),
            complete::Due::Never => {}
        }
        if popup
            .waiting
            .is_some_and(|since| now.saturating_duration_since(since) < ANSWER_WAIT)
        {
            ui.ctx().request_repaint_after(ANSWER_POLL);
        }
        let cache = &mut self.native_hover_markdown_cache;
        complete::docs_ui(ui, popup, |ui, doc| {
            support::lsp_markup(ui, cache, (&doc.text, doc.markdown));
        });
    }
}

#[cfg(test)]
#[path = "completion_tests.rs"]
mod tests;
