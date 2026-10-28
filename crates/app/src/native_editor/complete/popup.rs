//! The completion list under the caret: keys, rows, and clicks.
use super::*;

/// What the popup asked for this frame.
pub(crate) enum PopupAction {
    None,
    Close,
    Accept(usize),
}

/// Keys and clicks for a visible popup: Up/Down move, Enter/Tab accept,
/// Escape closes. Paints the list under the caret.
pub(crate) fn popup_ui(ui: &egui::Ui, popup: &mut CompletionPopup) -> PopupAction {
    if popup.visible.is_empty() {
        return PopupAction::None;
    }
    popup.selected = popup.selected.min(popup.visible.len().saturating_sub(1));
    let (mut action, moved) = popup_keys(ui, popup);
    let Some(anchor) = popup.anchor else {
        return action;
    };
    egui::Area::new(egui::Id::unique(("native-completion", &popup.path)))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor.left_bottom())
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(460.0);
                let max_height = VISIBLE_ROWS as f32 * 20.0;
                egui::ScrollArea::vertical()
                    .max_height(max_height)
                    .show(ui, |ui| {
                        if let Some(clicked) = popup_rows(ui, popup, moved) {
                            action = clicked;
                        }
                    });
            });
        });
    action
}

/// Up/Down move the selection, Enter/Tab accept, Escape closes. Returns
/// the action and whether the selection moved.
fn popup_keys(ui: &egui::Ui, popup: &mut CompletionPopup) -> (PopupAction, bool) {
    let mut action = PopupAction::None;
    let mut moved = false;
    ui.input_mut(|input| {
        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
            popup.selected = popup
                .selected
                .saturating_add(1)
                .checked_rem(popup.visible.len())
                .unwrap_or(0);
            moved = true;
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
            popup.selected = popup
                .selected
                .checked_sub(1)
                .unwrap_or(popup.visible.len().saturating_sub(1));
            moved = true;
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            || input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
        {
            action = PopupAction::Accept(popup.selected);
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            action = PopupAction::Close;
        }
    });
    (action, moved)
}

/// The visible items; returns an accept for a clicked row.
fn popup_rows(ui: &mut egui::Ui, popup: &CompletionPopup, moved: bool) -> Option<PopupAction> {
    let mut action = None;
    for (row, index) in popup.visible.iter().enumerate().take(200) {
        let Some(item) = popup.items.get(*index) else {
            continue;
        };
        let selected = row == popup.selected;
        let response = ui.horizontal(|ui| item_row(ui, item, selected)).inner;
        if selected && moved {
            response.scroll_to_me(None);
        }
        if response.clicked() {
            action = Some(PopupAction::Accept(row));
        }
    }
    action
}

/// Kind initial, label, and detail; returns the label's response.
fn item_row(
    ui: &mut egui::Ui,
    item: &native_lsp::CompletionItem,
    selected: bool,
) -> egui::Response {
    let kind = item
        .kind
        .as_deref()
        .and_then(|kind| kind.chars().next())
        .unwrap_or('•');
    ui.weak(egui::RichText::new(kind.to_string()).monospace());
    let label = ui.selectable_label(selected, egui::RichText::new(&item.label).monospace());
    if let Some(detail) = &item.detail {
        ui.add(egui::Label::new(egui::RichText::new(detail).weak().small()).truncate());
    }
    label
}
