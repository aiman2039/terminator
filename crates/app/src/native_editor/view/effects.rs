//! Engine effects a native view's source widget reports.
use super::*;

impl ViewPass<'_> {
    /// Act on one engine effect. Layout changes and language-server
    /// requests only queue here; they run after the buffer borrow.
    pub(super) fn apply_effect(&mut self, ui: &egui::Ui, effect: Effect, post: &mut PostEdit) {
        match effect {
            Effect::Save => self.save(false),
            Effect::SaveForce => self.save(true),
            Effect::Quit => self.quit(post),
            Effect::QuitForce => *post = PostEdit::Close { force: true },
            Effect::WriteQuit { force } => self.write_quit(ui, force),
            Effect::QuitAll { force } => self.requests.quit_all = Some(force),
            Effect::Split => self.requests.splits.push(NativeSplit::Below),
            Effect::Vsplit => self.requests.splits.push(NativeSplit::Beside),
            Effect::TabNew => self.requests.splits.push(NativeSplit::NewTab),
            Effect::OpenFilePicker => self.requests.open_file_picker = true,
            Effect::Hover => self.request_hover(),
            Effect::GotoDefinition => self.requests.goto_at = Some(self.caret_at()),
            Effect::GotoImplementation => {
                self.requests.locate_at =
                    Some((self.caret_at(), native_lsp::Locate::Implementation));
            }
            Effect::GotoTypeDefinition => {
                self.requests.locate_at =
                    Some((self.caret_at(), native_lsp::Locate::TypeDefinition));
            }
            Effect::FindInFiles => self.requests.find_in_files = Some(self.one_line_selection()),
            Effect::FindReferences => self.requests.references_at = Some(self.caret_at()),
            Effect::Hierarchy { types } => self.request_hierarchy(types),
            Effect::RenameSymbol => self.requests.rename_at = Some(self.caret_at()),
            Effect::Complete => self.requests.complete_now = true,
            Effect::Format => {
                self.requests.format_now = true;
                self.requests.format_range = self.selection_range();
            }
            Effect::CodeActions => self.request_code_actions(),
            Effect::FindBar { replace } => self.open_find_bar(replace),
            Effect::Bell => {}
        }
    }

    /// `:q` refuses a dirty buffer.
    fn quit(&mut self, post: &mut PostEdit) {
        if self.doc.dirty() {
            self.doc.save_error = Some("No write since last change (add ! to override)".into());
        } else {
            *post = PostEdit::Close { force: false };
        }
    }

    /// The write is async: closing here would always see a dirty buffer
    /// and refuse, so defer to the save-settled check instead.
    fn write_quit(&mut self, ui: &egui::Ui, force: bool) {
        self.save(force);
        self.requests.defer_close_after_save = true;
        let ViewSite {
            project, tab, node, ..
        } = self.site;
        self.requests.wq_issuer = App::close_issuer(CloseIssuerInput {
            ui,
            project,
            tab,
            node,
        });
    }

    fn request_hover(&mut self) {
        let cursor = self.doc.engine.cursor();
        self.requests.hover_at = Some((self.caret_at(), (cursor.line, cursor.col)));
    }

    fn request_hierarchy(&mut self, types: bool) {
        let kind = if types {
            native_lsp::HierarchyKind::Type
        } else {
            native_lsp::HierarchyKind::Call
        };
        self.requests.hierarchy_at = Some((self.caret_at(), kind));
    }

    /// The selection, else the caret, as an LSP range.
    fn request_code_actions(&mut self) {
        let Some(live) = self.doc.doc.as_ref() else {
            return;
        };
        let caret = self.doc.engine.cursor();
        let (from, to) = self.doc.engine.selection().unwrap_or((caret, caret));
        self.requests.actions_at = Some(lsp_range(live, from, to));
    }

    /// The selection as an LSP range, when one exists.
    fn selection_range(&self) -> Option<native_lsp::Range> {
        let live = self.doc.doc.as_ref()?;
        let (from, to) = self.doc.engine.selection()?;
        Some(lsp_range(live, from, to))
    }

    /// The selected text when it is one line (a find seed), else empty.
    fn one_line_selection(&self) -> String {
        self.selected_text()
            .filter(|text| !text.contains('\n'))
            .unwrap_or_default()
    }

    fn selected_text(&self) -> Option<String> {
        self.doc
            .doc
            .as_ref()
            .filter(|_| self.doc.engine.selection().is_some())
            .map(|live| self.doc.engine.copy_text(live))
    }

    /// Open (or refocus) the find bar, seeded with a one-line selection,
    /// IntelliJ-style. A multi-line selection scopes the search to it
    /// ("In selection").
    fn open_find_bar(&mut self, replace: bool) {
        let multi_line = self
            .selected_text()
            .is_some_and(|text| text.trim_end_matches('\n').contains('\n'));
        let seed = self.one_line_selection();
        let bar = self.popups.find.get_or_insert_with(|| {
            find::FindBar::new(self.site.path.to_owned(), String::new(), replace)
        });
        if !seed.is_empty() {
            bar.query = seed;
        }
        bar.replacing |= replace;
        bar.focus = true;
        if multi_line && let Some(live) = self.doc.doc.as_ref() {
            bar.scope_to_selection(&self.doc.engine, live);
        }
    }
}

/// Two carets as an LSP (UTF-16) range.
fn lsp_range(
    live: &Doc,
    from: terminator_native_edit::vim::Cursor,
    to: terminator_native_edit::vim::Cursor,
) -> native_lsp::Range {
    let position = |cursor: terminator_native_edit::vim::Cursor| native_lsp::Position {
        line: u32::try_from(cursor.line).unwrap_or(u32::MAX),
        character: native_lsp::char_col_to_utf16(&live.line_text(cursor.line), cursor.col),
    };
    native_lsp::Range {
        start: position(from),
        end: position(to),
    }
}
