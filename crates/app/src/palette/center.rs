use super::*;

impl App {
    pub(crate) fn palette_center(&mut self, ui: &mut egui::Ui) {
        // Adopt finished background scans while the palette is open.
        self.poll_file_index();
        self.lsp.poll();
        self.lsp.request_open_document_symbols();
        ui.set_min_size(ui.available_size());
        let project_name = self
            .selected_project()
            .map(|project| project.name.clone())
            .unwrap_or_default();
        self.palette_title_row(ui, &project_name);
        ui.add_space(8.0);
        self.palette_filter_chips(ui);
        self.palette_search_field(ui, &project_name);
        self.palette_request_symbols(ui);
        if self.lsp.workspace_symbols_pending() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        self.refresh_palette_cache();
        let items = self
            .palette
            .cache
            .as_ref()
            .map_or_else(Vec::new, |cache| cache.items.clone());
        if items.is_empty() {
            ui.weak(if self.lsp.workspace_symbols_pending() {
                "Searching symbols…"
            } else {
                "No matches."
            });
            return;
        }
        let keyboard_moved = self.palette_keyboard_nav(ui, items.len());
        let mut chosen = self.palette_result_list(ui, &items, keyboard_moved);
        if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            chosen = Some(self.palette.index);
        }
        if let Some(index) = chosen
            && let Some(item) = items.get(index).cloned()
        {
            self.run_palette(ui.ctx(), item);
        }
    }

    fn palette_title_row(&mut self, ui: &mut egui::Ui, project_name: &str) {
        ui.horizontal(|ui| {
            ui.strong("Search");
            if !project_name.is_empty() {
                ui.weak(format!("· {project_name}"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    self.close_palette();
                }
            });
        });
    }

    fn palette_filter_chips(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (name, bit) in PALETTE_FILTERS {
                let on = self.palette.filters & bit != 0;
                let response = ui
                    .add(
                        egui::Button::new(*name)
                            .small()
                            .selected(on)
                            .frame(true)
                            .corner_radius(egui::CornerRadius::same(appearance::WIDGET_RADIUS)),
                    )
                    .on_hover_text(format!("Show {name}"));
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), &format!("palette-filter:{name}"), response.rect);
                if response.clicked() {
                    self.toggle_palette_filter(*bit, on);
                }
            }
        });
    }

    fn toggle_palette_filter(&mut self, bit: u8, on: bool) {
        // A chip leaves File Structure for the full search.
        self.palette.file_scope = None;
        if on {
            self.palette.filters &= !bit;
        } else {
            self.palette.filters |= bit;
        }
        self.palette.index = 0;
    }

    fn palette_search_field(&mut self, ui: &mut egui::Ui, project_name: &str) {
        let scope_name = self
            .palette
            .file_scope
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned());
        let hint = if let Some(name) = scope_name {
            format!("File structure of {name}…")
        } else if project_name.is_empty() {
            "Search…".to_string()
        } else {
            format!("Search {project_name}…")
        };
        let search = ui.add(
            appearance::singleline(&mut self.palette.query)
                .hint_text(hint)
                .desired_width(f32::INFINITY),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "palette-search", search.rect);
        if search.changed() {
            self.palette.index = 0;
            self.palette.query_changed_at = Instant::now();
            self.lsp.clear_workspace_symbols();
        }
        // A pressed pointer is choosing a filter. Focus returns on release
        // so that click is not taken by the field.
        if !ui.input(|input| input.pointer.primary_down()) {
            search.request_focus();
        }
    }

    fn palette_request_symbols(&mut self, ui: &egui::Ui) {
        if let Some(scope) = self.palette.file_scope.clone() {
            // The outline is the file's own; no workspace query.
            if !crate::markdown::supported(&scope) {
                self.lsp.request_symbols(&scope);
            }
            return;
        }
        if !self.palette_wants(FILTER_SYMBOLS) {
            self.lsp.clear_workspace_symbols();
            return;
        }
        if self.palette.query.trim().chars().count() < 2 {
            return;
        }
        if self.palette.query_changed_at.elapsed() < SYMBOL_QUERY_DELAY {
            ui.ctx().request_repaint_after(SYMBOL_QUERY_DELAY);
            return;
        }
        let scoped_path = self.selected_project().map(|project| project.path.clone());
        if let Some(path) = scoped_path.as_deref() {
            self.lsp
                .request_scoped_symbols(&self.palette.query, Some(path));
        } else {
            self.lsp.clear_workspace_symbols();
        }
    }

    fn refresh_palette_cache(&mut self) {
        let project_id = self.selected_project().map(|project| project.id.clone());
        let stale = self.palette.cache.as_ref().is_none_or(|cache| {
            cache.query != self.palette.query
                || cache.state_revision != self.state.revision
                || cache.file_index_at != self.file_index_at
                || cache.lsp_revision != self.lsp.search_revision()
                || cache.filters != self.palette.filters
                || cache.project != project_id
                || cache.scope != self.palette.file_scope
        });
        if !stale {
            return;
        }
        self.palette.cache = Some(PaletteCache {
            query: self.palette.query.clone(),
            state_revision: self.state.revision,
            file_index_at: self.file_index_at,
            lsp_revision: self.lsp.search_revision(),
            filters: self.palette.filters,
            project: project_id,
            scope: self.palette.file_scope.clone(),
            items: self.filtered_palette(),
        });
    }

    /// Arrow keys move the selection, wrapping; returns whether it moved.
    fn palette_keyboard_nav(&mut self, ui: &egui::Ui, len: usize) -> bool {
        self.palette.index = self.palette.index.min(len.saturating_sub(1));
        let mut keyboard_moved = false;
        if ui.input(|input| input.key_pressed(egui::Key::ArrowDown))
            && let Some(next) = self
                .palette
                .index
                .checked_add(1)
                .and_then(|index| index.checked_rem(len))
        {
            self.palette.index = next;
            keyboard_moved = true;
        }
        if ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
            self.palette.index = self
                .palette
                .index
                .checked_sub(1)
                .unwrap_or_else(|| len.saturating_sub(1));
            keyboard_moved = true;
        }
        keyboard_moved
    }

    /// The result rows; returns the clicked index.
    fn palette_result_list(
        &self,
        ui: &mut egui::Ui,
        items: &[PaletteItem],
        keyboard_moved: bool,
    ) -> Option<usize> {
        let mut chosen = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, item) in items.iter().enumerate() {
                    let selected = index == self.palette.index;
                    let response = ui
                        .horizontal(|ui| {
                            let response = ui.selectable_label(selected, item.label());
                            let detail = item.detail(self);
                            if !detail.is_empty() {
                                ui.weak(egui::RichText::new(detail).small());
                            }
                            response
                        })
                        .inner;
                    if selected && keyboard_moved {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    #[cfg(feature = "test-support")]
                    diagnostics::record(
                        ui.ctx(),
                        &format!("palette-item:{}", item.label()),
                        response.rect,
                    );
                    if response.clicked() {
                        chosen = Some(index);
                    }
                }
            });
        chosen
    }

    fn close_palette(&mut self) {
        self.palette.open = false;
        self.palette.query.clear();
        self.palette.index = 0;
        self.palette.filters = 0;
        self.palette.file_scope = None;
        self.palette.cache = None;
        self.lsp.clear_workspace_symbols();
    }

    pub(super) fn run_palette(&mut self, ctx: &egui::Context, item: PaletteItem) {
        self.close_palette();
        match item {
            PaletteItem::Project(id, _) => self.select_project(id),
            PaletteItem::Session(id, _) => self.go_session(&id),
            PaletteItem::Agent(id, _, _) => self.go_session(&id),
            PaletteItem::File(path) => {
                if let Some(project) = self
                    .project_for_palette_path(&path)
                    .map(|project| project.id.clone())
                    && self.selected.as_ref() != Some(&project)
                {
                    self.select_project(project);
                }
                self.open_file(OpenFileInput {
                    path,
                    line: None,
                    split: None,
                    external: false,
                });
            }
            PaletteItem::Symbol {
                path, line, column, ..
            } => {
                if let Some(project) = self
                    .project_for_palette_path(&path)
                    .map(|project| project.id.clone())
                    && self.selected.as_ref() != Some(&project)
                {
                    self.select_project(project);
                }
                self.open_file_at(
                    path,
                    native_lsp::Position {
                        line,
                        character: column,
                    },
                );
            }
            PaletteItem::Settings(section) => {
                self.open_settings();
                self.request_settings_section(section);
            }
            PaletteItem::AddProject => self.add_project = true,
            PaletteItem::Action(action) => self.run_shortcut(ctx, action),
            PaletteItem::VncProfile(id, _) => {
                if let Some(profile) = self.state.vnc_profiles.iter().find(|p| p.id == id).cloned()
                {
                    self.connect_vnc_profile(&profile, None);
                }
            }
            PaletteItem::NewWorktree => self.open_worktree_wizard(),
            PaletteItem::OpenPlayer => self.open_player(),
            PaletteItem::ToggleIdeSidebarHeight => self.toggle_ide_sidebar_height(),
            PaletteItem::SaveLayout => self.save_layout_dialog(),
            PaletteItem::ApplyLayout(name) => {
                self.preferences.apply_layout(&name);
            }
        }
    }
}
