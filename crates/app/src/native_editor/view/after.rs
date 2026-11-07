//! What a native view's frame queued, run once the buffer borrow ends.
use super::*;

impl App {
    /// Put this file's popups back, show or hide blame, and act on blame
    /// clicks.
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
        // The menu's Annotate needs no gutter popup.
        if requests.blame_toggle {
            self.toggle_editor_blame(path);
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
        // First, before anything sends the text with the accepted item in
        // it: the server resolves against the text it listed the item for.
        if let Some(accepted) = requests.accepted_completion.take() {
            self.lsp
                .request_completion_resolve(path, &accepted.raw, accepted.target);
        }
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

    /// Focus follow, file picker, gutter actions, and file jumps.
    pub(super) fn native_view_follow_up(
        &mut self,
        (ctx, path): (&egui::Context, &Path),
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
        // The picker opens a tab only later, outside the workspace render.
        if let Some(pick) = requests.git_compare.take() {
            self.open_compare_picker(path, pick.into());
        }
        // The diff tab opens once the workspace is checked back in.
        if requests.gutter_diff {
            self.pending_gutter_diffs.push(path.to_owned());
        }
        if let Some((mark, text)) = requests.gutter_stage.take() {
            self.stage_gutter_block(path, mark, text);
        }
        if let Some(request) = requests.goto_file.take() {
            self.start_goto_file(ctx, path, request);
        }
        self.finish_goto_file(path);
        if requests.alternate_file {
            match self.alternate_location() {
                Some(target) if target.path != path => {
                    self.queue_jump(path, &target.path, Some((target.line, target.character)));
                }
                _ => self.native_notice(path, "E23: No alternate file"),
            }
        }
    }

    /// `gf` and `:e`: resolve the name on the file pool (it stats
    /// candidates, which can block on slow volumes), never on the GUI
    /// thread. `edit` (`:e`, `:sp file`) also accepts a new file; `place`
    /// opens it in a split or a new tab instead of the focused pane.
    fn start_goto_file(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        (name, edit, place): (
            String,
            bool,
            Option<terminator_native_edit::vim::SplitPlace>,
        ),
    ) {
        let root = self.selected_project().map(|project| project.path.clone());
        let (current, wanted, ctx) = (path.to_owned(), name.clone(), ctx.clone());
        let rx = super::super::disk::run_on_files(
            &self.services,
            ("native-goto-file", path, Policy::ReplaceableRead),
            move || {
                let target = if edit {
                    goto_file::resolve_edit_file(&wanted, &current, root.as_deref())
                } else {
                    goto_file::resolve_goto_file(&wanted, &current, root.as_deref())
                        .map(|(path, line, column)| (path, line, column, false))
                };
                ctx.request_repaint();
                Ok(target)
            },
        );
        match (rx, self.native_docs.get_mut(path)) {
            (Some(rx), Some(doc)) => doc.extras.goto = Some((name, place, rx)),
            _ => self.info = Some("Could not look up the file".to_owned()),
        }
    }

    /// Jump once the `gf` lookup landed.
    fn finish_goto_file(&mut self, path: &Path) {
        let Some(doc) = self.native_docs.get_mut(path) else {
            return;
        };
        match doc.extras.take_goto() {
            Some((_, place, Some((target, line, column, new)))) => {
                if new {
                    self.native_nav.new_files.insert(target.clone());
                }
                match place {
                    None => self.queue_jump(path, &target, Some((line, column))),
                    Some(place) => {
                        let line = line.saturating_add(1);
                        self.native_nav
                            .split_opens
                            .push((path.to_owned(), target, line, place));
                    }
                }
            }
            Some((name, _, None)) => {
                self.native_notice(path, format!("E447: Can't find file \"{name}\" in path"));
            }
            None => {}
        }
    }

    /// Open `target` at an LSP (zero-based, UTF-16) position once the
    /// workspace is checked back in; the jump is recorded for Back. With
    /// no position, the file opens where it was left.
    pub(super) fn queue_jump(&mut self, from: &Path, target: &Path, position: Option<(u32, u32)>) {
        let Some(uri) = native_lsp::file_uri(target) else {
            return;
        };
        let (line, character) = position.unwrap_or_default();
        let at = native_lsp::Position { line, character };
        self.pending_goto.push(lsp_manager::GotoJump {
            from: from.to_owned(),
            targets: vec![native_lsp::GotoTarget {
                uri: uri.to_string(),
                range: native_lsp::Range { start: at, end: at },
            }],
            at_position: position.is_some(),
        });
    }

    /// Inlay, goto, references, hierarchy, and rename requests, then the
    /// answers that already landed. Quick Documentation asks on its own
    /// ([`Self::native_quick_doc`]).
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
        if requests.goto_at.is_some()
            || requests.references_at.is_some()
            || requests.locate_at.is_some()
            || requests.hierarchy_at.is_some()
        {
            let _ = self.push_live_text(path, language);
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
        match requests.find_in_files.take() {
            Some((seed, true)) => self.open_replace_in_files(site.project, seed),
            Some((seed, false)) => self.open_find_in_files(site.project, seed),
            None => {}
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
            let request = NativeCloseRequest::new(
                path.to_owned(),
                requests.wq_issuer.map(CloseIssuer::deferred),
                CloseOwner::Pane,
            );
            // A repeated `:wq` replaces the pane's own earlier request.
            self.native_close_after_save
                .retain(|r| !(r.path == path && r.owner == CloseOwner::Pane));
            self.native_close_after_save.push(request);
        }
        // `:qa` resolves at drain time: the current workspace is checked
        // out of `layouts` here, so its tabs are invisible to the dirty
        // check until check-in.
        if requests.quit_all.is_some() {
            self.pending_quit_all = requests.quit_all;
        }
        if requests.history.is_some() {
            self.native_nav.history = requests.history;
        }
        if requests.pane_move.is_some() {
            self.native_nav.pane_move = requests.pane_move;
        }
        if requests.cycle_tab.is_some() {
            self.native_nav.cycle_tab = requests.cycle_tab;
        }
        if requests.save_all {
            self.native_save_all();
        }
        if requests.recent_files {
            self.run_shortcut(ui.ctx(), crate::chrome::shortcuts::Action::RecentFiles);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_annotate_toggles_blame_without_a_gutter_popup() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, terminator_core::Paths::at(dir.path().into()));
        let path = dir.path().join("a.rs");
        let popups = || ViewPopups {
            find: None,
            completion: None,
            signature: None,
            actions: None,
            gutter: None,
        };
        let requests = ViewRequests {
            blame_toggle: true,
            ..Default::default()
        };
        app.native_view_check_in(&path, popups(), &requests);
        assert!(app.editor_blame.contains_key(&path), "Annotate shows blame");
        app.native_view_check_in(&path, popups(), &requests);
        assert!(
            !app.editor_blame.contains_key(&path),
            "Annotate again hides it"
        );
    }
}
