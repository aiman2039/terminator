//! The line-break and encoding cells of the editor's status row open
//! menus (`JetBrains` status bar). A pick converts the buffer; the next
//! save writes the new format.
use super::*;
use crate::native_editor::disk::{apply_edits, text_edits};
use crate::native_editor::file_io::{LineEnding, TextEncoding};
use encoding_rs::{UTF_8, UTF_16BE, UTF_16LE};
use terminator_native_edit::doc::normalize_breaks;

/// Status cells as [`ViewPass::status_items`] lists them.
const ENDING_CELL: usize = 0;
const ENCODING_CELL: usize = 1;

/// Encodings offered for conversion, with their menu labels.
fn encodings() -> [(&'static str, TextEncoding); 4] {
    let form = |encoding, bom| TextEncoding { encoding, bom };
    [
        ("UTF-8", form(UTF_8, false)),
        ("UTF-8 with BOM", form(UTF_8, true)),
        ("UTF-16 LE", form(UTF_16LE, true)),
        ("UTF-16 BE", form(UTF_16BE, true)),
    ]
}

impl ViewPass<'_> {
    /// Menus on the status cells at `cells` (one rect per status item).
    /// Read-only buffers keep plain cells.
    pub(super) fn status_menus(&mut self, ui: &egui::Ui, cells: &[egui::Rect]) {
        if self.doc.doc.is_none() || self.read_only() {
            return;
        }
        let id = egui::Id::unique(("native-status-menu", &self.frame.source_id));
        let ending = self.doc.line_ending;
        if let Some(cell) = cells.get(ENDING_CELL)
            && let Some(picked) = cell_menu(
                ui,
                CellMenu {
                    id: id.with("ending"),
                    cell: *cell,
                    tip: "Line breaks (saved with the next save)",
                },
                |ui| ending_items(ui, ending),
            )
        {
            self.convert_line_breaks(picked);
        }
        let encoding = self.doc.encoding;
        if let Some(cell) = cells.get(ENCODING_CELL)
            && let Some(picked) = cell_menu(
                ui,
                CellMenu {
                    id: id.with("encoding"),
                    cell: *cell,
                    tip: "Encoding (saved with the next save)",
                },
                |ui| encoding_items(ui, encoding),
            )
        {
            self.doc.encoding = picked;
        }
    }

    /// Make every break `\n` in the buffer (one undo step) and save them
    /// as `ending` (LF or CRLF).
    fn convert_line_breaks(&mut self, ending: LineEnding) {
        if let Some(buffer) = self.doc.doc.as_mut() {
            let text = buffer.text();
            if let std::borrow::Cow::Owned(normalized) = normalize_breaks(&text) {
                apply_edits(buffer, &text_edits(&text, &normalized));
            }
        }
        self.doc.line_ending = ending;
    }
}

/// One clickable status cell.
struct CellMenu<'a> {
    id: egui::Id,
    cell: egui::Rect,
    tip: &'a str,
}

/// Make `cell` a button with a hover highlight; its click toggles a menu
/// filled by `items`. Returns the menu's pick.
fn cell_menu<R>(
    ui: &egui::Ui,
    menu: CellMenu<'_>,
    items: impl FnOnce(&mut egui::Ui) -> Option<R>,
) -> Option<R> {
    let CellMenu { id, cell, tip } = menu;
    let rect = cell.expand2(egui::vec2(4.0, 0.0));
    let response = ui
        .interact(rect, id, egui::Sense::click())
        .on_hover_text(tip);
    if response.hovered() || response.is_pointer_button_down_on() {
        // The editor painted the cell text: a light veil marks the hover.
        let veil = ui
            .visuals()
            .widgets
            .hovered
            .weak_bg_fill
            .gamma_multiply(0.45);
        ui.painter().rect_filled(
            rect.shrink2(egui::vec2(0.0, 2.0)),
            appearance::WIDGET_RADIUS,
            veil,
        );
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    #[cfg(feature = "test-support")]
    crate::diagnostics::record(ui.ctx(), &format!("native-status-cell:{tip}"), rect);
    egui::Popup::menu(&response)
        .style(appearance::menu_style)
        .show(items)
        .and_then(|shown| shown.inner)
}

/// LF and CRLF, the current one checked. Returns the pick.
fn ending_items(ui: &mut egui::Ui, current: LineEnding) -> Option<LineEnding> {
    let choices = [
        ("LF line breaks", LineEnding::Lf),
        ("CRLF line breaks", LineEnding::Crlf),
    ];
    choice_items(ui, ("Rows2", current), &choices)
}

/// The offered encodings, the current one checked. Returns the pick.
fn encoding_items(ui: &mut egui::Ui, current: TextEncoding) -> Option<TextEncoding> {
    choice_items(ui, ("FileType", current), &encodings())
}

/// One menu row per choice, `✓` beside the current one. Returns the pick.
fn choice_items<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    (icon, current): (&str, T),
    choices: &[(&str, T)],
) -> Option<T> {
    let mut picked = None;
    for &(label, choice) in choices {
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label,
            icon,
            shortcut: if choice == current { "✓" } else { "" },
        })
        .clicked()
        {
            picked = Some(choice);
            ui.close();
        }
    }
    picked
}
