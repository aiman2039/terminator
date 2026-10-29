//! The native view's header row: breadcrumb, mode, and file actions.
use super::*;

impl ViewPass<'_> {
    /// Keyboard focus id of this view's source widget. Clicking a header
    /// button moves focus to it; buttons hand focus back to the file so
    /// arrows and typing keep editing text.
    pub(super) fn file_focus(&self) -> egui::Id {
        source_focus_id(&self.frame.source_id)
    }

    /// Draw the header: one fixed-height row. Returns a heading line
    /// picked from the Markdown heading menu, if any.
    pub(super) fn header(&mut self, ui: &mut egui::Ui) -> Option<usize> {
        let size = egui::vec2(ui.available_width(), appearance::TOOLBAR_BUTTON + 4.0);
        ui.allocate_ui_with_layout(
            size,
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.set_min_size(size);
                ui.set_max_height(size.y);
                self.header_row(ui)
            },
        )
        .inner
    }

    /// Right to left: actions keep their place at the right edge, and
    /// the breadcrumb takes the rest, truncated with an ellipsis on a
    /// narrow pane instead of wrapping to a second row.
    fn header_row(&mut self, ui: &mut egui::Ui) -> Option<usize> {
        self.header_recent(ui);
        self.header_bookmark(ui);
        self.header_references(ui);
        self.header_reload(ui);
        self.header_save(ui);
        self.header_status(ui);
        let heading_jump = self.header_mode(ui);
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            self.header_title(ui);
        });
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

    /// Dirty dot, then breadcrumb and caret symbol in the UI font as one
    /// label, so a narrow pane truncates them together.
    fn header_title(&self, ui: &mut egui::Ui) {
        let files = &self.frame.files;
        let font = egui::TextStyle::Body.resolve(ui.style());
        let visuals = ui.visuals();
        let (weak, text) = (visuals.weak_text_color(), visuals.text_color());
        if self.doc.dirty() {
            ui.label(egui::RichText::new("●").small().color(text))
                .on_hover_text("Unsaved changes");
        }
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &files.breadcrumb,
            0.0,
            egui::TextFormat::simple(font.clone(), weak),
        );
        if let Some(symbol) = self.current_symbol() {
            job.append(
                &format!("  ›  {symbol}"),
                0.0,
                egui::TextFormat::simple(font, text),
            );
        }
        ui.add(egui::Label::new(job).truncate())
            .on_hover_text(&files.file_location);
    }

    /// Markdown mode toggles and heading menu, else the Vim mode badge.
    fn header_mode(&mut self, ui: &mut egui::Ui) -> Option<usize> {
        if self.frame.is_markdown {
            // Right to left: modes, then the heading menu, then the
            // format buttons at the right.
            self.markdown_format(ui);
            let heading_jump = self.heading_menu(ui);
            self.markdown_modes(ui);
            return heading_jump;
        }
        if self.frame.vim {
            // Modeless editing has no mode to report: the badge
            // would always read NORMAL and only add noise.
            let mode = self.doc.engine.mode();
            let colors = appearance::editor_style().palette.modes;
            ui.label(
                egui::RichText::new(mode_name(mode))
                    .small()
                    .color(mode_color(mode, &colors)),
            );
        }
        None
    }

    /// Edit, Preview, Split from the left: listed in reverse for the
    /// right-to-left header.
    fn markdown_modes(&mut self, ui: &mut egui::Ui) {
        for (option, icon) in [
            (markdown::Mode::Split, "Columns2"),
            (markdown::Mode::Preview, "Eye"),
            (markdown::Mode::Edit, "Pencil"),
        ] {
            let response = appearance::selectable_icon(appearance::SelectableIconInput {
                ui,
                icon,
                tip: option.label(),
                selected: self.doc.markdown_mode == option,
            });
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
            let label = format!(
                "{}{title}",
                "  ".repeat(usize::from(level).saturating_sub(1))
            );
            if appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: &label,
                icon: "ListTree",
                shortcut: "",
            })
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
        if appearance::selectable_icon(appearance::SelectableIconInput {
            ui,
            icon: "FileText",
            tip: label,
            selected: pinned,
        })
        .clicked()
        {
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
            let pinned = files.pinned_files.contains(item);
            if appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: &name,
                icon: "FileText",
                shortcut: if pinned { "Bookmarked" } else { "" },
            })
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
