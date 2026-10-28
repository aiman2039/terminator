//! The attention card: status glyph, title, preview, and actions.
use super::*;

pub(super) fn attention_status_glyph(
    ui: &mut egui::Ui,
    state: AgentState,
    theme: &AppearanceConfig,
) -> egui::Response {
    let tint = state_color(state, theme);
    // An exact button, not `Button::image`: the button minimum follows
    // `interact_size` and inflated this glyph past the 22px row, pushing its
    // center below the label and action icons.
    let response = appearance::toolbar_button(ui).on_hover_ui(|ui| {
        ui.set_max_width(240.0);
        ui.colored_label(tint, state.label());
        ui.label(attention_status_detail(state));
    });
    appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
        ui,
        rect: response.rect,
        icon: attention_status_icon(state),
        size: 14.0,
        tint,
    });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, state.label()));
    response
}

pub(super) struct AttentionTitleInput<'a> {
    ui: &'a mut egui::Ui,
    notice: &'a Notification,
    group_extra: &'a [Notification],
    session: Option<&'a Session>,
    theme: &'a AppearanceConfig,
    brand: Option<(&'static str, &'a str)>,
    show_read: bool,
}

pub(super) fn attention_title(input: AttentionTitleInput<'_>) -> egui::Response {
    let AttentionTitleInput {
        ui,
        notice,
        group_extra,
        session,
        theme,
        brand,
        show_read,
    } = input;
    let spacing = ui.spacing().item_spacing.x;
    let brand_response = brand.map(|(icon, label)| {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(18.0, ATTENTION_ACTION_SIZE),
            egui::Sense::hover(),
        );
        // Brand glyphs carry their own colors; a white tint leaves them as-is.
        appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
            ui,
            rect,
            icon,
            size: 14.0,
            tint: Color32::WHITE,
        });
        response.on_hover_text(label)
    });
    let icon = attention_status_glyph(ui, notice.state, theme);
    #[cfg(feature = "test-support")]
    diagnostics::record(
        ui.ctx(),
        &format!("agent-status:{}", notice.session_id),
        icon.rect,
    );
    let session_label = session.map_or("Unknown session", |session| session.label.as_str());
    let text = if group_extra.is_empty() {
        session_label.to_string()
    } else {
        format!(
            "{session_label} · {} events",
            group_extra.len().saturating_add(1)
        )
    };
    // Measure after the leading icons so a brand glyph cannot push the
    // actions onto a second row.
    let label_width = attention_label_width(ui.available_width(), spacing, show_read);
    let label = ui
        .add_sized(
            [label_width, ATTENTION_ACTION_SIZE],
            egui::Label::new(RichText::new(text).size(12.0))
                .halign(egui::Align::LEFT)
                .truncate()
                .sense(egui::Sense::click()),
        )
        .on_hover_ui(|ui| {
            ui.set_max_width(360.0);
            if let Some(session) = session {
                ui.weak(crate::remote_ui::location_label(session));
            }
            ui.label(notice_preview(&notice.summary));
            for extra in group_extra.iter().take(8) {
                ui.label(format!(
                    "{}: {}",
                    extra.state.label(),
                    notice_preview(&extra.summary)
                ));
            }
            if group_extra.len() > 8 {
                ui.weak(format!("…and {} more", group_extra.len().saturating_sub(8)));
            }
        });
    match brand_response {
        Some(brand) => brand.union(icon).union(label),
        None => icon.union(label),
    }
}

pub(super) fn attention_actions(
    ui: &mut egui::Ui,
    session_id: &str,
    show_read: bool,
) -> AttentionAction {
    let count = if show_read {
        ATTENTION_ACTION_READ_COUNT
    } else {
        ATTENTION_ACTION_COUNT
    };
    let width = ATTENTION_ACTION_SIZE * count;
    ui.push_id(session_id, |ui| {
        ui.horizontal(|ui| {
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.spacing_mut().interact_size =
                egui::vec2(ATTENTION_ACTION_SIZE, ATTENTION_ACTION_SIZE);
            let mut action = AttentionAction::None;
            let mut buttons = vec![
                ("ArrowRight", "Go to context", "go", AttentionAction::Go),
                (
                    "Moon",
                    "Snooze 10 minutes",
                    "snooze",
                    AttentionAction::Snooze,
                ),
                ("X", "Dismiss", "dismiss", AttentionAction::Dismiss),
            ];
            if show_read {
                buttons.insert(
                    1,
                    (
                        "CircleCheck",
                        "Mark read (keeps agent state)",
                        "read",
                        AttentionAction::Read,
                    ),
                );
            }
            for (icon, tip, name, next) in buttons {
                let response = appearance::sidebar_action(ui, icon, tip);
                #[cfg(feature = "test-support")]
                diagnostics::record(
                    ui.ctx(),
                    &format!("agent-{name}:{session_id}"),
                    response.rect,
                );
                #[cfg(not(feature = "test-support"))]
                let _ = name;
                if response.clicked() {
                    action = next;
                }
            }
            action
        })
        .inner
    })
    .inner
}

pub(crate) fn attention_card(ui: &mut egui::Ui, input: AttentionCard<'_>) -> AttentionAction {
    ui.push_id(("attention-card", &input.notice.session_id), |ui| {
        attention_card_contents(ui, input)
    })
    .inner
}

pub(super) fn attention_card_contents(
    ui: &mut egui::Ui,
    input: AttentionCard<'_>,
) -> AttentionAction {
    let AttentionCard {
        theme,
        notice,
        session,
        selected,
        highlight,
        brand_icon,
        brand_label,
        show_read,
        group_extra,
    } = input;
    let brand = match (brand_icon, brand_label) {
        (Some(icon), Some(label)) => Some((icon, label)),
        (Some(icon), None) => Some((icon, "Agent")),
        _ => None,
    };
    let stroke = if highlight {
        egui::Stroke::new(1.0, appearance::color(&theme.accent))
    } else if selected {
        egui::Stroke::new(1.0, appearance::color(&theme.selection))
    } else {
        egui::Stroke::new(theme.border_width, appearance::color(&theme.border))
    };
    let frame = egui::Frame::group(ui.style())
        .stroke(stroke)
        .corner_radius(6)
        .fill(appearance::color(&theme.window))
        .inner_margin(egui::Margin::symmetric(6, 2));
    let height = ATTENTION_ACTION_SIZE + frame.total_margin().sum().y;
    if !notice.resolved && skip_clipped_row(ui, height) {
        return AttentionAction::None;
    }
    let inner = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);
        ui.spacing_mut().interact_size.y = ATTENTION_ACTION_SIZE;
        let action = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
                let header = attention_title(AttentionTitleInput {
                    ui,
                    notice,
                    group_extra,
                    session,
                    theme,
                    brand,
                    show_read,
                });
                #[cfg(feature = "test-support")]
                diagnostics::record(
                    ui.ctx(),
                    &format!("agent-row:{}", notice.session_id),
                    header.rect,
                );
                let mut action = if header.clicked() {
                    AttentionAction::Go
                } else {
                    AttentionAction::None
                };
                let buttons = attention_actions(ui, &notice.session_id, show_read);
                if buttons != AttentionAction::None {
                    action = buttons;
                }
                action
            })
            .inner;
        if notice.resolved {
            ui.weak("This event has resolved.");
        }
        action
    });
    // Header and action buttons own clicks. A later frame-wide click target
    // sits on top of those buttons in egui and would steal Dismiss/Snooze as Go.
    inner.inner
}
