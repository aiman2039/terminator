//! A text segmented control: equal cells on a track, one selected.
use super::super::scale::{FONT_SMALL, WIDGET_RADIUS};
use eframe::egui::{self, FontId};

/// Height of the control.
const SEGMENTED_HEIGHT: f32 = 26.0;

/// One cell. The control shows `label` when every label fits its cell,
/// else `short`, else the icon.
pub struct SegmentCell<'a> {
    pub label: &'a str,
    pub short: &'a str,
    pub icon: &'a str,
    /// Hover text and accessible name.
    pub tip: &'a str,
}

/// Which text all cells use at this width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SegmentFace {
    Label,
    Short,
    Icon,
}

/// Inner padding a cell keeps around its text.
const CELL_PAD: f32 = 10.0;

/// The fullest face that fits each cell of `width`.
fn segment_face(ui: &egui::Ui, cells: &[SegmentCell<'_>], width: f32) -> SegmentFace {
    let count = f32::from(u16::try_from(cells.len().max(1)).unwrap_or(u16::MAX));
    let room = width / count - CELL_PAD;
    let fits = |short: bool| {
        cells.iter().all(|cell| {
            let text = if short { cell.short } else { cell.label };
            ui.painter()
                .layout_no_wrap(
                    text.to_owned(),
                    FontId::proportional(FONT_SMALL),
                    egui::Color32::PLACEHOLDER,
                )
                .size()
                .x
                <= room
        })
    };
    if fits(false) {
        SegmentFace::Label
    } else if fits(true) {
        SegmentFace::Short
    } else {
        SegmentFace::Icon
    }
}

/// Paints the control over the full width; returns each cell's response.
pub fn segmented(
    ui: &mut egui::Ui,
    cells: &[SegmentCell<'_>],
    selected: usize,
) -> Vec<egui::Response> {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, SEGMENTED_HEIGHT), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, WIDGET_RADIUS, ui.visuals().faint_bg_color);
    let face = segment_face(ui, cells, width);
    let count = f32::from(u16::try_from(cells.len().max(1)).unwrap_or(u16::MAX));
    let cell_width = rect.width() / count;
    let mut left = rect.left();
    let mut responses = Vec::with_capacity(cells.len());
    for (index, cell) in cells.iter().enumerate() {
        let cell_rect = egui::Rect::from_min_size(
            egui::pos2(left, rect.top()),
            egui::vec2(cell_width, rect.height()),
        );
        left += cell_width;
        let is_selected = index == selected;
        let cell_response = ui
            .interact(cell_rect, response.id.with(index), egui::Sense::click())
            .on_hover_text(cell.tip);
        let fill = if is_selected {
            Some(ui.visuals().widgets.active.bg_fill)
        } else if cell_response.hovered() {
            Some(ui.visuals().widgets.hovered.bg_fill)
        } else {
            None
        };
        if let Some(fill) = fill {
            ui.painter()
                .rect_filled(cell_rect.shrink(3.0), WIDGET_RADIUS, fill);
        }
        let color = if is_selected {
            ui.visuals().text_color()
        } else {
            ui.visuals().weak_text_color()
        };
        match face {
            SegmentFace::Icon => {
                egui::Image::new(crate::icons::source(cell.icon))
                    .tint(color)
                    .paint_at(
                        ui,
                        egui::Rect::from_center_size(
                            cell_rect.center(),
                            egui::Vec2::splat(super::super::TOOLBAR_ICON),
                        ),
                    );
            }
            SegmentFace::Label | SegmentFace::Short => {
                let text = if face == SegmentFace::Label {
                    cell.label
                } else {
                    cell.short
                };
                ui.painter().with_clip_rect(cell_rect.shrink(3.0)).text(
                    cell_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    FontId::proportional(FONT_SMALL),
                    color,
                );
            }
        }
        crate::appearance::paint_focus(ui, &cell_response);
        cell_response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Button, true, is_selected, cell.tip)
        });
        responses.push(cell_response);
    }
    responses
}
