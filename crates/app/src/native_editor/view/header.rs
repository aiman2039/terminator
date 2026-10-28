//! The native view's header row: breadcrumb, mode, and file actions.
use super::*;

impl ViewPass<'_> {
    /// Keyboard focus id of this view's source widget. Clicking a header
    /// button moves focus to it; buttons hand focus back to the file so
    /// arrows and typing keep editing text.
    pub(super) fn file_focus(&self) -> egui::Id {
        source_focus_id(&self.frame.source_id)
    }

    /// Draw the header. Returns a heading line picked from the Markdown
    /// heading menu, if any.
    pub(super) fn header(&mut self, ui: &mut egui::Ui) -> Option<usize> {
        // Wrapped: on a narrow pane the mode badge and buttons move to
        // a second row instead of being clipped off the edge.
        ui.horizontal_wrapped(|ui| self.header_row(ui)).inner
    }

    fn header_row(&mut self, ui: &mut egui::Ui) -> Option<usize> {
        self.header_title(ui);
        let heading_jump = self.header_mode(ui);
        self.header_status(ui);
        self.header_save(ui);
        self.header_reload(ui);
        self.header_references(ui);
        self.header_bookmark(ui);
        self.header_recent(ui);
        heading_jump
    }

    /// Innermost outline symbol around the caret, for the breadcrumb.
    fn current_symbol(&self) -> Option<&str> {
        self.doc.doc.as_ref().and_then(|_| {
            let line = u32::try_from(self.doc.engine.cursor().line).ok()?;
            let outline = &self.frame.outline;
            native_lsp::innermost_symbol(outline, line)
                .and_then(|index| outline.get(index))
                .map(|symbol| symbol.name.as_str())
        })
    }

    fn header_title(&self, ui: &mut egui::Ui) {
        let files = &self.frame.files;
        let reserved = if self.frame.is_markdown { 260.0 } else { 200.0 };
        let title_width = (ui.available_width() - reserved).clamp(48.0, 260.0);
        ui.add_sized(
            [title_width, appearance::TOOLBAR_BUTTON],
            egui::Label::new(egui::RichText::new(&files.breadcrumb).weak()).truncate(),
        )
        .on_hover_text(&files.file_location);
        if self.doc.crlf {
            ui.weak("CRLF")
                .on_hover_text("Windows line endings; saving keeps them");
        }
        if let Some(symbol) = self.current_symbol() {
            ui.weak("›");
            ui.monospace(symbol);
        }
        if self.doc.dirty() {
            ui.monospace("●");
        }
    }

    /// Markdown mode toggles and heading menu, else the Vim mode badge.
    fn header_mode(&mut self, ui: &mut egui::Ui) -> Option<usize> {
        if self.frame.is_markdown {
            self.markdown_modes(ui);
            return self.heading_menu(ui);
        }
        if self.frame.vim {
            // Modeless editing has no mode to report: the badge
            // would always read NORMAL and only add noise.
            let mode = self.doc.engine.mode();
            ui.colored_label(mode_color(mode), format!("-- {} --", mode_name(mode)));
        }
        None
    }

    fn markdown_modes(&mut self, ui: &mut egui::Ui) {
        for (option, icon) in [
            (markdown::Mode::Edit, "Pencil"),
            (markdown::Mode::Preview, "Eye"),
            (markdown::Mode::Split, "Columns2"),
        ] {
            let response = appearance::selectable_icon(
                ui,
                icon,
                option.label(),
                self.doc.markdown_mode == option,
            );
            #[cfg(feature = "test-support")]
            crate::diagnostics::record(
                ui.ctx(),
                &format!("native-markdown-mode:{}", option.label()),
                response.rect,
            );
            if response.clicked() {
                self.doc.markdown_mode = option;
                if option == markdown::Mode::Edit {
                    ui.memory_mut(|memory| memory.request_focus(self.file_focus()));
                }
            }
        }
    }

    fn heading_menu(&self, ui: &mut egui::Ui) -> Option<usize> {
        let mut heading_jump = None;
        appearance::icon_menu_button(ui, "ListTree", |ui| {
            heading_jump = Self::heading_items(ui, self.doc.doc.as_ref());
        })
        .response
        .on_hover_text("Go to heading");
        heading_jump
    }

    /// One button per Markdown heading; returns the clicked heading's line.
    fn heading_items(ui: &mut egui::Ui, buffer: Option<&Doc>) -> Option<usize> {
        let buffer = buffer?;
        let mut jump = None;
        let mut found = false;
        for block in parse_blocks(buffer) {
            let BlockKind::Heading(level) = block.kind else {
                continue;
            };
            found = true;
            let title = buffer.line_text(block.start_line);
            let title = title.trim_start_matches('#').trim();
            if ui
                .button(format!("{} {title}", "·".repeat(level.into())))
                .clicked()
            {
                jump = Some(block.start_line);
                ui.close();
            }
        }
        if !found {
            ui.weak("No headings");
        }
        jump
    }

    fn header_status(&self, ui: &mut egui::Ui) {
        if self.doc.saving {
            ui.weak("Saving…");
        }
        if self.frame.rename_applying {
            ui.weak("Renaming…");
        }
    }

    fn header_save(&mut self, ui: &mut egui::Ui) {
        let enabled = self.doc.dirty() && !self.doc.saving && !self.frame.rename_applying;
        if ui
            .add_enabled_ui(enabled, |ui| {
                appearance::sidebar_action(ui, "Save", "Save file")
            })
            .inner
            .clicked()
        {
            self.save(false);
            ui.memory_mut(|memory| memory.request_focus(self.file_focus()));
        }
    }

    /// Reload while dirty would silently discard edits: disabled.
    fn header_reload(&mut self, ui: &mut egui::Ui) {
        let enabled = !self.doc.dirty() && !self.doc.loading && !self.frame.rename_applying;
        let reload = ui
            .add_enabled_ui(enabled, |ui| {
                appearance::sidebar_action(ui, "RefreshCw", "Reload file")
            })
            .inner
            .on_disabled_hover_text("Unsaved changes are kept");
        if reload.clicked() {
            self.doc.load_error = None;
            self.doc.loading = false;
            self.doc.load_rx = None;
            self.doc.doc = None;
            ui.memory_mut(|memory| memory.request_focus(self.file_focus()));
        }
    }

    fn header_references(&mut self, ui: &mut egui::Ui) {
        if native_lsp::server_language(self.frame.language).is_some()
            && appearance::sidebar_action(ui, "ListTree", "Find references").clicked()
        {
            self.requests.references_at = Some(self.caret_at());
            ui.memory_mut(|memory| memory.request_focus(self.file_focus()));
        }
    }

    fn header_bookmark(&mut self, ui: &mut egui::Ui) {
        let pinned = self.frame.files.pinned;
        let label = if pinned {
            "Remove bookmark"
        } else {
            "Bookmark file"
        };
        if appearance::selectable_icon(ui, "FileText", label, pinned).clicked() {
            self.requests.toggle_pin = true;
            ui.memory_mut(|memory| memory.request_focus(self.file_focus()));
        }
    }

    fn header_recent(&mut self, ui: &mut egui::Ui) {
        let files = &self.frame.files;
        let requests = &mut *self.requests;
        appearance::icon_menu_button(ui, "History", |ui| {
            if let Some(item) = Self::recent_items(ui, files) {
                requests.recent_open = Some(item);
            }
        })
        .response
        .on_hover_text("Recent files");
    }

    /// The recent-files menu; returns the clicked file.
    fn recent_items(ui: &mut egui::Ui, files: &ViewFiles) -> Option<PathBuf> {
        if files.recent_files.is_empty() {
            ui.weak("No other recent files in this project");
        }
        let mut picked = None;
        for item in &files.recent_files {
            let name = item.file_name().unwrap_or_default().to_string_lossy();
            let marker = if files.pinned_files.contains(item) {
                "◆ "
            } else {
                ""
            };
            if ui
                .button(format!("{marker}{name}"))
                .on_hover_text(item.display().to_string())
                .clicked()
            {
                picked = Some(item.clone());
                ui.close();
            }
        }
        picked
    }
}
