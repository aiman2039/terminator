//! The shared full-width sidebar row.
use super::{PaintStatusIconInput, icon_color, paint_status_icon};
use eframe::egui::{self, Color32, FontId, TextStyle};

pub(super) struct RowSpec<'a> {
    pub(super) label: &'a str,
    pub(super) icon: &'a str,
    pub(super) selected: bool,
    pub(super) height: f32,
    pub(super) trailing: &'a str,
    pub(super) tint: Color32,
    pub(super) icon_tint: Option<Color32>,
    pub(super) spin: bool,
    pub(super) subtitle: Option<&'a str>,
    pub(super) brand: Option<&'a str>,
    /// Reserve the brand slot even when `brand` is absent so the status icon
    /// stays in one column across rows. Session rows reserve it; plain rows
    /// (project folders, settings, headers) stay compact.
    pub(super) reserve_brand: bool,
}

/// Consistent full-width native sidebar row with fixed icon and status columns.
pub(crate) struct RowInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub label: &'a str,
    pub icon: &'a str,
    pub selected: bool,
    pub height: f32,
    pub trailing: &'a str,
    pub tint: Color32,
}

pub fn row(input: RowInput<'_>) -> egui::Response {
    let RowInput {
        ui,
        label,
        icon,
        selected,
        height,
        trailing,
        tint,
    } = input;
    row_ext(
        ui,
        RowSpec {
            label,
            icon,
            selected,
            height,
            trailing,
            tint,
            icon_tint: None,
            spin: false,
            subtitle: None,
            brand: None,
            reserve_brand: false,
        },
    )
}

pub(super) fn row_ext(ui: &mut egui::Ui, spec: RowSpec<'_>) -> egui::Response {
    let RowSpec {
        label,
        icon,
        selected,
        height,
        trailing,
        tint,
        icon_tint,
        spin,
        subtitle,
        brand,
        reserve_brand,
    } = spec;
    // A reserved brand slot keeps the status icon and the label in one column
    // whether or not this row paints a brand glyph.
    let reserved = reserve_brand || brand.is_some();
    let label_left = if reserved { 36.0 } else { 26.0 };
    let previous_height = ui.spacing().interact_size.y;
    ui.spacing_mut().interact_size.y = height;
    let response = ui.add_sized(
        [ui.available_width(), height],
        egui::Button::new("").frame(false),
    );
    ui.spacing_mut().interact_size.y = previous_height;
    // Keep geometry and interaction IDs stable while avoiding text layout and
    // image loading for rows outside the sidebar viewport.
    if !ui.is_rect_visible(response.rect) {
        response
            .widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
        return response;
    }
    if response.hovered() || selected {
        ui.painter().rect_filled(
            response.rect,
            4,
            if selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.hovered.bg_fill
            },
        );
    }
    let right = response.rect.right() - trailing_width(ui, trailing, tint);
    let rect = egui::Rect::from_min_max(
        egui::pos2(response.rect.left() + label_left, response.rect.top()),
        egui::pos2(
            right.max(response.rect.left() + label_left),
            response.rect.bottom(),
        ),
    );
    let text = RowText::layout(LayoutInput {
        ui,
        rect,
        label,
        subtitle,
    });
    let label_center = text.top + text.label.size().y * 0.5;
    paint_row_icons(
        ui,
        RowIcons {
            row: response.rect,
            label_center,
            reserved,
            brand,
            icon,
            tint: icon_tint.unwrap_or(icon_color()),
            spin,
        },
    );
    text.paint(ui, rect);
    paint_trailing(PaintTrailingInput {
        ui,
        anchor: egui::pos2(response.rect.right(), label_center),
        trailing,
        tint,
    });
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    response
}

fn trailing_width(ui: &egui::Ui, trailing: &str, tint: Color32) -> f32 {
    if trailing.is_empty() {
        4.0
    } else {
        ui.painter()
            .layout_no_wrap(trailing.into(), FontId::proportional(12.0), tint)
            .size()
            .x
            + 12.0
    }
}

const SUBTITLE_GAP: f32 = 1.0;

