//! The attention card: two lines. Line 1: status glyph, session label,
//! project and age; the action buttons take the age's place while the
//! pointer is on the card or the keyboard is in it. Line 2: brand glyph and
//! the notice text, one line with an ellipsis.
use super::*;

/// Height of the notice line under the title row.
const NOTICE_LINE: f32 = 16.0;
/// Card padding: horizontal, vertical.
const PAD: egui::Vec2 = egui::vec2(6.0, 3.0);
/// The brand glyph on the notice line.
const BRAND_SIZE: f32 = 12.0;
/// Height of the "This event has resolved." line.
const RESOLVED_LINE: f32 = 18.0;

pub(super) fn attention_status_glyph(
    ui: &mut egui::Ui,
    state: AgentState,
    theme: &AppearanceConfig,
) -> egui::Response {
    let tint = state_color(state, theme);
    // An exact button, not `Button::image`: the button minimum follows
    // `interact_size` and inflated this glyph past the 22px row.
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

use super::card_actions::{
    ActionsInput, CardButton, action_buttons, attention_actions, card_keys, card_menu,
};

/// One line of text cut with an ellipsis at `width`.
fn one_line(
    ui: &egui::Ui,
    parts: &[(&str, f32, Color32)],
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::default();
    for (text, size, color) in parts {
        job.append(
            text,
            0.0,
            egui::TextFormat::simple(egui::FontId::proportional(*size), *color),
        );
    }
    job.wrap.max_width = width.max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}

pub(crate) fn attention_card(ui: &mut egui::Ui, input: AttentionCard<'_>) -> AttentionAction {
    attention_card_in(ui, input, None)
}

/// An [`attention_card`] that names the session's project on line 1.
pub(crate) fn attention_card_in(
    ui: &mut egui::Ui,
    input: AttentionCard<'_>,
    project: Option<&str>,
) -> AttentionAction {
    ui.push_id(("attention-card", &input.notice.session_id), |ui| {
        attention_card_contents(ui, input, project)
    })
    .inner
}

fn card_height(resolved: bool) -> f32 {
    PAD.y * 2.0 + ATTENTION_ACTION_SIZE + NOTICE_LINE + if resolved { RESOLVED_LINE } else { 0.0 }
}

/// The card frame: hover fill, and an accent, selection or border line.
fn paint_card_frame(ui: &egui::Ui, input: &AttentionCard<'_>, rect: egui::Rect, hovered: bool) {
    let theme = input.theme;
    let stroke = if input.highlight {
        egui::Stroke::new(1.0, appearance::color(&theme.accent))
    } else if input.selected {
        egui::Stroke::new(1.0, appearance::color(&theme.selection))
    } else {
        egui::Stroke::new(theme.border_width, appearance::color(&theme.border))
    };
    let fill = appearance::color(if hovered { &theme.hover } else { &theme.window });
    ui.painter().rect(
        rect,
        appearance::RADIUS_MEDIUM,
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );
}

fn paint_resolved_line(ui: &egui::Ui, left: f32, line1_bottom: f32) {
    ui.painter().text(
        egui::pos2(left, line1_bottom + NOTICE_LINE + RESOLVED_LINE * 0.5),
        egui::Align2::LEFT_CENTER,
        "This event has resolved.",
        egui::FontId::proportional(appearance::FONT_CAPTION),
        ui.visuals().weak_text_color(),
    );
}

fn attention_card_contents(
    ui: &mut egui::Ui,
    input: AttentionCard<'_>,
    project: Option<&str>,
) -> AttentionAction {
    let notice = input.notice;
    let height = card_height(notice.resolved);
    if !notice.resolved && skip_clipped_row(ui, height) {
        return AttentionAction::None;
    }
    // The card is registered before its glyph and buttons, so they sit on
    // top of it in egui and keep their own clicks.
    let card_id = ui.scope_id().with("card");
    let buttons = action_buttons(&input);
    let keyboard_inside = ui.memory(|memory| memory.focused()).is_some_and(|id| {
        id == card_id
            || buttons
                .iter()
                .any(|(_, _, name, _)| card_id.with(*name) == id)
    });
    // Read before the card takes input, so Space peeks instead of clicking.
    let key_action = if keyboard_inside {
        card_keys(ui, &input)
    } else {
        AttentionAction::None
    };
    let (_, rect) = ui.allocate_space(egui::vec2(ui.available_width(), height));
    let card = ui.interact(rect, card_id, egui::Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    paint_card_frame(ui, &input, rect, hovered);
    appearance::paint_focus(ui, &card);
    let inner = rect.shrink2(PAD);
    let line1 =
        egui::Rect::from_min_size(inner.min, egui::vec2(inner.width(), ATTENTION_ACTION_SIZE));
    let glyph = ui
        .scope_builder(
            egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(
                line1.min,
                egui::Vec2::splat(ATTENTION_ACTION_SIZE),
            )),
            |ui| attention_status_glyph(ui, notice.state, input.theme),
        )
        .inner;
    #[cfg(feature = "test-support")]
    diagnostics::record(
        ui.ctx(),
        &format!("agent-status:{}", notice.session_id),
        glyph.rect,
    );
    let show = hovered || keyboard_inside;
    let action = paint_title_row(
        ui,
        TitleRow {
            input: &input,
            card: card.id,
            line1,
            title_left: glyph.rect.right() + 4.0,
            project,
            show,
            buttons,
        },
    );
    paint_notice_line(ui, &input, line1);
    if notice.resolved {
        paint_resolved_line(ui, inner.left(), line1.bottom());
    }
    let card = card_hover(card, &input);
    let menu_action = card_menu(&card, &input);
    if action != AttentionAction::None {
        action
    } else if key_action != AttentionAction::None {
        key_action
    } else if menu_action != AttentionAction::None {
        menu_action
    } else if card.clicked() || glyph.clicked() {
        AttentionAction::Go
    } else {
        AttentionAction::None
    }
}

struct TitleRow<'a> {
    input: &'a AttentionCard<'a>,
    card: egui::Id,
    line1: egui::Rect,
    title_left: f32,
    project: Option<&'a str>,
    /// Buttons in place of the age.
    show: bool,
    buttons: Vec<CardButton>,
}

