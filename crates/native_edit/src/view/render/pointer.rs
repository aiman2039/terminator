//! Pointer input on the file area: caret placement, gutter clicks,
//! word/line picks, drag selection and keyboard focus.
use super::canvas::Canvas;
use super::{CHANGE_BAND, SourceView};
use crate::doc::Buffer;
use crate::vim::Effect;
use crate::vim::ModalEngine;
use egui::Sense;

impl<B: Buffer> SourceView<'_, B> {
    /// Register the stable keyboard target each frame. Otherwise egui
    /// drops it as a disappeared widget after the focus request.
    pub(super) fn register_focus_target(&self, canvas: &Canvas<'_>) {
        let ui = canvas.ui;
        let focus_id = self.geometry.focus_id;
        ui.ctx().data_mut(|data| {
            data.insert_temp(focus_id.with("native-source"), true);
        });
        let focus = ui.interact(
            canvas.response.rect,
            focus_id,
            Sense::focusable_noninteractive(),
        );
        focus.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::TextInput, ui.is_enabled(), "Source editor")
        });
    }

    /// Primary and secondary clicks: fold arrows, the change band,
    /// Alt+Shift caret toggles, then caret placement.
    pub(super) fn handle_clicks(&mut self, canvas: &Canvas<'_>) {
        let response = &canvas.response;
        let click_modifiers = canvas.ui.input(|input| input.modifiers);
        let band_click = self.band_click(canvas);
        let fold_click = self.fold_click(canvas);
        if let Some(line) = fold_click {
            self.out.fold_toggles.push(line);
        } else if let Some(click) = band_click {
            self.out.change_clicked = Some(click);
        } else if response.clicked()
            && !self.options.vim
            && click_modifiers.alt
            && click_modifiers.shift
            && let Some((line, col)) = response
                .interact_pointer_pos()
                .and_then(|pos| canvas.cursor_at(pos))
        {
            // Alt+Shift+click: add or remove a caret (`IntelliJ`).
            canvas.request_focus();
            self.engine.toggle_caret_at(self.doc, line, col);
        } else if response.clicked()
            || (response.secondary_clicked() && self.engine.selection().is_none())
        {
            self.place_clicked_caret(canvas, click_modifiers);
        }
    }

    /// A click in the change band opens that change instead of moving
    /// the caret.
    fn band_click(&self, canvas: &Canvas<'_>) -> Option<(usize, egui::Pos2)> {
        let left = canvas.response.rect.left();
        canvas
            .click_pos()
            .filter(|pos| self.options.show_line_numbers && pos.x < left + CHANGE_BAND)
            .and_then(|pos| {
                let (line, _) = canvas.cursor_at(pos)?;
                self.options
                    .change_marks
                    .iter()
                    .position(|mark| {
                        (mark.start..mark.end.max(mark.start.saturating_add(1))).contains(&line)
                    })
                    .map(|index| (index, pos))
            })
    }

    /// A click on a fold arrow (the gutter's last cell) toggles it.
    fn fold_click(&self, canvas: &Canvas<'_>) -> Option<usize> {
        let left = canvas.response.rect.left();
        let gutter = self.geometry.gutter;
        let advance = self.geometry.advance;
        canvas
            .click_pos()
            .filter(|pos| {
                self.options.show_line_numbers
                    && pos.x >= left + gutter - advance
                    && pos.x < left + gutter
            })
            .and_then(|pos| canvas.cursor_at(pos))
            .map(|(line, _)| line)
            .filter(|line| canvas.folds.region_at(*line).is_some())
    }

    fn place_clicked_caret(&mut self, canvas: &Canvas<'_>, click_modifiers: egui::Modifiers) {
        let response = &canvas.response;
        canvas.request_focus();
        if response.clicked() {
            self.engine.clear_extra_carets();
        }
        let extend = response.clicked() && click_modifiers.shift;
        // A plain click after a drag collapses that selection instead of
        // extending it the way a `v` selection does; modeless clicks
        // always collapse (IDE carets never extend without Shift).
        let mouse_drag_key = self.geometry.mouse_drag_key;
        let dragged = canvas
            .ui
            .ctx()
            .data_mut(|data| data.remove_temp::<bool>(mouse_drag_key).unwrap_or(false));
        if !extend && (dragged || !self.options.vim) {
            self.engine.clear_mouse_selection(self.doc);
        }
        let Some((line, col)) = response
            .interact_pointer_pos()
            .and_then(|pos| canvas.cursor_at(pos))
        else {
            return;
        };
        if extend {
            self.engine.extend_selection_to(self.doc, line, col);
            return;
        }
        self.out
            .effects
            .extend(self.engine.place_cursor(self.doc, line, col));
        // Cmd/Ctrl+click: go to the declaration under the pointer
        // (`IntelliJ`), now that the caret sits on it.
        if response.clicked() && click_modifiers.command {
            self.out.effects.push(Effect::GotoDefinition);
        }
    }

    /// Double click selects a word, triple click a line.
    pub(super) fn handle_multi_clicks(&mut self, canvas: &Canvas<'_>) {
        let response = &canvas.response;
        if response.double_clicked()
            && let Some((line, col)) = response
                .interact_pointer_pos()
                .and_then(|pos| canvas.cursor_at(pos))
        {
            self.engine.select_word_at(self.doc, line, col);
            canvas.request_focus();
        }
        if response.triple_clicked()
            && let Some((line, _)) = response
                .interact_pointer_pos()
                .and_then(|pos| canvas.cursor_at(pos))
        {
            self.engine.select_line_at(self.doc, line);
        }
    }

    pub(super) fn handle_drag(&mut self, canvas: &Canvas<'_>) {
        let ui = canvas.ui;
        if canvas.response.drag_started() {
            // The press point, not the position where egui decided this was
            // a drag: the anchor must be where the button went down.
            let anchor = ui.input(|input| input.pointer.press_origin());
            if let Some((line, col)) = anchor.and_then(|pos| canvas.cursor_at(pos)) {
                self.engine.begin_mouse_selection(self.doc, line, col);
                let mouse_drag_key = self.geometry.mouse_drag_key;
                ui.ctx()
                    .data_mut(|data| data.insert_temp(mouse_drag_key, true));
                canvas.request_focus();
            }
        } else if canvas.response.dragged()
            && let Some(pos) = ui.input(|input| input.pointer.interact_pos())
            && let Some((line, col)) = canvas.cursor_at(pos)
        {
            self.engine.extend_mouse_selection(self.doc, line, col);
        }
    }

    /// Autofocus, the focus state keys depend on, and the focus lock that
    /// keeps Tab, arrows and Escape in the file.
    pub(super) fn update_focus(&self, canvas: &Canvas<'_>) -> bool {
        let ui = canvas.ui;
        let focus_id = self.geometry.focus_id;
        if self.options.autofocus && ui.memory(|memory| memory.focused().is_none()) {
            canvas.request_focus();
        }
        let focused = ui.memory(|memory| memory.has_focus(focus_id))
            && !ui
                .input(|input| input.pointer.any_pressed() && !canvas.response.contains_pointer());
        if focused {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    focus_id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                );
            });
        }
        focused
    }
}
