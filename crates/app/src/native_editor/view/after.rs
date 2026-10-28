//! What a native view's frame queued, run once the buffer borrow ends.
use super::*;

impl App {
    /// Put this file's popups back, and act on blame clicks.
    pub(super) fn native_view_check_in(
        &mut self,
        path: &Path,
        popups: ViewPopups,
        requests: &ViewRequests,
    ) {
        let ViewPopups {
            find,
            completion,
            signature,
            actions,
            gutter,
        } = popups;
        if find.is_some() {
            self.find_bar = find;
        }
        if completion.is_some() {
            self.completion = completion;
        }
        if gutter.is_some() {
            self.gutter_popup = gutter;
            self.native_view_blame(path, requests);
        }
        if signature.is_some() {
            self.signature = signature;
        }
        if actions.is_some() {
            self.code_actions = actions;
        }
    }

    fn native_view_blame(&mut self, path: &Path, requests: &ViewRequests) {
        if requests.blame_toggle {
            self.toggle_editor_blame(path);
        }
        if let Some(line) = requests.blame_click {
            self.open_blame_commit(path, line);
        }
    }

    /// Push the live buffer text past the sync debounce so the server sees
    /// what was just typed. Returns the pushed revision.
    fn push_live_text(&mut self, path: &Path, language: Language) -> Option<u64> {
        let (revision, text) = self
            .native_docs
            .get(path)
            .and_then(|doc| doc.buffer())
            .map(|buffer| (buffer.revision(), buffer.text()))?;
        self.lsp.submit_text(SubmitTextInput {
            path,
            language,
            revision,
            text: &text,
        });
        Some(revision)
    }

    /// A picked code action, then format and code-action requests.
    pub(super) fn native_view_edit_requests(
        &mut self,
        path: &Path,
        language: Language,
        requests: &mut ViewRequests,
    ) {
        if let Some(item) = requests.picked_action.take() {
            self.run_code_action(path, &item);
        }
        // Both act on the text as typed: send it past the debounce.
        if !(requests.format_now || requests.actions_at.is_some()) {
            return;
        }
        let Some(revision) = self.push_live_text(path, language) else {
            return;
        };
        if requests.format_now && !self.lsp.request_format(path, revision) {
            self.info = Some("The language server is not ready to format".into());
        }
        if let Some(range) = requests.actions_at.take() {
            self.native_view_code_actions(path, range, requests.caret_rect);
        }
    }

    fn native_view_code_actions(
        &mut self,
        path: &Path,
        range: native_lsp::Range,
        anchor: Option<egui::Rect>,
    ) {
        match self.lsp.request_code_actions(path, range) {
            Some(generation) => {
                self.code_actions = Some(ActionsPopup {
                    path: path.to_owned(),
                    generation: Some(generation),
                    items: Vec::new(),
                    selected: 0,
                    anchor,
                });
            }
            None => self.info = Some("The language server is not ready".into()),
        }
    }

    pub(super) fn native_view_signature(
        &mut self,
        path: &Path,
        language: Language,
        requests: &ViewRequests,
    ) {
        if let Some((position, line, open)) = requests.signature_request
            && self.push_live_text(path, language).is_some()
            && let Some(generation) = self.lsp.request_signature_help(path, position)
        {
            // Keep showing the previous signature until the update lands.
            let help = self
                .signature
                .take()
                .filter(|shown| shown.path == path && shown.open == open)
                .and_then(|shown| shown.help);
            self.signature = Some(SignaturePopup {
                path: path.to_owned(),
                generation,
                help,
                line,
                open,
                anchor: requests.caret_rect,
            });
        }
        if let Some((answer_path, generation, help)) = self.lsp.take_signature()
            && let Some(shown) = self.signature.as_mut()
            && shown.generation == generation
            && shown.path == answer_path
        {
            shown.help = help;
        }
    }

