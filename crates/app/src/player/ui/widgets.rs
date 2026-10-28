use super::*;

pub(super) fn mode_toggle(app: &mut App, ui: &mut Ui) {
    let radio = app.player.radio_mode;
    let (rect, _) = ui.allocate_exact_size(vec2(72.0, 32.0), Sense::hover());
    ui.painter().rect(
        rect,
        8.0,
        ui.visuals().extreme_bg_color,
        ui.visuals().widgets.inactive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let mid = rect.center().x;
    let files = Rect::from_min_max(rect.min, pos2(mid, rect.max.y)).shrink(2.0);
    let radio_rect = Rect::from_min_max(pos2(mid, rect.min.y), rect.max).shrink(2.0);
    if mode_half(ModeHalfInput {
        ui,
        rect: files,
        icon: "ListMusic",
        tip: "Files",
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
        tip: "Radio",
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
    pub(crate) tip: &'a str,
    pub(crate) active: bool,
}

pub(super) fn mode_half(input: ModeHalfInput<'_>) -> egui::Response {
    let ModeHalfInput {
        ui,
        rect,
        icon,
        tip,
        active,
    } = input;
    let response = ui
        .interact(rect, ui.scope_id().with(icon), Sense::click())
        .on_hover_text(tip);
    if active {
        ui.painter()
            .rect_filled(rect, 6.0, ui.visuals().selection.bg_fill);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, 6.0, ui.visuals().widgets.hovered.bg_fill);
    }
    let tint = if active {
        ui.visuals().selection.stroke.color
    } else {
        appearance::icon_color()
    };
    egui::Image::new(icons::source(icon))
        .tint(tint)
        .paint_at(ui, Rect::from_center_size(rect.center(), vec2(16.0, 16.0)));
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
        egui::Button::new(egui::RichText::new(label).size(13.0))
            .fill(fill)
            .stroke(stroke)
            .corner_radius(6.0)
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
    ui.painter().rect_filled(rect, 2.0, groove);
    let y = rect.max.y - 4.0 - (rect.height() - 8.0) * value.clamp(0.0, 1.0);
    let knob = Rect::from_center_size(pos2(rect.center().x, y), vec2(rect.width() + 4.0, 6.0));
    ui.painter().rect_filled(knob, 2.0, thumb);
    if response.clicked() || response.dragged() {
        return response.interact_pointer_pos().map(|pos| {
            (1.0 - (pos.y - rect.min.y - 4.0) / (rect.height() - 8.0).max(1.0)).clamp(0.0, 1.0)
        });
    }
    None
}
