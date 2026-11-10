//! Paints a `Tab::Document`: a header row like the image tab, then the body.
use super::{DocumentKind, TableState, table};
use crate::app::OpenFileModeInput;
use crate::{App, Job, appearance, services};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use std::{path::Path, time::Duration};

/// Width of one table column before the user resizes it.
const COLUMN_WIDTH: f32 = 140.0;

impl App {
    pub(crate) fn document_view(&mut self, ui: &mut egui::Ui, path: &Path, kind: DocumentKind) {
        match kind {
            DocumentKind::Table => self.table_view(ui, path),
        }
    }

    fn table_view(&mut self, ui: &mut egui::Ui, path: &Path) {
        self.tables.mark_visible(path);
        let mut as_text = false;
        let mut reload = false;
        appearance::wrapping_path_row(ui, &services::compact_path(path), |ui| {
            reload = appearance::sidebar_action(ui, "RefreshCw", "Reload table").clicked();
            as_text = appearance::sidebar_action(ui, "FileText", "Open as text").clicked();
            if appearance::sidebar_action(ui, "ExternalLink", "Open externally").clicked() {
                let _ = self.jobs.send(Job::External(path.into()));
            }
        });
        if as_text {
            self.open_file_mode(OpenFileModeInput {
                path: path.into(),
                line: None,
                split: None,
                external: false,
                text: true,
            });
        }
        let Some(state) = self.tables.state(&self.services, path, reload) else {
            ui.weak("Close another table to load this file.");
            return;
        };
        if !state.loading && state.table.is_none() && state.error.is_none() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        show_state(ui, state, path);
    }
}

fn show_state(ui: &mut egui::Ui, state: &mut TableState, path: &Path) {
    if let Some(error) = &state.error {
        let text = if state.table.is_some() {
            format!("Showing previous table: {error}")
        } else {
            error.clone()
        };
        ui.colored_label(ui.visuals().error_fg_color, text);
    }
    let Some(table) = state.table.clone() else {
        if state.error.is_none() {
            ui.spinner();
        }
        return;
    };
    ui.horizontal(|ui| {
        ui.add(
            appearance::singleline(&mut state.filter)
                .hint_text("Filter rows")
                .desired_width(220.0),
        );
        let shown = state.order().len();
        let mut note = if shown == table.rows.len() {
            format!("{} rows × {} columns", table.rows.len(), table.columns())
        } else {
            format!("{shown} of {} rows", table.rows.len())
        };
        if table.truncated {
            note.push_str(&format!(
                " · showing the first {} rows; open as text for the rest",
                table.rows.len()
            ));
        }
        ui.weak(note);
    });
    paint_table(ui, state, &table, path);
}

fn paint_table(ui: &mut egui::Ui, state: &mut TableState, table: &table::Table, path: &Path) {
    let columns = table.columns();
    if columns == 0 {
        ui.weak("The file has no rows.");
        return;
    }
    let row_height = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    let sort = state.sort;
    let mut clicked = None;
    let order = state.order();
    egui::ScrollArea::horizontal()
        .id_salt(("table-scroll", path))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            TableBuilder::new(ui)
                .id_salt(("table", path))
                .striped(true)
                .resizable(true)
                .auto_shrink([false, false])
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .columns(Column::initial(COLUMN_WIDTH).clip(true), columns)
                .header(row_height, |mut header| {
                    for column in 0..columns {
                        header.col(|ui| {
                            let name = table.headers.get(column).map_or("", String::as_str);
                            let arrow = match sort {
                                Some(sort) if sort.column == column && sort.ascending => " ▲",
                                Some(sort) if sort.column == column => " ▼",
                                _ => "",
                            };
                            let label = egui::RichText::new(format!("{name}{arrow}")).strong();
                            if ui
                                .add(egui::Label::new(label).sense(egui::Sense::click()))
                                .on_hover_text("Sort by this column")
                                .clicked()
                            {
                                clicked = Some(column);
                            }
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_height, order.len(), |mut row| {
                        let cells = order
                            .get(row.index())
                            .and_then(|index| table.rows.get(*index));
                        for column in 0..columns {
                            row.col(|ui| {
                                let text = cells
                                    .and_then(|cells| cells.get(column))
                                    .map_or("", String::as_str);
                                ui.add(egui::Label::new(text).truncate());
                            });
                        }
                    });
                });
        });
    if let Some(column) = clicked {
        state.toggle_sort(column);
    }
}
