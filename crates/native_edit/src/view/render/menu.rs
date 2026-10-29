//! The editor context menu: clipboard, history, find, blame, file and
//! language-server actions.
use super::SourceView;
use super::canvas::Canvas;
use super::helpers::RenderHelpers;
use crate::doc::Buffer;
use crate::vim::Effect;
use crate::vim::ModalEngine;

/// One context-menu row: label, icon name and shortcut hint.
struct MenuRow<'s> {
    label: &'s str,
    icon: &'s str,
    shortcut: &'s str,
}

impl<B: Buffer> SourceView<'_, B> {
    pub(super) fn show_context_menu(&mut self, canvas: &Canvas<'_>) {
        let mut popup = egui::Popup::context_menu(&canvas.response);
        if let Some(style) = self.options.menu_style {
            popup = popup.style(style);
        }
        popup.show(|ui| self.context_menu_items(ui));
        self.out.context_menu_opened = canvas.response.context_menu_opened();
    }

    fn context_menu_items(&mut self, ui: &mut egui::Ui) {
        if let Some(header) = self.options.menu_header {
            header(ui, self.id, self.id);
        }
        let (key, shift) = RenderHelpers::menu_modifiers();
        self.menu_clipboard_items(ui, key);
        ui.separator();
        self.menu_history_items(ui, key, shift);
        ui.separator();
        self.menu_find_items(ui, key);
        self.menu_blame_item(ui);
        ui.separator();
        self.menu_file_items(ui, key);
        self.menu_lsp_items(ui);
    }

    fn item(&self, ui: &mut egui::Ui, row: MenuRow<'_>) -> egui::Response {
        let MenuRow {
            label,
            icon,
            shortcut,
        } = row;
        match self.options.menu_item {
            Some(draw) => draw(ui, label, icon, shortcut),
            None => ui.button(label),
        }
    }

    fn enabled_item(&self, ui: &mut egui::Ui, enabled: bool, row: MenuRow<'_>) -> egui::Response {
        ui.add_enabled_ui(enabled, |ui| self.item(ui, row)).inner
    }

    /// Queue `effect` and close the menu when `response` was clicked.
    fn effect_on_click(&mut self, ui: &egui::Ui, response: &egui::Response, effect: Effect) {
        if response.clicked() {
            self.out.effects.push(effect);
            ui.close();
        }
    }

    fn menu_clipboard_items(&mut self, ui: &mut egui::Ui, key: &str) {
        let copy = MenuRow {
            label: "Copy",
            icon: "Copy",
            shortcut: &format!("{key}C"),
        };
        if self.item(ui, copy).clicked() {
            let text = self.engine.copy_text(self.doc);
            if !text.is_empty() {
                ui.ctx().copy_text(text);
            }
            ui.close();
        }
        self.menu_copy_reference(ui);
        let cut = MenuRow {
            label: "Cut",
            icon: "Eraser",
            shortcut: &format!("{key}X"),
        };
        let can_cut = !self.options.read_only && self.engine.selection().is_some();
        if self.enabled_item(ui, can_cut, cut).clicked() {
            let text = self.engine.cut(self.doc);
            if !text.is_empty() {
                ui.ctx().copy_text(text);
            }
            ui.close();
        }
        let paste = MenuRow {
            label: "Paste",
            icon: "ClipboardPaste",
            shortcut: &format!("{key}V"),
        };
        if self
            .enabled_item(ui, !self.options.read_only, paste)
            .clicked()
        {
            let focus_id = self.geometry.focus_id;
            ui.memory_mut(|memory| memory.request_focus(focus_id));
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
            ui.close();
        }
        let select_all = MenuRow {
            label: "Select all",
            icon: "TextSelect",
            shortcut: &format!("{key}A"),
        };
        if self.item(ui, select_all).clicked() {
            self.engine.select_all(self.doc);
            ui.close();
        }
    }

    fn menu_copy_reference(&self, ui: &mut egui::Ui) {
        let Some(reference) = &self.options.reference else {
            return;
        };
        let row = MenuRow {
            label: "Copy reference",
            icon: "Copy",
            shortcut: if cfg!(target_os = "macos") {
                "⌥⇧⌘C"
            } else {
                "Ctrl+Alt+Shift+C"
            },
        };
        if self.item(ui, row).clicked() {
            ui.ctx()
                .copy_text(RenderHelpers::caret_reference(reference, self.engine));
            ui.close();
        }
    }

    fn menu_history_items(&mut self, ui: &mut egui::Ui, key: &str, shift: &str) {
        let editable = !self.options.read_only;
        let undo = MenuRow {
            label: "Undo",
            icon: "History",
            shortcut: &format!("{key}Z"),
        };
        if self.enabled_item(ui, editable, undo).clicked() {
            self.engine.undo(self.doc);
            ui.close();
        }
        let redo = MenuRow {
            label: "Redo",
            icon: "RefreshCw",
            shortcut: &format!("{shift}Z"),
        };
        if self.enabled_item(ui, editable, redo).clicked() {
            self.engine.redo(self.doc);
            ui.close();
        }
    }

    fn menu_find_items(&mut self, ui: &mut egui::Ui, key: &str) {
        let editable = !self.options.read_only;
        let find = MenuRow {
            label: "Find in file",
            icon: "Search",
            shortcut: &format!("{key}F"),
        };
        if self.item(ui, find).clicked() {
            self.engine.begin_search();
            ui.close();
        }
        let replace = MenuRow {
            label: "Replace in file",
            icon: "Pencil",
            shortcut: "",
        };
        if self.enabled_item(ui, editable, replace).clicked() {
            self.engine.begin_ex("%s/");
            ui.close();
        }
        let save = MenuRow {
            label: "Save",
            icon: "Save",
            shortcut: &format!("{key}S"),
        };
        let response = self.enabled_item(ui, editable, save);
        self.effect_on_click(ui, &response, Effect::Save);
    }

    fn menu_blame_item(&mut self, ui: &mut egui::Ui) {
        let Some(on) = self.options.blame else {
            return;
        };
        ui.separator();
        let label = if on {
            "Close annotations"
        } else {
            "Annotate with Git Blame"
        };
        let row = MenuRow {
            label,
            icon: "History",
            shortcut: "",
        };
        if self.item(ui, row).clicked() {
            self.out.toggle_blame = true;
            ui.close();
        }
    }

    fn menu_file_items(&mut self, ui: &mut egui::Ui, key: &str) {
        let copy_path = MenuRow {
            label: "Copy file path",
            icon: "Copy",
            shortcut: "",
        };
        if self.item(ui, copy_path).clicked() {
            ui.ctx().copy_text(self.id.to_owned());
            ui.close();
        }
        let open = MenuRow {
            label: "Open file…",
            icon: "File",
            shortcut: &format!("{key}O"),
        };
        let response = self.item(ui, open);
        self.effect_on_click(ui, &response, Effect::OpenFilePicker);
        let find = MenuRow {
            label: "Find in files…",
            icon: "Search",
            shortcut: if cfg!(target_os = "macos") {
                "⇧⌘F"
            } else {
                "Ctrl+Shift+F"
            },
        };
        let response = self.item(ui, find);
        self.effect_on_click(ui, &response, Effect::FindInFiles);
    }

    fn menu_lsp_items(&mut self, ui: &mut egui::Ui) {
        if crate::lsp::server_language(self.language).is_none() {
            return;
        }
        ui.separator();
        let call_keys = if cfg!(target_os = "macos") {
            "⌃⌥H"
        } else {
            "Ctrl+Alt+H"
        };
        let (implementation_keys, type_keys) = if cfg!(target_os = "macos") {
            ("⌥⌘B", "⇧⌘B")
        } else {
            ("Ctrl+Alt+B", "Ctrl+Shift+B")
        };
        let rows = [
            (
                "Go to definition",
                "ExternalLink",
                "",
                Effect::GotoDefinition,
            ),
            (
                "Go to implementation",
                "ExternalLink",
                implementation_keys,
                Effect::GotoImplementation,
            ),
            (
                "Go to type declaration",
                "ExternalLink",
                type_keys,
                Effect::GotoTypeDefinition,
            ),
            ("Find references", "ListTree", "", Effect::FindReferences),
            (
                "Call hierarchy",
                "ListTree",
                call_keys,
                Effect::Hierarchy { types: false },
            ),
            (
                "Type hierarchy",
                "ListTree",
                "",
                Effect::Hierarchy { types: true },
            ),
        ];
        for (label, icon, shortcut, effect) in rows {
            let row = MenuRow {
                label,
                icon,
                shortcut,
            };
            let response = self.item(ui, row);
            self.effect_on_click(ui, &response, effect);
        }
        let rename = MenuRow {
            label: "Rename symbol…",
            icon: "Pencil",
            shortcut: "",
        };
        let rename_button = self.enabled_item(ui, !self.options.read_only, rename);
        self.out.rename_action_rect = Some(rename_button.rect);
        self.effect_on_click(ui, &rename_button, Effect::RenameSymbol);
        let hover = MenuRow {
            label: "Show hover",
            icon: "Info",
            shortcut: "",
        };
        let hover_button = self.item(ui, hover);
        self.out.hover_action_rect = Some(hover_button.rect);
        self.effect_on_click(ui, &hover_button, Effect::Hover);
    }
}
