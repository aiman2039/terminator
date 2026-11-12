//! The pane caption's Resume chip. It shows on every pane, with or without
//! focus: its visibility follows agent presence only.
use super::*;
use crate::appearance::paint_focus;

const RESUME_LABEL: &str = "Resume";
/// The chip's label when presence could not confirm the agent stopped.
const RESUME_UNSURE_LABEL: &str = "Resume ?";

const fn resume_label(unsure: bool) -> &'static str {
    if unsure {
        RESUME_UNSURE_LABEL
    } else {
        RESUME_LABEL
    }
}

/// The Resume chip's width.
pub(super) fn resume_chip_width(ui: &egui::Ui, unsure: bool) -> f32 {
    let text = ui
        .painter()
        .layout_no_wrap(
            resume_label(unsure).into(),
            FontId::proportional(FONT_SMALL),
            icon_color(),
        )
        .size()
        .x;
    6.0 + super::super::TOOLBAR_ICON + 4.0 + text + 8.0
}

/// An outlined `▷ Resume` chip (`▷ Resume ?` when unsure). Only a click
/// acts on it.
pub(super) fn resume_chip(
    ui: &egui::Ui,
    bar_id: egui::Id,
    rect: egui::Rect,
    chip: ResumeChip<'_>,
) -> egui::Response {
    let label = resume_label(chip.unsure);
    let response = ui
        .interact(rect, bar_id.with("resume-chip"), egui::Sense::click())
        .on_hover_text(chip.tip)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    let visuals = ui.visuals();
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, WIDGET_RADIUS, visuals.widgets.hovered.bg_fill);
    }
    ui.painter().rect_stroke(
        rect,
        WIDGET_RADIUS,
        visuals.widgets.inactive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let icon = egui::Rect::from_center_size(
        egui::pos2(
            rect.left() + 6.0 + super::super::TOOLBAR_ICON / 2.0,
            rect.center().y,
        ),
        egui::Vec2::splat(super::super::TOOLBAR_ICON),
    );
    egui::Image::new(crate::icons::source("Play"))
        .tint(icon_color())
        .paint_at(ui, icon);
    ui.painter().text(
        egui::pos2(icon.right() + 4.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        FontId::proportional(FONT_SMALL),
        visuals.text_color(),
    );
    paint_focus(ui, &response);
    response
}
