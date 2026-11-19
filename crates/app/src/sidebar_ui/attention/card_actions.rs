//! The attention card's verbs: its buttons, keys and context menu.
use super::*;

/// One card button: icon, tip, test name, action.
pub(super) type CardButton = (&'static str, &'static str, &'static str, AttentionAction);

/// The labeled Peek slot's text.
const PEEK_LABEL: &str = "Peek";

/// The card's buttons, left to right: Go, Review, the labeled Peek and
/// Dismiss. Pin, Read/Unread (keys P, U) and Snooze are in the card menu.
pub(super) fn action_buttons() -> Vec<CardButton> {
    vec![
        ("ArrowRight", "Go to context", "go", AttentionAction::Go),
        (
            "GitCompareArrows",
            "Review changes",
            "review",
            AttentionAction::Review,
        ),
        (
            "Eye",
            "Peek at the screen, read only (Space)",
            "peek",
            AttentionAction::Peek,
        ),
        ("X", "Dismiss", "dismiss", AttentionAction::Dismiss),
    ]
}

/// Keys on a focused card: Space peeks, U toggles read, P pins.
pub(super) fn card_keys(ui: &egui::Ui, input: &AttentionCard<'_>) -> AttentionAction {
    let none = egui::Modifiers::NONE;
    ui.input_mut(|keys| {
        if keys.consume_key(none, egui::Key::Space) {
            AttentionAction::Peek
        } else if keys.consume_key(none, egui::Key::P) {
            AttentionAction::Pin
        } else if keys.consume_key(none, egui::Key::U) {
            if !input.notice.read {
                AttentionAction::Read
            } else if input.triage {
                AttentionAction::Unread
            } else {
                AttentionAction::None
            }
        } else {
            AttentionAction::None
        }
    })
}

/// The card's context menu: the triage verbs with their keys.
pub(super) fn card_menu(card: &egui::Response, input: &AttentionCard<'_>) -> AttentionAction {
    let mut action = AttentionAction::None;
    card.context_menu(|ui| {
        let mut item = |ui: &mut egui::Ui, label: &str, icon: &str, key: &str, next| {
            let input = appearance::MenuItemInput {
                ui,
                label,
                icon,
                shortcut: key,
            };
            if appearance::menu_item(input).clicked() {
                action = next;
                ui.close();
            }
        };
        item(ui, "Peek screen", "Eye", "Space", AttentionAction::Peek);
        let pin = if input.pinned { "Unpin" } else { "Pin to top" };
        item(ui, pin, "Pin", "P", AttentionAction::Pin);
        if !input.notice.read {
            item(ui, "Mark read", "CircleCheck", "U", AttentionAction::Read);
        } else if input.triage {
            item(ui, "Mark unread", "Circle", "U", AttentionAction::Unread);
        }
        ui.separator();
        if input.triage {
            for (label, secs) in super::SNOOZE_CHOICES {
                item(
                    ui,
                    label,
                    "Moon",
                    "",
                    AttentionAction::SnoozeFor(secs(now())),
                );
            }
        } else {
            item(ui, "Snooze 10 minutes", "Moon", "", AttentionAction::Snooze);
        }
        item(ui, "Dismiss", "X", "", AttentionAction::Dismiss);
    });
    action
}

pub(super) struct ActionsInput<'a> {
    pub card: egui::Id,
    pub rect: egui::Rect,
    /// Prefix of the test-support targets: `agent` or `terminal`.
    pub target: &'a str,
    pub session_id: &'a str,
    pub buttons: Vec<CardButton>,
    /// Paint the buttons; hidden ones still take a click or the keyboard.
    pub show: bool,
    /// Text beside the labeled buttons (Peek); off on a narrow card.
    pub labels: bool,
}

/// The action buttons in `rect`. They always take input, so a pointer
/// that reaches them (and shows them) or a Tab key finds them.
pub(super) fn attention_actions(ui: &egui::Ui, input: ActionsInput<'_>) -> AttentionAction {
    let ActionsInput {
        card,
        rect,
        target,
        session_id,
        buttons,
        show,
        labels,
    } = input;
    let mut action = AttentionAction::None;
    let mut left = rect.left();
    for (icon, tip, name, next) in buttons {
        let width = slot_width(ui, name, labels);
        let slot = egui::Rect::from_min_size(
            egui::pos2(left, rect.top()),
            egui::vec2(width, ATTENTION_ACTION_SIZE),
        );
        left += width;
        let response = ui
            .interact(slot, card.with(name), egui::Sense::click())
            .on_hover_text(tip);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
        if show || response.has_focus() {
            if response.hovered() {
                ui.painter().rect_filled(
                    slot,
                    appearance::WIDGET_RADIUS,
                    ui.visuals().widgets.active.bg_fill,
                );
            }
            let glyph =
                egui::Rect::from_min_size(slot.min, egui::Vec2::splat(ATTENTION_ACTION_SIZE));
            appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
                ui,
                rect: glyph,
                icon,
                size: appearance::TOOLBAR_ICON,
                tint: appearance::icon_color(),
            });
            if let Some(label) = slot_label(name, labels) {
                ui.painter().text(
                    egui::pos2(glyph.right(), slot.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    egui::FontId::proportional(appearance::FONT_SMALL),
                    ui.visuals().text_color(),
                );
            }
            appearance::paint_focus(ui, &response);
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("{target}-{name}:{session_id}"),
            response.rect,
        );
        #[cfg(not(feature = "test-support"))]
        let _ = (target, session_id);
        if response.clicked() {
            action = next;
        }
    }
    action
}

/// The text beside a slot's glyph: only Peek has one, and only while
/// the card has room for `labels`.
fn slot_label(name: &str, labels: bool) -> Option<&'static str> {
    (labels && name == "peek").then_some(PEEK_LABEL)
}

/// A slot's width: the glyph square, plus its label and a margin.
fn slot_width(ui: &egui::Ui, name: &str, labels: bool) -> f32 {
    slot_label(name, labels).map_or(ATTENTION_ACTION_SIZE, |label| {
        let text = ui
            .painter()
            .layout_no_wrap(
                label.to_owned(),
                egui::FontId::proportional(appearance::FONT_SMALL),
                ui.visuals().text_color(),
            )
            .size()
            .x;
        ATTENTION_ACTION_SIZE + text + 6.0
    })
}

/// The buttons' total width.
pub(super) fn buttons_width(ui: &egui::Ui, buttons: &[CardButton], labels: bool) -> f32 {
    buttons
        .iter()
        .map(|(_, _, name, _)| slot_width(ui, name, labels))
        .sum()
}