/// Session label, project and age (or the buttons) on line 1.
fn paint_title_row(ui: &egui::Ui, row: TitleRow<'_>) -> AttentionAction {
    let TitleRow {
        input,
        card,
        line1,
        title_left,
        project,
        show,
        buttons,
    } = row;
    let notice = input.notice;
    #[expect(clippy::cast_precision_loss, reason = "a handful of buttons")]
    let count = buttons.len() as f32;
    let actions_rect = egui::Rect::from_min_max(
        egui::pos2(line1.right() - ATTENTION_ACTION_SIZE * count, line1.top()),
        line1.max,
    );
    let tint = state_color(notice.state, input.theme);
    let age = crate::agent_presence::short_age(notice.created, now());
    let age_width = ui
        .painter()
        .layout_no_wrap(
            age.clone(),
            egui::FontId::proportional(appearance::FONT_CAPTION),
            tint,
        )
        .size()
        .x;
    let title_right = if show {
        actions_rect.left() - 4.0
    } else {
        line1.right() - age_width - 6.0
    };
    paint_title_text(
        ui,
        input,
        project,
        egui::Rect::from_min_max(
            egui::pos2(title_left, line1.top()),
            egui::pos2(title_right, line1.bottom()),
        ),
    );
    // The title's place before the buttons, also while the age shows.
    let title_rect = egui::Rect::from_min_max(
        egui::pos2(title_left, line1.top()),
        egui::pos2((actions_rect.left() - 4.0).max(title_left), line1.bottom()),
    );
    #[cfg(feature = "test-support")]
    diagnostics::record(
        ui.ctx(),
        &format!("agent-row:{}", notice.session_id),
        title_rect,
    );
    let _ = title_rect;
    if !show {
        ui.painter().text(
            egui::pos2(line1.right(), line1.center().y),
            egui::Align2::RIGHT_CENTER,
            age,
            egui::FontId::proportional(appearance::FONT_CAPTION),
            tint,
        );
    }
    attention_actions(
        ui,
        ActionsInput {
            card,
            rect: actions_rect,
            session_id: &notice.session_id,
            buttons,
            show,
        },
    )
}