/// The row's label and optional subtitle, vertically centered as a block.
/// Painted rather than a selectable Label widget: the row must own clicks
/// on its text as well as its icon and empty space.
struct RowText {
    label: std::sync::Arc<egui::Galley>,
    subtitle: Option<std::sync::Arc<egui::Galley>>,
    top: f32,
}

struct LayoutInput<'a> {
    ui: &'a egui::Ui,
    rect: egui::Rect,
    label: &'a str,
    subtitle: Option<&'a str>,
}

impl RowText {
    fn layout(input: LayoutInput<'_>) -> Self {
        let LayoutInput {
            ui,
            rect,
            label,
            subtitle,
        } = input;
        let single_line = |text: &str, font: FontId, color: Color32| {
            let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, rect.width());
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            ui.painter().layout_job(job)
        };
        let label = single_line(
            label,
            TextStyle::Body.resolve(ui.style()),
            ui.visuals().text_color(),
        );
        let subtitle = subtitle.map(|text| {
            single_line(
                text,
                FontId::proportional(11.0),
                ui.visuals().weak_text_color(),
            )
        });
        let block = label.size().y + subtitle.as_ref().map_or(0.0, |s| SUBTITLE_GAP + s.size().y);
        let top = rect.center().y - block * 0.5;
        Self {
            label,
            subtitle,
            top,
        }
    }

    fn paint(self, ui: &egui::Ui, rect: egui::Rect) {
        let painter = ui.painter().with_clip_rect(rect);
        let sub_top = self.top + self.label.size().y + SUBTITLE_GAP;
        painter.galley(
            egui::pos2(rect.left(), self.top),
            self.label,
            ui.visuals().text_color(),
        );
        if let Some(sub) = self.subtitle {
            painter.galley(
                egui::pos2(rect.left(), sub_top),
                sub,
                ui.visuals().weak_text_color(),
            );
        }
    }
}

struct RowIcons<'a> {
    row: egui::Rect,
    label_center: f32,
    reserved: bool,
    brand: Option<&'a str>,
    icon: &'a str,
    tint: Color32,
    spin: bool,
}

/// Optional brand glyph, then the status icon in its fixed column.
fn paint_row_icons(ui: &mut egui::Ui, icons: RowIcons<'_>) {
    let RowIcons {
        row,
        label_center,
        reserved,
        brand,
        icon,
        tint,
        spin,
    } = icons;
    if let Some(brand) = brand {
        paint_status_icon(PaintStatusIconInput {
            ui,
            rect: egui::Rect::from_center_size(
                egui::pos2(row.left() + 9.0, label_center),
                egui::vec2(14.0, 14.0),
            ),
            icon: brand,
            tint: icon_color(),
            spin: false,
        });
    }
    let size = if brand.is_some() { 14.0 } else { 16.0 };
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(
            row.left() + if reserved { 25.0 } else { 12.0 },
            label_center,
        ),
        egui::vec2(size, size),
    );
    paint_status_icon(PaintStatusIconInput {
        ui,
        rect: icon_rect,
        icon,
        tint,
        spin,
    });
}

struct PaintTrailingInput<'a> {
    ui: &'a egui::Ui,
    anchor: egui::Pos2,
    trailing: &'a str,
    tint: Color32,
}

/// A `●` trailing marker is a dot; other text is right-aligned at `anchor`
/// (the row's right edge and label center).
fn paint_trailing(input: PaintTrailingInput<'_>) {
    let PaintTrailingInput {
        ui,
        anchor,
        trailing,
        tint,
    } = input;
    if trailing == "●" {
        ui.painter()
            .circle_filled(egui::pos2(anchor.x - 10.0, anchor.y), 3.5, tint);
    } else {
        ui.painter().text(
            egui::pos2(anchor.x - 5.0, anchor.y),
            egui::Align2::RIGHT_CENTER,
            trailing,
            FontId::proportional(12.0),
            if trailing.contains(' ') {
                ui.visuals().weak_text_color()
            } else {
                tint
            },
        );
    }
}
