//! Search Everywhere: a popup that floats above the panes, like the
//! JetBrains one. Tabs pick one kind, the field searches, the list shows
//! ranked rows, and the footer shows the selected row's path. Esc or a
//! click outside the popup closes it.
use super::popup::{
    FOOTER_HEIGHT, PaletteColors, PaletteRowInput, ROW_HEIGHT, palette_footer, palette_rect,
    palette_row, pressed_outside,
};
use super::*;
use std::sync::Arc;

impl App {
    /// Paints the Search Everywhere popup while the palette is open.
    pub(crate) fn show_palette(&mut self, ctx: &egui::Context) {
        if !self.palette.open {
            return;
        }
        let first_frame = !std::mem::replace(&mut self.palette.shown, true);
        let colors = PaletteColors::new(&self.theme);
        let rect = palette_rect(ctx.content_rect());
        let shadow = ctx.global_style().visuals.popup_shadow;
        let area = egui::Area::new(egui::Id::unique("search-everywhere"))
            .order(egui::Order::Foreground)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.surface)
                    .stroke(egui::Stroke::new(1.0, colors.border))
                    .corner_radius(appearance::PANEL_RADIUS)
                    .shadow(shadow)
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        let inner = rect.shrink(8.0).size();
                        ui.set_min_size(inner);
                        ui.set_max_size(inner);
                        self.palette_body(ui, colors);
                    });
            });
        if !first_frame && self.palette.open && pressed_outside(ctx, area.response.rect) {
            self.close_palette();
        }
    }

    fn palette_body(&mut self, ui: &mut egui::Ui, colors: PaletteColors) {
        // Adopt finished background scans while the palette is open.
        self.poll_file_index();
        self.lsp.poll();
        self.lsp.request_open_document_symbols();
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.close_palette();
            return;
        }
        let project_name = self
            .selected_project()
            .map(|project| project.name.clone())
            .unwrap_or_default();
        self.palette_tabs(ui, colors);
        ui.add_space(6.0);
        self.palette_search_field(ui, &project_name, colors);
        ui.add_space(6.0);
        self.palette_request_symbols(ui);
        if self.lsp.workspace_symbols_pending() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if let Some(wait) = self.refresh_palette_cache() {
            ui.ctx().request_repaint_after(wait);
        }
        let (items, details) = self.palette.cache.as_ref().map_or_else(
            || (Arc::from([]), Arc::from([])),
            |cache| (Arc::clone(&cache.items), Arc::clone(&cache.details)),
        );
        let list = egui::vec2(
            ui.available_width(),
            (ui.available_height() - FOOTER_HEIGHT).max(ROW_HEIGHT),
        );
        let chosen = ui
            .allocate_ui(list, |ui| {
                ui.set_min_size(list);
                if items.is_empty() {
                    ui.label(
                        egui::RichText::new(if self.lsp.workspace_symbols_pending() {
                            "Searching symbols…"
                        } else {
                            "No matches."
                        })
                        .color(colors.secondary),
                    );
                    return None;
                }
                let keyboard_moved = self.palette_keyboard_nav(ui, items.len());
                let mut chosen = self.palette_result_list(PaletteRowsInput {
                    ui,
                    items: &items,
                    details: &details,
                    keyboard_moved,
                    colors,
                });
                if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    chosen = Some(self.palette.index);
                }
                chosen
            })
            .inner;
        palette_footer(ui, details.get(self.palette.index), colors);
        if let Some(index) = chosen
            && let Some(item) = items.get(index).cloned()
        {
            self.run_palette(ui.ctx(), item);
        }
    }

    /// One tab per kind plus All; the selected tab fills with the accent.
    fn palette_tabs(&mut self, ui: &mut egui::Ui, colors: PaletteColors) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let tabs = std::iter::once(("All", 0)).chain(PALETTE_FILTERS.iter().copied());
            for (name, bit) in tabs {
                let on = self.palette.filters == bit;
                let (fill, stroke) = if on {
                    (
                        colors.accent.gamma_multiply(0.3),
                        egui::Stroke::new(1.0, colors.accent),
                    )
                } else {
                    (egui::Color32::TRANSPARENT, egui::Stroke::NONE)
                };
                let response = ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(name)
                                .size(appearance::FONT_BODY)
                                .color(colors.text),
                        )
                        .fill(fill)
                        .stroke(stroke)
                        .corner_radius(egui::CornerRadius::same(appearance::WIDGET_RADIUS))
                        .min_size(egui::vec2(0.0, ROW_HEIGHT)),
                    )
                    .on_hover_text(format!("Show {name}"));
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), &format!("palette-filter:{name}"), response.rect);
                if response.clicked() {
                    self.select_palette_tab(bit);
                }
            }
        });
    }

    fn select_palette_tab(&mut self, bit: u8) {
        // A tab leaves File Structure for the full search.
        self.palette.file_scope = None;
        self.palette.filters = bit;
        self.palette.index = 0;
        self.palette.focus_pending = true;
    }

    fn palette_search_field(
        &mut self,
        ui: &mut egui::Ui,
        project_name: &str,
        colors: PaletteColors,
    ) {
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
        let id = egui::Id::unique("palette-search-field");
        let focused = ui.memory(|memory| memory.has_focus(id));
        let border = if focused {
            egui::Stroke::new(1.5, colors.accent)
        } else {
            egui::Stroke::new(1.0, colors.border)
        };
        let search = egui::Frame::new()
            .fill(colors.field)
            .stroke(border)
            .corner_radius(egui::CornerRadius::same(appearance::WIDGET_RADIUS))
            .inner_margin(egui::Margin::symmetric(8, 5))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Image::new(crate::icons::source("Search"))
                            .tint(colors.secondary)
                            .fit_to_exact_size(egui::Vec2::splat(appearance::TOOLBAR_ICON)),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.palette.query)
                            .id(id)
                            .frame(egui::Frame::NONE)
                            .font(egui::FontId::proportional(appearance::FONT_LARGE))
                            .hint_text(hint)
                            .desired_width(f32::INFINITY),
                    )
                })
                .inner
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "palette-search", search.rect);
        if search.changed() {
            self.palette.index = 0;
            self.palette.query_changed_at = Instant::now();
            self.lsp.clear_workspace_symbols();
        }
        // Focus once on open or after a tab. A pressed pointer is choosing a
        // tab; focus returns on release so that click is not taken.
        if self.palette.focus_pending && !ui.input(|input| input.pointer.primary_down()) {
            search.request_focus();
            self.palette.focus_pending = false;
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

    /// Rebuilds the ranked rows when the palette input changed. A daemon
    /// state change alone rebuilds at most once per [`STATE_REFRESH`], so
    /// frequent snapshots do not re-rank on every frame (#177). Returns how
    /// long to wait before a deferred rebuild is due.
    pub(super) fn refresh_palette_cache(&mut self) -> Option<Duration> {
        let project_id = self.selected_project().map(|project| project.id.clone());
        let input_changed = self.palette.cache.as_ref().is_none_or(|cache| {
            cache.query != self.palette.query
                || cache.file_index_at != self.file_index_at
                || cache.lsp_revision != self.lsp.search_revision()
                || cache.filters != self.palette.filters
                || cache.project != project_id
                || cache.scope != self.palette.file_scope
        });
        if !input_changed {
            let cache = self.palette.cache.as_ref()?;
            if cache.state_revision == self.state.revision {
                return None;
            }
            let age = cache.built_at.elapsed();
            if age < STATE_REFRESH {
                return STATE_REFRESH.checked_sub(age);
            }
        }
        let items: std::sync::Arc<[PaletteItem]> = self.filtered_palette().into();
        let details = items.iter().map(|item| item.row_text(self)).collect();
        self.palette.cache = Some(PaletteCache {
            query: self.palette.query.clone(),
            state_revision: self.state.revision,
            file_index_at: self.file_index_at,
            lsp_revision: self.lsp.search_revision(),
            filters: self.palette.filters,
            project: project_id,
            scope: self.palette.file_scope.clone(),
            built_at: Instant::now(),
            items,
            details,
        });
        None
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

    /// The result rows, laid out only while visible; returns the clicked index.
    fn palette_result_list(&self, input: PaletteRowsInput<'_>) -> Option<usize> {
        let PaletteRowsInput {
            ui,
            items,
            details,
            keyboard_moved,
            colors,
        } = input;
        let mut chosen = None;
        let stride = ROW_HEIGHT + ui.spacing().item_spacing.y;
        let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
        if keyboard_moved {
            area = area.vertical_scroll_offset(palette_scroll_offset(
                self.palette.index,
                stride,
                ui.available_height(),
            ));
        }
        area.show_rows(ui, ROW_HEIGHT, items.len(), |ui, range| {
            for (index, item) in items.iter().enumerate().take(range.end).skip(range.start) {
                let response = palette_row(PaletteRowInput {
                    ui,
                    item,
                    text: details.get(index),
                    selected: index == self.palette.index,
                    colors,
                });
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

    pub(crate) fn close_palette(&mut self) {
        self.palette.open = false;
        self.palette.focus_pending = false;
        self.palette.query.clear();
        self.palette.index = 0;
        self.palette.filters = 0;
        self.palette.file_scope = None;
        self.palette.cache = None;
        self.lsp.clear_workspace_symbols();
    }

    pub(super) fn run_palette(&mut self, ctx: &egui::Context, item: PaletteItem) {
        // Read a typed `:line[:col]` before closing clears the query.
        let typed = results::file_query(self.palette.query.trim());
        self.close_palette();
        match item {
            PaletteItem::Project(id, _) => self.select_project(id),
            PaletteItem::Session(id, _) => self.go_session(&id),
            PaletteItem::Agent(id, _, _) => self.go_session(&id),
            PaletteItem::File(path) => self.open_palette_file(path, &typed),
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
            PaletteItem::Editor(command) => self.run_editor_command(ctx, command),
            PaletteItem::VncProfile(id, _) => {
                if let Some(profile) = self.state.vnc_profiles.iter().find(|p| p.id == id).cloned()
                {
                    self.connect_vnc_profile(&profile, None);
                }
            }
            PaletteItem::NewWorktree => self.open_worktree_wizard(),
            PaletteItem::OpenPlayer => self.open_player(),
            PaletteItem::CheckForUpdates => self.check_for_updates(),
            PaletteItem::ToggleIdeSidebarHeight => self.toggle_ide_sidebar_height(),
            PaletteItem::SaveLayout => self.save_layout_dialog(),
            PaletteItem::ApplyLayout(name) => {
                if let Some(preset) = self.preferences.apply_layout(&name) {
                    self.pending_left_width =
                        Some(crate::preferences::clamp_left_width(preset.left_width));
                }
            }
        }
    }

    /// A Go to File pick opens in its own project, at the typed 1-based
    /// line and column when the query had them (`main.rs:42:5`).
    fn open_palette_file(&mut self, path: PathBuf, typed: &results::FileQuery) {
        if let Some(project) = self
            .project_for_palette_path(&path)
            .map(|project| project.id.clone())
            && self.selected.as_ref() != Some(&project)
        {
            self.select_project(project);
        }
        let Some(line) = typed.line else {
            self.open_file(OpenFileInput {
                path,
                line: None,
                split: None,
                external: false,
            });
            return;
        };
        let at = native_lsp::Position {
            line: line.saturating_sub(1),
            character: typed.column.unwrap_or(1).saturating_sub(1),
        };
        self.open_file_at(path, at);
    }
}

struct PaletteRowsInput<'a> {
    ui: &'a mut egui::Ui,
    items: &'a [PaletteItem],
    details: &'a [RowText],
    keyboard_moved: bool,
    colors: PaletteColors,
}
/// Scroll offset that centers row `index` in a viewport of `viewport` height.
pub(super) fn palette_scroll_offset(index: usize, row_stride: f32, viewport: f32) -> f32 {
    let top = f32::from(u16::try_from(index).unwrap_or(u16::MAX)) * row_stride;
    (top - (viewport - row_stride) / 2.0).max(0.0)
}
