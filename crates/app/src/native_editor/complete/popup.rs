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
            appearance::popup_text_style(ui.style_mut());
            appearance::popup_frame(ui.style()).show(ui, |ui| {
                ui.set_width(POPUP_WIDTH);
                ui.spacing_mut().item_spacing.y = 0.0;
                let max_height = VISIBLE_ROWS as f32 * ROW_HEIGHT;
                egui::ScrollArea::vertical()
                    .max_height(max_height)
                    .show(ui, |ui| {
                        if let Some(clicked) = popup_rows(ui, popup, moved) {
                            action = clicked;
                        }
                    });
                popup_footer(ui);
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

/// Popup width and row height, as in the `IntelliJ` completion list.
const POPUP_WIDTH: f32 = 520.0;
const ROW_HEIGHT: f32 = 22.0;

/// The visible items; returns an accept for a clicked row.
fn popup_rows(ui: &mut egui::Ui, popup: &CompletionPopup, moved: bool) -> Option<PopupAction> {
    let mut action = None;
    for (row, index) in popup.visible.iter().enumerate().take(200) {
        let Some(item) = popup.items.get(*index) else {
            continue;
        };
        let selected = row == popup.selected;
        let response = item_row(ui, item, &popup.prefix, selected);
        if selected && moved {
            response.scroll_to_me(None);
        }
        if response.clicked() {
            action = Some(PopupAction::Accept(row));
        }
    }
    action
}

/// Kind badge letter and tint, shared with the Structure tool window.
/// Keywords get no badge, as in the `IntelliJ` lookup.
pub(crate) fn kind_badge(kind: Option<&str>) -> Option<(char, egui::Color32)> {
    match kind? {
        "Keyword" => None,
        kind => Some(crate::sidebar_ui::structure::kind_badge(kind)),
    }
}

/// Kind badge, label with the typed prefix in the match color, and the
/// detail right-aligned in muted text; returns the row's response.
fn item_row(
    ui: &mut egui::Ui,
    item: &native_lsp::CompletionItem,
    prefix: &str,
    selected: bool,
) -> egui::Response {
    let response = appearance::popup_row(ui, ROW_HEIGHT, selected);
    if !ui.is_rect_visible(response.rect) {
        return response;
    }
    let rect = response.rect;
    let painter = ui.painter_at(rect);
    let badge = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 13.0, rect.center().y),
        egui::vec2(15.0, 15.0),
    );
    if let Some((letter, tint)) = kind_badge(item.kind.as_deref()) {
        painter.rect_filled(badge, 3, tint.gamma_multiply(0.22));
        painter.text(
            badge.center(),
            egui::Align2::CENTER_CENTER,
            letter,
            egui::FontId::monospace(10.0),
            tint,
        );
    }
    let text = ui.visuals().text_color();
    let muted = ui.visuals().weak_text_color();
    let label = ui.painter().layout_job(appearance::highlighted_label(
        appearance::HighlightedLabelInput {
            text: &item.label,
            query: prefix,
            font: egui::FontId::monospace(13.0),
            color: text,
            matched: appearance::popup_match_color(ui.visuals()),
        },
    ));
    let label_left = badge.right() + 7.0;
    let label_width = label.size().x;
    painter.galley(
        egui::pos2(label_left, rect.center().y - label.size().y * 0.5),
        label,
        text,
    );
    if let Some(detail) = &item.detail {
        let room = (rect.right() - 8.0 - (label_left + label_width + 16.0)).max(0.0);
        let mut job = egui::text::LayoutJob::simple_singleline(
            detail.clone(),
            egui::FontId::proportional(12.0),
            muted,
        );
        job.wrap.max_width = room;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = ui.painter().layout_job(job);
        painter.galley(
            egui::pos2(
                rect.right() - 8.0 - galley.size().x,
                rect.center().y - galley.size().y * 0.5,
            ),
            galley,
            muted,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, true, selected, &item.label)
    });
    response
}

/// Key hints under the list, as in the `IntelliJ` lookup footer.
fn popup_footer(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    ui.separator();
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(
            egui::RichText::new("Press ⏎ or ⇥ to insert, Esc to close")
                .small()
                .weak(),
        );
    });
}
