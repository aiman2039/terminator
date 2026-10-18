//! Problems and Structure sidebars: workspace-wide language-server
//! diagnostics and the active editor's document outline. Both jump
//! through the ordinary file-open path, so focus, splits, and the
//! pending-line handoff behave exactly like an Explorer open.

#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::{App, Tab, appearance};
use eframe::egui::{self, RichText};
use std::path::PathBuf;
use terminator_native_edit::lsp::Severity;

/// Panel title with the manual re-index action on the right. Returns
/// true when the button was clicked.
fn lsp_panel_header(ui: &mut egui::Ui, title: &str, tip: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.strong(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let button = appearance::sidebar_action(ui, "RefreshCw", tip);
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), &format!("lsp-restart:{title}"), button.rect);
            clicked = button.clicked();
        });
    });
    ui.separator();
    clicked
}

impl App {
    /// Path of the native editor in the selected project's active pane.
    fn active_editor_path(&self) -> Option<PathBuf> {
        let project_id = self.selected.clone()?;
        match self
            .layouts
            .get(&project_id)
            .and_then(|workspace| workspace.active_pane().cloned())
        {
            Some(Tab::NativeEditor { path }) => Some(path),
            _ => None,
        }
    }

    pub(crate) fn problems_panel(&mut self, ui: &mut egui::Ui) {
        if lsp_panel_header(
            ui,
            "Problems",
            "Restart language servers and re-index (also retries a missing server)",
        ) {
            self.lsp.restart_all();
        }
        let files = self.lsp.all_problems();
        let total: usize = files.iter().map(|(_, items)| items.len()).sum();
        if files.is_empty() {
            ui.weak("No problems reported by language servers.");
            return;
        }
        ui.weak(format!(
            "{total} problem{} in {} file{}",
            if total == 1 { "" } else { "s" },
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        ));
        ui.separator();
        let failed = appearance::color(&self.theme.status_failed);
        let waiting = appearance::color(&self.theme.status_waiting);
        let mut jump: Option<(PathBuf, u32)> = None;
        appearance::sidebar_scroll("problems")
            .max_height(ui.available_height())
            .show(ui, |ui| {
                for (path, items) in &files {
                    let name = path.file_name().map_or_else(
                        || path.display().to_string(),
                        |n| n.to_string_lossy().into_owned(),
                    );
                    ui.horizontal(|ui| {
                        ui.strong(name);
                        ui.weak(format!("({})", items.len()));
                    });
                    ui.weak(path.display().to_string());
                    for item in items {
                        let (tag, color) = match item.severity {
                            Some(Severity::Error) => ("E", Some(failed)),
                            Some(Severity::Warning) => ("W", Some(waiting)),
                            _ => ("·", None),
                        };
                        let line = item.range.start.line.saturating_add(1);
                        let label = format!(
                            "{tag} {line}: {}",
                            item.message.lines().next().unwrap_or("")
                        );
                        let response = match color {
                            Some(color) => ui.colored_label(color, label),
                            None => ui.weak(label),
                        };
                        if response
                            .interact(egui::Sense::click())
                            .on_hover_text("Open file at this problem")
                            .clicked()
                        {
                            jump = Some((path.clone(), line));
                        }
                    }
                    ui.add_space(6.0);
                }
            });
        if let Some((path, line)) = jump {
            self.open_file(path, Some(line), None, false);
        }
    }

    #[allow(clippy::cast_precision_loss)] // outline depth is small
    pub(crate) fn structure_panel(&mut self, ui: &mut egui::Ui) {
        let restart = lsp_panel_header(
            ui,
            "Structure",
            "Restart the language server and re-request symbols",
        );
        let Some(path) = self.active_editor_path() else {
            ui.weak("Open a file in the native editor to see its outline.");
            return;
        };
        ui.add(
            egui::Label::new(
                RichText::new(path.file_name().unwrap_or_default().to_string_lossy()).strong(),
            )
            .truncate(),
        )
        .on_hover_text(path.display().to_string());
        let language =
            terminator_native_edit::highlight::language_for_path(&path.to_string_lossy());
        if terminator_native_edit::lsp::server_language(language).is_none() {
            ui.weak("No document outline for this file type.");
            ui.weak("Structure supports Rust, Python, JavaScript and TypeScript through a language server.");
            return;
        }
        if restart {
            if self.lsp.tracked(&path) {
                self.lsp.restart(&path);
            } else {
                self.lsp.restart_all();
            }
        }
        if !self.lsp.tracked(&path) {
            ui.weak("Waiting for the native editor to load this file.");
            return;
        }
        if let Some(note) = self.lsp.server_note(&path) {
            ui.weak(note);
            return;
        }
        self.lsp.request_symbols(&path);
        let symbols = self.lsp.symbols_for(&path);
        if symbols.is_empty() {
            ui.weak(
                "No symbols yet — the server may still be indexing (it retries every few seconds).",
            );
            return;
        }
        let mut jump: Option<u32> = None;
        ui.weak(format!("{} symbols", symbols.len()));
        appearance::sidebar_scroll("structure")
            .max_height(ui.available_height())
            .show(ui, |ui| {
                for symbol in &symbols {
                    ui.horizontal(|ui| {
                        ui.add_space((symbol.depth as f32) * 14.0);
                        let label = RichText::new(&symbol.name);
                        let response = ui.selectable_label(false, label).on_hover_text(format!(
                            "{} — line {}",
                            symbol.kind,
                            symbol.range.start.line.saturating_add(1)
                        ));
                        if response.clicked() {
                            jump = Some(symbol.range.start.line.saturating_add(1));
                        }
                    });
                }
            });
        if let Some(line) = jump {
            self.open_file(path, Some(line), None, false);
        }
    }
}