/// "label · project · N events" in `rect`, cut with an ellipsis.
fn paint_title_text(
    ui: &egui::Ui,
    input: &AttentionCard<'_>,
    project: Option<&str>,
    rect: egui::Rect,
) {
    let label = input
        .session
        .map_or("Unknown session", |session| session.label.as_str());
    let weak = ui.visuals().weak_text_color();
    let mut project = project.map(|name| format!(" · {name}")).unwrap_or_default();
    if input.pinned {
        project.push_str(" · pinned");
    }
    let events = if input.group_extra.is_empty() {
        String::new()
    } else {
        format!(" · {} events", input.group_extra.len().saturating_add(1))
    };
    let small = appearance::FONT_SMALL;
    let title = one_line(
        ui,
        &[
            (label, small, ui.visuals().text_color()),
            (&project, small, weak),
            (&events, small, weak),
        ],
        rect.width(),
    );
    ui.painter().galley(
        egui::pos2(rect.left(), rect.center().y - title.size().y * 0.5),
        title,
        ui.visuals().text_color(),
    );
}

/// Line 2: the brand glyph, then the notice text cut with an ellipsis.
fn paint_notice_line(ui: &egui::Ui, input: &AttentionCard<'_>, line1: egui::Rect) {
    let line = egui::Rect::from_min_size(
        egui::pos2(line1.left(), line1.bottom()),
        egui::vec2(line1.width(), NOTICE_LINE),
    );
    let mut left = line.left() + 2.0;
    if let Some(brand) = input.brand_icon {
        // Brand glyphs carry their own colors; a white tint leaves them as-is.
        appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
            ui,
            rect: egui::Rect::from_min_size(
                egui::pos2(left, line.center().y - BRAND_SIZE * 0.5),
                egui::Vec2::splat(BRAND_SIZE),
            ),
            icon: brand,
            size: BRAND_SIZE,
            tint: Color32::WHITE,
        });
        left += BRAND_SIZE + 4.0;
    }
    let right = appearance::paint_usage_chips(ui, line, line.right(), input.usage);
    let text = notice_preview(&input.notice.summary);
    let weak = ui.visuals().weak_text_color();
    let galley = one_line(
        ui,
        &[(&text, appearance::FONT_CAPTION, weak)],
        right - 4.0 - left,
    );
    ui.painter().galley(
        egui::pos2(left, line.center().y - galley.size().y * 0.5),
        galley,
        weak,
    );
}

/// The card tooltip: where the session runs, and each folded notice.
fn card_hover(card: egui::Response, input: &AttentionCard<'_>) -> egui::Response {
    let label = match (input.brand_icon, input.brand_label) {
        (Some(_), Some(label)) => label,
        (Some(_), None) => "Agent",
        _ => "",
    };
    card.on_hover_ui(|ui| {
        ui.set_max_width(360.0);
        if !label.is_empty() {
            ui.weak(label);
        }
        if let Some(session) = input.session {
            ui.weak(crate::remote_ui::location_label(session));
        }
        ui.label(notice_preview(&input.notice.summary));
        for extra in input.group_extra.iter().take(8) {
            ui.label(format!(
                "{}: {}",
                extra.state.label(),
                notice_preview(&extra.summary)
            ));
        }
        if input.group_extra.len() > 8 {
            ui.weak(format!(
                "…and {} more",
                input.group_extra.len().saturating_sub(8)
            ));
        }
    })
}
