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
        if let Some((line, web)) = requests.blame_click {
            self.open_blame_commit(path, line, web);
        }
    }

    /// Push the live buffer text past the sync debounce so the server sees
    /// what was just typed. Returns the pushed revision.
    pub(super) fn push_live_text(&mut self, path: &Path, language: Language) -> Option<u64> {
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
        if let Some((item, revision)) = requests.picked_action.take() {
            self.run_code_action(path, &item, revision);
        }
        // Both act on the text as typed: send it past the debounce.
        if !(requests.format_now || requests.actions_at.is_some()) {
            return;
        }
        let Some(revision) = self.push_live_text(path, language) else {
            return;
        };
        if requests.format_now {
            // A selection formats only itself when the server can.
            let sent = match requests.format_range.take() {
                Some(range) if self.lsp.request_range_format(path, revision, range) => true,
                _ => self.lsp.request_format(path, revision),
            };
            if !sent {
                self.info = Some("The language server is not ready to format".into());
            }
        }
        if let Some(range) = requests.actions_at.take() {
            self.native_view_code_actions(path, (range, revision), requests.caret_rect);
        }
    }

    /// Open the Alt+Enter popup at once with the built-in items; the
    /// server's actions join it when they arrive.
    fn native_view_code_actions(
        &mut self,
        path: &Path,
        (range, revision): (native_lsp::Range, u64),
        anchor: Option<egui::Rect>,
    ) {
        let generation = self.lsp.request_code_actions(path, range);
        self.code_actions = Some(ActionsPopup {
            path: path.to_owned(),
            builtins: actions::BuiltinAction::for_file(generation.is_some()),
            generation,
            revision,
            items: Vec::new(),
            selected: 0,
            anchor,
        });
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
                prefix: String::new(),
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
        (ctx, path): (&egui::Context, &Path),
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
        if requests.open_local_history {
            self.open_local_history(path.to_owned());
        }
        // The diff tab opens once the workspace is checked back in.
        if requests.gutter_diff {
            self.pending_gutter_diffs.push(path.to_owned());
        }
        if let Some((mark, text)) = requests.gutter_stage.take() {
            self.stage_gutter_block(path, mark, text);
        }
        if requests.toggle_pin {
            if frame.files.pinned {
                self.preferences.pinned_native_files.remove(path);
            } else {
                self.preferences.pinned_native_files.insert(path.to_owned());
            }
        }
        if let Some(path_to_open) = requests.recent_open.take() {
            self.queue_jump(path, &path_to_open, (0, 0));
        }
        if let Some(name) = requests.goto_file.take() {
            self.start_goto_file(ctx, path, name);
        }
        self.finish_goto_file(path);
        if requests.alternate_file {
            match self.alternate_location() {
                Some(target) if target.path != path => {
                    self.queue_jump(path, &target.path, (target.line, target.character));
                }
                _ => self.info = Some("No alternate file".to_owned()),
            }
        }
    }

    /// `gf`: resolve the name on the file pool (it stats candidates, which
    /// can block on slow volumes), never on the GUI thread.
    fn start_goto_file(&mut self, ctx: &egui::Context, path: &Path, name: String) {
        let root = self.selected_project().map(|project| project.path.clone());
        let (current, wanted, ctx) = (path.to_owned(), name.clone(), ctx.clone());
        let rx = super::super::disk::run_on_files(
            &self.services,
            ("native-goto-file", path, Policy::ReplaceableRead),
            move || {
                let target = goto_file::resolve_goto_file(&wanted, &current, root.as_deref());
                ctx.request_repaint();
                Ok(target)
            },
        );
        match (rx, self.native_docs.get_mut(path)) {
            (Some(rx), Some(doc)) => doc.extras.goto = Some((name, rx)),
            _ => self.info = Some("Could not look up the file".to_owned()),
        }
    }

    /// Jump once the `gf` lookup landed.
    fn finish_goto_file(&mut self, path: &Path) {
        let Some(doc) = self.native_docs.get_mut(path) else {
            return;
        };
        match doc.extras.take_goto() {
            Some((_, Some((target, line, column)))) => {
                self.queue_jump(path, &target, (line, column));
            }
            Some((name, None)) => self.info = Some(format!("Can't find file \"{name}\" in path")),
            None => {}
        }
    }

    /// Open `target` at an LSP (zero-based, UTF-16) position once the
    /// workspace is checked back in; the jump is recorded for Back.
    fn queue_jump(&mut self, from: &Path, target: &Path, (line, character): (u32, u32)) {
        let Some(uri) = native_lsp::file_uri(target) else {
            return;
        };
        let at = native_lsp::Position { line, character };
        self.pending_goto.push(lsp_manager::GotoJump {
            from: from.to_owned(),
            targets: vec![native_lsp::GotoTarget {
                uri: uri.to_string(),
                range: native_lsp::Range { start: at, end: at },
            }],
        });
    }

    /// Inlay, hover, goto, references, hierarchy, and rename requests,
    /// then the answers that already landed.
    pub(super) fn native_view_lsp_requests(
        &mut self,
        site: ViewSite<'_>,
        language: Language,
        requests: &mut ViewRequests,
    ) {
        let path = site.path;
        self.native_view_inlay_request(path);
        // Positions come from the live buffer: send text still waiting on
        // the sync debounce so the server answers for the same text.
        if requests.hover_at.is_some()
            || requests.goto_at.is_some()
            || requests.references_at.is_some()
            || requests.locate_at.is_some()
            || requests.hierarchy_at.is_some()
        {
            let _ = self.push_live_text(path, language);
        }
        if let Some((position, cursor)) = requests.hover_at.take() {
            self.lsp.request_hover(path, position, cursor);
        }
        if let Some(position) = requests.goto_at.take() {
            self.lsp.request_definition(path, position);
        }
        if let Some(position) = requests.references_at.take() {
            self.lsp.request_references(path, position);
        }
        if let Some((position, kind)) = requests.locate_at.take()
            && let Err(note) = self.lsp.request_locate(path, position, kind)
        {
            self.info = Some(note);
        }
        if let Some(seed) = requests.find_in_files.take() {
            self.open_find_in_files(site.project, seed);
        }
        if let Some((position, kind)) = requests.hierarchy_at.take() {
            self.start_hierarchy(path, position, kind);
        }
        if let Some(position) = requests.rename_at.take() {
            self.begin_native_rename(path, site.project, position);
        }
        if let Some(lines) = requests.agent_context.take() {
            self.add_to_agent_context(path, site.project, lines);
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
            self.native_close_after_save.insert(
                path.to_owned(),
                requests.wq_issuer.map(CloseIssuer::deferred),
            );
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
