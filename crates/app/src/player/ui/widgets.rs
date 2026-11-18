use super::*;

/// Gap between the mode toggle and its halves; the outer corner stays
/// concentric with the halves' widget corner.
const MODE_INSET: f32 = 2.0;

/// Padding between the playlist or radio surface and its content.
const PANEL_PAD: f32 = 8.0;

/// The rounded surface that holds the playlist or the radio catalog. It
/// takes the remaining height.
pub(super) fn surface_panel(app: &mut App, ui: &mut Ui, add: impl FnOnce(&mut App, &mut Ui)) {
    let panel = ui.available_rect_before_wrap();
    ui.painter().rect_filled(
        panel,
        appearance::RADIUS_LARGE,
        appearance::color(&app.theme.surface),
    );
    ui.advance_cursor_after_rect(panel);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(panel.shrink(PANEL_PAD))
            .id_salt("player-panel"),
    );
    add(app, &mut inner);
}

/// A line across the surface panel, between its header, list and footer.
pub(super) fn divider(ui: &mut Ui) {
    ui.add_space(4.0);
    let x = ui.max_rect().expand2(vec2(PANEL_PAD, 0.0)).x_range();
    let y = ui.cursor().min.y;
    ui.painter()
        .hline(x, y, ui.visuals().widgets.noninteractive.bg_stroke);
    ui.add_space(4.0);
}

/// The round accent Play/Pause button.
pub(super) fn play_button(ui: &mut Ui, playing: bool) -> egui::Response {
    let name = if playing { "Pause" } else { "Play" };
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(PLAY_BUTTON), Sense::click());
    let response = response.on_hover_text(name);
    let accent = ui.visuals().selection.stroke.color;
    let fill = if response.hovered() {
        accent.gamma_multiply(0.85)
    } else {
        accent
    };
    ui.painter()
        .circle_filled(rect.center(), PLAY_BUTTON / 2.0, fill);
    appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
        ui,
        rect,
        icon: name,
        size: 18.0,
        tint: appearance::contrast_text(accent),
    });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), name));
    appearance::paint_focus(ui, &response);
    response
}

/// Frameless icon and label button of the playlist footer; accent while its
/// menu is open.
pub(super) fn footer_button(ui: &mut Ui, (icon, label): (&str, &str), lit: bool) -> egui::Response {
    let tint = if lit {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().text_color()
    };
    appearance::icon_label_chip(appearance::IconLabelChipInput {
        ui,
        icon,
        label,
        tint,
        height: appearance::HEADER_BUTTON,
        max_width: 96.0,
    })
}

pub(super) fn mode_toggle(app: &mut App, ui: &mut Ui) {
    let radio = app.player.radio_mode;
    let (rect, _) = ui.allocate_exact_size(vec2(160.0, appearance::HEADER_BUTTON), Sense::hover());
    ui.painter().rect(
        rect,
        f32::from(appearance::WIDGET_RADIUS) + MODE_INSET,
        ui.visuals().extreme_bg_color,
        ui.visuals().widgets.inactive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let mid = rect.center().x;
    let files = Rect::from_min_max(rect.min, pos2(mid, rect.max.y)).shrink(MODE_INSET);
    let radio_rect = Rect::from_min_max(pos2(mid, rect.min.y), rect.max).shrink(MODE_INSET);
    if mode_half(ModeHalfInput {
        ui,
        rect: files,
        icon: "ListMusic",
        label: "Files",
        active: !radio,
    })
    .clicked()
    {
        app.player.radio_mode = false;
        app.preferences.player_radio_mode = false;
        app.player.pl_open = true;
    }
    if mode_half(ModeHalfInput {
        ui,
        rect: radio_rect,
        icon: "Radio",
        label: "Radio",
        active: radio,
    })
    .clicked()
    {
        app.player.radio_mode = true;
        app.preferences.player_radio_mode = true;
        app.player.pl_open = true;
    }
}

pub(crate) struct ModeHalfInput<'a> {
    pub(crate) ui: &'a mut Ui,
    pub(crate) rect: Rect,
    pub(crate) icon: &'a str,
    pub(crate) label: &'a str,
    pub(crate) active: bool,
}

pub(super) fn mode_half(input: ModeHalfInput<'_>) -> egui::Response {
    let ModeHalfInput {
        ui,
        rect,
        icon,
        label,
        active,
    } = input;
    let response = ui.interact(rect, ui.scope_id().with(icon), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, label));
    if active {
        ui.painter().rect_filled(
            rect,
            appearance::WIDGET_RADIUS,
            ui.visuals().selection.bg_fill,
        );
    } else if response.hovered() {
        ui.painter().rect_filled(
            rect,
            appearance::WIDGET_RADIUS,
            ui.visuals().widgets.hovered.bg_fill,
        );
    }
    let tint = if active {
        ui.visuals().selection.stroke.color
    } else {
        appearance::icon_color()
    };
    let text = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::proportional(appearance::FONT_SMALL),
        tint,
    );
    let glyph = appearance::TOOLBAR_ICON;
    let gap = 6.0;
    let left = rect.center().x - (glyph + gap + text.size().x) / 2.0;
    let middle = rect.center().y;
    egui::Image::new(icons::source(icon)).tint(tint).paint_at(
        ui,
        Rect::from_center_size(pos2(left + glyph / 2.0, middle), vec2(glyph, glyph)),
    );
    let at = pos2(left + glyph + gap, middle - text.size().y / 2.0);
    ui.painter().galley(at, text, tint);
    response
}

pub(super) fn chip(ui: &mut Ui, label: &str, active: bool) -> egui::Response {
    let fill = if active {
        ui.visuals().selection.bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let stroke = if active {
        Stroke::new(1.0, ui.visuals().selection.stroke.color)
    } else {
        ui.visuals().widgets.inactive.bg_stroke
    };
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(appearance::FONT_BODY))
            .fill(fill)
            .stroke(stroke)
            .corner_radius(appearance::WIDGET_RADIUS)
            .min_size(vec2(56.0, 28.0)),
    )
}

pub(super) fn eq_matches(app: &App, preset: &EqPreset) -> bool {
    let EqPreset { preamp, bands } = *preset;
    (app.player.eq_preamp - preamp).abs() < 0.03
        && app
            .player
            .eq_bands
            .iter()
            .zip(bands)
            .all(|(left, right)| (left - right).abs() < 0.03)
}

pub(crate) struct VSliderInput<'a> {
    pub(crate) ui: &'a mut Ui,
    pub(crate) rect: Rect,
    pub(crate) id: &'a str,
    pub(crate) value: f32,
    pub(crate) groove: Color32,
    pub(crate) thumb: Color32,
}

pub(super) fn v_slider(input: VSliderInput<'_>) -> Option<f32> {
    let VSliderInput {
        ui,
        rect,
        id,
        value,
        groove,
        thumb,
    } = input;
    let response = ui.interact(rect, ui.scope_id().with(id), Sense::click_and_drag());
    ui.painter()
        .rect_filled(rect, appearance::RADIUS_XSMALL, groove);
    let y = rect.max.y - 4.0 - (rect.height() - 8.0) * value.clamp(0.0, 1.0);
    let knob = Rect::from_center_size(pos2(rect.center().x, y), vec2(rect.width() + 4.0, 6.0));
    ui.painter()
        .rect_filled(knob, appearance::RADIUS_XSMALL, thumb);
    if response.clicked() || response.dragged() {
        return response.interact_pointer_pos().map(|pos| {
            (1.0 - (pos.y - rect.min.y - 4.0) / (rect.height() - 8.0).max(1.0)).clamp(0.0, 1.0)
        });
    }
    None
}
