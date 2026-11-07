//! Search popup geometry, theme colors, result rows and footer.
use super::*;

/// Height of one result row.
pub(super) const ROW_HEIGHT: f32 = 26.0;
/// Height kept for the footer under the list.
pub(super) const FOOTER_HEIGHT: f32 = 26.0;
/// Popup width bounds.
const MIN_WIDTH: f32 = 560.0;
const MAX_WIDTH: f32 = 960.0;
/// Popup height bounds.
const MIN_HEIGHT: f32 = 320.0;
const MAX_HEIGHT: f32 = 760.0;
/// Space kept between the popup and the window edges.
const EDGE: f32 = 16.0;

/// Where the popup sits in `content`: centered, near the top, about half
/// the width and 60% of the height, inside the window edges.
pub(crate) fn palette_rect(content: egui::Rect) -> egui::Rect {
    let room = (content.width() - 2.0 * EDGE).max(0.0);
    let width = (content.width() * 0.5)
        .clamp(MIN_WIDTH, MAX_WIDTH)
        .min(room);
    let top = content.top() + (content.height() * 0.18).max(EDGE);
    let height = (content.height() * 0.6)
        .clamp(MIN_HEIGHT, MAX_HEIGHT)
        .min((content.bottom() - EDGE - top).max(0.0));
    egui::Rect::from_min_size(
        egui::pos2(content.center().x - width / 2.0, top),
        egui::vec2(width, height),
    )
}

/// Theme colors copied out before the body borrows the app.
#[derive(Clone, Copy)]
pub(super) struct PaletteColors {
    pub(super) surface: egui::Color32,
    pub(super) field: egui::Color32,
    pub(super) border: egui::Color32,
    pub(super) text: egui::Color32,
    pub(super) secondary: egui::Color32,
    pub(super) accent: egui::Color32,
    pub(super) selection: egui::Color32,
    pub(super) hover: egui::Color32,
}

impl PaletteColors {
    pub(super) fn new(theme: &terminator_core::appearance::AppearanceConfig) -> Self {
        Self {
            surface: appearance::color(&theme.surface),
            field: appearance::color(&theme.window),
            border: appearance::color(&theme.border),
            text: appearance::color(&theme.text),
            secondary: appearance::color(&theme.secondary),
            accent: appearance::color(&theme.accent),
            selection: appearance::color(&theme.selection),
            hover: appearance::color(&theme.hover),
        }
    }
}

pub(super) struct PaletteRowInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) item: &'a PaletteItem,
    pub(super) text: Option<&'a RowText>,
    pub(super) selected: bool,
    pub(super) colors: PaletteColors,
}

/// One row: icon, title and a dim note on the left, the owner on the right.
pub(super) fn palette_row(input: PaletteRowInput<'_>) -> egui::Response {
    let PaletteRowInput {
        ui,
        item,
        text,
        selected,
        colors,
    } = input;
    let size = egui::vec2(ui.available_width(), ROW_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if selected {
        Some(colors.selection)
    } else {
        response.hovered().then_some(colors.hover)
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(appearance::RADIUS_SMALL),
            fill,
        );
    }
    let empty = RowText::default();
    let text = text.unwrap_or(&empty);
    let mut row = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(8.0, 0.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    if !text.right.is_empty() {
        row.add(
            egui::Label::new(
                egui::RichText::new(&text.right)
                    .size(appearance::FONT_SMALL)
                    .color(colors.secondary),
            )
            .selectable(false),
        );
    }
    row.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let title = if let Some(icon) = item.icon() {
            let mut image = egui::Image::new(crate::icons::source(icon))
                .fit_to_exact_size(egui::Vec2::splat(appearance::TOOLBAR_ICON));
            // Language icons carry their own palette.
            if !crate::icons::is_language(icon) {
                image = image.tint(colors.text);
            }
            ui.add(image);
            item.title()
        } else {
            item.label()
        };
        ui.add(
            egui::Label::new(
                egui::RichText::new(title)
                    .size(appearance::FONT_BODY)
                    .color(colors.text),
            )
            .selectable(false)
            .truncate(),
        );
        if !text.inline.is_empty() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&text.inline)
                        .size(appearance::FONT_SMALL)
                        .color(colors.secondary),
                )
                .selectable(false)
                .truncate(),
            );
        }
    });
    response
}

/// The selected row's path, under a rule at the bottom of the popup.
pub(super) fn palette_footer(ui: &mut egui::Ui, text: Option<&RowText>, colors: PaletteColors) {
    ui.separator();
    let footer = text.map_or("", |text| text.footer.as_str());
    ui.add(
        egui::Label::new(
            egui::RichText::new(footer)
                .size(appearance::FONT_SMALL)
                .color(colors.secondary),
        )
        .selectable(false)
        .truncate(),
    );
}

/// A primary press this frame that starts outside `popup`.
pub(super) fn pressed_outside(ctx: &egui::Context, popup: egui::Rect) -> bool {
    ctx.input(|input| {
        input.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } if !popup.contains(*pos)
            )
        })
    })
}
