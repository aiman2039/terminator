//! Keyboard control of the address-book cards.
use super::address_book::CardAction;
use super::card::CARD_WIDTH;
use super::*;

/// Space between cards, both directions.
pub(super) const CARD_GAP: f32 = 12.0;

/// Keyboard control of the cards while no text field has focus: arrows
/// move the selection, Enter connects, Delete asks to remove, and
/// Command/Ctrl+I opens Properties.
pub(super) fn grid_key_action(
    ui: &egui::Ui,
    profiles: &[VncProfile],
    selected: &mut Option<String>,
) -> Option<(VncProfile, CardAction)> {
    if profiles.is_empty() || ui.ctx().memory(|m| m.focused().is_some()) {
        return None;
    }
    let columns = card_columns(ui.available_width());
    let (step, action) = ui.input(|i| {
        let step = if i.key_pressed(egui::Key::ArrowRight) {
            Step::Forward(1)
        } else if i.key_pressed(egui::Key::ArrowLeft) {
            Step::Back(1)
        } else if i.key_pressed(egui::Key::ArrowDown) {
            Step::Forward(columns)
        } else if i.key_pressed(egui::Key::ArrowUp) {
            Step::Back(columns)
        } else {
            Step::Forward(0)
        };
        let action = if i.key_pressed(egui::Key::Enter) {
            Some(CardAction::Connect)
        } else if i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace) {
            Some(CardAction::Remove)
        } else if i.modifiers.command && i.key_pressed(egui::Key::I) {
            Some(CardAction::Properties)
        } else {
            None
        };
        (step, action)
    });
    let current = selected
        .as_ref()
        .and_then(|id| profiles.iter().position(|p| &p.id == id));
    let last = profiles.len().saturating_sub(1);
    let index = match (current, step) {
        (None, Step::Forward(0)) => return None,
        (None, _) => 0,
        (Some(at), Step::Forward(n)) => at.saturating_add(n).min(last),
        (Some(at), Step::Back(n)) => at.saturating_sub(n),
    };
    let profile = profiles.get(index)?;
    *selected = Some(profile.id.clone());
    action.map(|action| (profile.clone(), action))
}

enum Step {
    Forward(usize),
    Back(usize),
}

/// Cards per row in `width`, at least one.
fn card_columns(width: f32) -> usize {
    let mut columns = 1usize;
    let mut used = CARD_WIDTH;
    while used + CARD_GAP + CARD_WIDTH <= width {
        columns = columns.saturating_add(1);
        used += CARD_GAP + CARD_WIDTH;
    }
    columns
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profiles(count: usize) -> Vec<VncProfile> {
        (0..count)
            .map(|n| VncProfile {
                id: format!("p{n}"),
                name: String::new(),
                host: "localhost".into(),
                port: 5900,
                username: String::new(),
                password: String::new(),
                shared: true,
                view_only: false,
                last_connected: 0,
                options: Default::default(),
                credential_saved: false,
                labels: Vec::new(),
            })
            .collect()
    }

    fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(CARD_WIDTH * 2.0 + CARD_GAP, 400.0),
            )),
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn arrows_move_the_selection_and_enter_connects_it() {
        let ctx = egui::Context::default();
        let hosts = profiles(5);
        let mut selected = None;
        let mut run = |key, modifiers| {
            let mut result = None;
            let mut output = ctx.run_ui(press(key, modifiers), |ui| {
                result = grid_key_action(ui, &hosts, &mut selected);
            });
            output.textures_delta.clear();
            (
                result.map(|(p, a)| (p.id, matches!(a, CardAction::Connect))),
                selected.clone(),
            )
        };
        assert_eq!(
            run(egui::Key::ArrowRight, egui::Modifiers::NONE)
                .1
                .as_deref(),
            Some("p0")
        );
        assert_eq!(
            run(egui::Key::ArrowDown, egui::Modifiers::NONE)
                .1
                .as_deref(),
            Some("p2")
        );
        assert_eq!(
            run(egui::Key::ArrowLeft, egui::Modifiers::NONE)
                .1
                .as_deref(),
            Some("p1")
        );
        let (action, _) = run(egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(action, Some(("p1".into(), true)));
    }

    #[test]
    fn cards_per_row_fit_the_width() {
        assert_eq!(card_columns(10.0), 1);
        assert_eq!(card_columns(CARD_WIDTH * 3.0 + CARD_GAP * 2.0), 3);
    }
}