    /// Completion asks with the live text: push it past the sync debounce
    /// first so the server sees what was just typed.
    pub(super) fn native_view_completion(
        &mut self,
        path: &Path,
        language: Language,
        requests: &ViewRequests,
    ) {
        if let Some((position, line, start)) = requests.completion_request
            && self.push_live_text(path, language).is_some()
            && let Some(generation) = self.lsp.request_completion(path, position)
        {
            self.completion = Some(CompletionPopup {
                path: path.to_owned(),
                generation: Some(generation),
                items: Vec::new(),
                line,
                start,
                visible: Vec::new(),
                selected: 0,
                anchor: requests.caret_rect,
            });
        }
        if let Some(results) = self.lsp.take_completion()
            && let Some(popup) = self.completion.as_mut()
            && popup.generation == Some(results.generation)
            && popup.path == results.path
        {
            popup.items = results.items;
            popup.generation = None;
        }
    }

    /// Focus follow, file picker, bookmark toggle, and recent-file opens.
    pub(super) fn native_view_follow_up(
        &mut self,
        path: &Path,
        frame: &ViewFrame,
        requests: &mut ViewRequests,
    ) {
        if requests.activated_editor {
            self.follow_focus_tab(Some(Tab::NativeEditor {
                path: path.to_owned(),
            }));
        }
        if requests.open_file_picker {
            self.open_path = true;
        }
        if requests.toggle_pin {
            if frame.files.pinned {
                self.preferences.pinned_native_files.remove(path);
            } else {
                self.preferences.pinned_native_files.insert(path.to_owned());
            }
        }
        if let Some(path_to_open) = requests.recent_open.take()
            && let Some(uri) = native_lsp::file_uri(&path_to_open)
        {
            let at = native_lsp::Position {
                line: 0,
                character: 0,
            };
            self.pending_goto.push(lsp_manager::GotoJump {
                from: path.to_owned(),
                targets: vec![native_lsp::GotoTarget {
                    uri: uri.to_string(),
                    range: native_lsp::Range { start: at, end: at },
                }],
            });
        }
    }

    /// Inlay, hover, goto, references, hierarchy, and rename requests,
    /// then the answers that already landed.
    pub(super) fn native_view_lsp_requests(
        &mut self,
        site: ViewSite<'_>,
        requests: &mut ViewRequests,
    ) {
        let path = site.path;
        self.native_view_inlay_request(path);
        if let Some((position, cursor)) = requests.hover_at.take() {
            self.lsp.request_hover(path, position, cursor);
        }
        if let Some(position) = requests.goto_at.take() {
            self.lsp.request_definition(path, position);
        }
        if let Some(position) = requests.references_at.take() {
            self.lsp.request_references(path, position);
        }
        if let Some((position, kind)) = requests.hierarchy_at.take() {
            self.start_hierarchy(path, position, kind);
        }
        if let Some(position) = requests.rename_at.take() {
            self.begin_native_rename(path, site.project, position);
        }
        if let Some(results) = self.lsp.take_references() {
            self.show_usages("Find Usages", results.targets);
        }
        // Finished jumps queue until the workspace is checked back in:
        // this view renders while it is checked out, so opening the
        // target now would miss its project and drop the jump.
        if let Some(jump) = self.lsp.take_goto() {
            self.pending_goto.push(jump);
        }
    }

    fn native_view_inlay_request(&mut self, path: &Path) {
        if self.state.settings.inlay_hints
            && let Some(lines) = self
                .native_docs
                .get(path)
                .and_then(|doc| doc.doc.as_ref())
                .map(|buffer| buffer.line_count())
        {
            self.lsp.request_inlay_hints(path, lines);
        }
    }

    /// Deferred `:wq`, `:qa`, and splits.
    pub(super) fn native_view_deferred(
        &mut self,
        ui: &egui::Ui,
        path: &Path,
        requests: ViewRequests,
    ) {
        if requests.defer_close_after_save {
            self.native_close_after_save = Some(path.to_owned());
            self.native_close_after_save_issuer = requests.wq_issuer.map(CloseIssuer::deferred);
        }
        // `:qa` resolves at drain time: the current workspace is checked
        // out of `layouts` here, so its tabs are invisible to the dirty
        // check until check-in.
        if requests.quit_all.is_some() {
            self.pending_quit_all = requests.quit_all;
        }
        // The issuing viewport travels with the request so splits land
        // in the floating window they were typed in.
        let viewport = ui.ctx().viewport_id();
        for split in requests.splits {
            self.pending_native_splits
                .push((viewport, path.to_owned(), split));
        }
    }
}
