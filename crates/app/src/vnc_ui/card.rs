//! Painting of address-book host cards: thumbnail, status dot, name and
//! endpoint, plus the dashed "New host" card.
use super::*;

pub(super) const CARD_WIDTH: f32 = 184.0;
const THUMB_HEIGHT: f32 = 112.0;
pub(super) const CARD_HEIGHT: f32 = THUMB_HEIGHT + 48.0;
const CARD_RADIUS: f32 = 8.0;
pub(super) const ROW_HEIGHT: f32 = 44.0;

pub(super) struct PaintCardInput<'a> {
    pub(super) ui: &'a egui::Ui,
    pub(super) rect: egui::Rect,
    pub(super) hovered: bool,
    pub(super) selected: bool,
    pub(super) profile: &'a VncProfile,
    pub(super) texture: Option<&'a TextureHandle>,
    pub(super) status: Option<egui::Color32>,
}

/// Card body: thumbnail (live screen or a monitor glyph), status dot,
/// name and endpoint.
pub(super) fn paint_card(input: PaintCardInput<'_>) {
    let PaintCardInput {
        ui,
        rect,
        hovered,
        selected,
        profile,
        texture,
        status,
    } = input;
    let visuals = ui.visuals();
    let painter = ui.painter();
    let fill = if hovered {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        visuals.faint_bg_color
    };
    painter.rect_filled(rect, CARD_RADIUS, fill);
    if selected {
        painter.rect_stroke(
            rect,
            CARD_RADIUS,
            visuals.selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    let thumb = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 8.0, rect.top() + 8.0),
        egui::vec2(rect.width() - 16.0, THUMB_HEIGHT - 8.0),
    );
    paint_thumbnail(ui, thumb, texture);
    if let Some(color) = status {
        painter.circle_filled(
            egui::pos2(thumb.left() + 10.0, thumb.top() + 10.0),
            5.0,
            color,
        );
    }
    let text_left = rect.left() + 10.0;
    let width = rect.width() - 20.0;
    let name = painter.layout(
        profile.display_name(),
        egui::FontId::proportional(13.0),
        visuals.strong_text_color(),
        width,
    );
    painter.galley(
        egui::pos2(text_left, thumb.bottom() + 8.0),
        truncate(ui, name, width),
        visuals.strong_text_color(),
    );
    let endpoint = painter.layout(
        subtitle(profile),
        egui::FontId::proportional(11.0),
        visuals.weak_text_color(),
        width,
    );
    painter.galley(
        egui::pos2(text_left, thumb.bottom() + 26.0),
        truncate(ui, endpoint, width),
        visuals.weak_text_color(),
    );
}

/// Endpoint, last connection and labels.
fn subtitle(profile: &VncProfile) -> String {
    let mut text = profile.endpoint();
    if profile.last_connected > 0 {
        text.push_str(" · ");
        text.push_str(&crate::agent_presence::ago(
            profile.last_connected,
            terminator_core::now(),
        ));
    }
    for label in &profile.labels {
        text.push_str(" · #");
        text.push_str(label);
    }
    text
}

/// A list-view row: status dot, name, and subtitle. `texture` is unused.
pub(super) fn paint_row(input: PaintCardInput<'_>) {
    let PaintCardInput {
        ui,
        rect,
        hovered,
        selected,
        profile,
        status,
        ..
    } = input;
    let visuals = ui.visuals();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(rect, 6.0, visuals.widgets.hovered.weak_bg_fill);
    }
    if selected {
        painter.rect_stroke(
            rect,
            6.0,
            visuals.selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
    let dot = status.unwrap_or_else(|| visuals.weak_text_color());
    painter.circle_filled(egui::pos2(rect.left() + 14.0, rect.center().y), 4.0, dot);
    let left = rect.left() + 28.0;
    let width = (rect.width() - 36.0).max(40.0);
    let name = painter.layout(
        profile.display_name(),
        egui::FontId::proportional(13.0),
        visuals.strong_text_color(),
        width,
    );
    painter.galley(
        egui::pos2(left, rect.top() + 5.0),
        truncate(ui, name, width),
        visuals.strong_text_color(),
    );
    let detail = painter.layout(
        subtitle(profile),
        egui::FontId::proportional(11.0),
        visuals.weak_text_color(),
        width,
    );
    painter.galley(
        egui::pos2(left, rect.top() + 24.0),
        truncate(ui, detail, width),
        visuals.weak_text_color(),
    );
}

/// The screen image fitted into `thumb`, or a monitor glyph without one.
fn paint_thumbnail(ui: &egui::Ui, thumb: egui::Rect, texture: Option<&TextureHandle>) {
    let visuals = ui.visuals();
    ui.painter()
        .rect_filled(thumb, 4.0, visuals.extreme_bg_color);
    match texture {
        Some(texture) => {
            let fitted = fit_rect(thumb, texture.size_vec2());
            egui::Image::new((texture.id(), fitted.size())).paint_at(ui, fitted);
        }
        None => appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
            ui,
            rect: thumb,
            icon: "Monitor",
            size: 32.0,
            tint: visuals.weak_text_color(),
        }),
    }
}

/// Single-line galley cut with an ellipsis to `width`.
fn truncate(
    ui: &egui::Ui,
    galley: std::sync::Arc<egui::Galley>,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    if galley.rows.len() <= 1 {
        return galley;
    }
    let mut job = egui::text::LayoutJob::single_section(
        galley.text().to_owned(),
        galley
            .job
            .sections
            .first()
            .map(|section| section.format.clone())
            .unwrap_or_default(),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    ui.fonts_mut(|fonts| fonts.layout_job(job))
}

/// Largest rectangle with `size`'s aspect ratio centered in `outer`.
fn fit_rect(outer: egui::Rect, size: egui::Vec2) -> egui::Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return outer;
    }
    let scale = (outer.width() / size.x).min(outer.height() / size.y);
    egui::Rect::from_center_size(outer.center(), egui::vec2(size.x * scale, size.y * scale))
}

pub(super) fn new_host_card(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(CARD_WIDTH, CARD_HEIGHT), egui::Sense::click());
    let visuals = ui.visuals();
    let stroke = egui::Stroke::new(1.0, visuals.weak_text_color());
    let fill = if response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter()
        .rect(rect, CARD_RADIUS, fill, stroke, egui::StrokeKind::Inside);
    appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
        ui,
        rect: egui::Rect::from_center_size(
            egui::pos2(rect.center().x, rect.center().y - 10.0),
            egui::vec2(32.0, 32.0),
        ),
        icon: "Plus",
        size: 24.0,
        tint: visuals.weak_text_color(),
    });
    ui.painter().text(
        egui::pos2(rect.center().x, rect.center().y + 18.0),
        egui::Align2::CENTER_CENTER,
        "New host",
        egui::FontId::proportional(12.0),
        visuals.weak_text_color(),
    );
    response.on_hover_text("Add a VNC host")
}
