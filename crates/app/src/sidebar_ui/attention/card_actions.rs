//! The attention card's verbs: its buttons, keys and context menu.
use super::*;

/// One card button: icon, tip, test name, action.
pub(super) type CardButton = (&'static str, &'static str, &'static str, AttentionAction);

/// The card's buttons, left to right. With `notice-triage-v1` the Moon
/// button opens the snooze durations; without it, it snoozes 10 minutes.
pub(super) fn action_buttons(input: &AttentionCard<'_>) -> Vec<CardButton> {
    let mut buttons = vec![
        ("ArrowRight", "Go to context", "go", AttentionAction::Go),
        (
            "GitCompareArrows",
            "Review changes",
            "review",
            AttentionAction::Review,
        ),
    ];
    if input.show_read && !input.notice.read {
        buttons.push((
            "CircleCheck",
            "Mark read (keeps agent state) (U)",
            "read",
            AttentionAction::Read,
        ));
    } else if input.triage && input.notice.read {
        buttons.push((
            "Circle",
            "Mark unread (U)",
            "unread",
            AttentionAction::Unread,
        ));
    }
    buttons.push((
        "Pin",
        if input.pinned {
            "Unpin session (P)"
        } else {
            "Pin session to the top (P)"
        },
        "pin",
        AttentionAction::Pin,
    ));
    buttons.push(if input.triage {
        ("Moon", "Snooze…", "snooze", AttentionAction::None)
    } else {
        (
            "Moon",
            "Snooze 10 minutes",
            "snooze",
            AttentionAction::Snooze,
        )
    });
    buttons.push(("X", "Dismiss", "dismiss", AttentionAction::Dismiss));
    buttons
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

/// The snooze durations under the Moon button.
fn snooze_menu(response: &egui::Response) -> AttentionAction {
    let mut action = AttentionAction::None;
    egui::Popup::menu(response)
        .style(appearance::menu_style)
        .show(|ui| {
            for (label, secs) in super::SNOOZE_CHOICES {
                if appearance::menu_action(ui, true, label, "Moon").clicked() {
                    action = AttentionAction::SnoozeFor(secs(now()));
                }
            }
        });
    action
}

pub(super) struct ActionsInput<'a> {
    pub card: egui::Id,
    pub rect: egui::Rect,
    pub session_id: &'a str,
    pub buttons: Vec<CardButton>,
    /// Paint the buttons; hidden ones still take a click or the keyboard.
    pub show: bool,
}

/// The action buttons in `rect`. They always take input, so a pointer
/// that reaches them (and shows them) or a Tab key finds them.
pub(super) fn attention_actions(ui: &egui::Ui, input: ActionsInput<'_>) -> AttentionAction {
    let ActionsInput {
        card,
        rect,
        session_id,
        buttons,
        show,
    } = input;
    let mut action = AttentionAction::None;
    let mut left = rect.left();
    for (icon, tip, name, next) in buttons {
        let slot = egui::Rect::from_min_size(
            egui::pos2(left, rect.top()),
            egui::Vec2::splat(ATTENTION_ACTION_SIZE),
        );
        left += ATTENTION_ACTION_SIZE;
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
            appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
                ui,
                rect: slot,
                icon,
                size: appearance::TOOLBAR_ICON,
                tint: appearance::icon_color(),
            });
            appearance::paint_focus(ui, &response);
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("agent-{name}:{session_id}"),
            response.rect,
        );
        #[cfg(not(feature = "test-support"))]
        let _ = (name, session_id);
        if name == "snooze" && next == AttentionAction::None {
            let picked = snooze_menu(&response);
            if picked != AttentionAction::None {
                action = picked;
            }
        } else if response.clicked() {
            action = next;
        }
    }
    action
}
