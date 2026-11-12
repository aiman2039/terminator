use super::scale::{RADIUS_XSMALL, WIDGET_RADIUS};
use super::theme::icon_color;
use eframe::egui::{self, Color32};

mod buttons;
mod confirm;
mod menus;
mod popup_list;
mod tabs;
pub use buttons::*;
pub use confirm::*;
pub use menus::*;
pub use popup_list::*;
pub use tabs::*;

/// Glyph size of every icon-only toolbar control. The window header keeps it
/// in its larger square, so every toolbar row shows one icon size.
pub const TOOLBAR_ICON: f32 = 14.0;

/// Compact toolbar square: sidebar toolbars, tab strip, pane and editor bars,
/// and the status bar. Every frameless icon control there shares it so the
/// icons sit on one row regardless of the ambient [`egui::Style`] spacing.
pub const TOOLBAR_BUTTON: f32 = 22.0;

/// Window-header square. Header tools, sidebar toggles, Player, Agents,
/// search, and the header menus all use it.
pub const HEADER_BUTTON: f32 = 28.0;

/// Title-bar sidebar glyph. Resting state has no frame; hover fills only.
pub fn framed_icon(ui: &mut egui::Ui, icon: &str, tip: &str) -> egui::Response {
    framed_icon_button(FramedIconButtonInput {
        ui,
        icon,
        tip,
        selected: false,
        tint: icon_color(),
    })
}

/// Same square as [`framed_icon`], with a selected fill and a custom glyph tint.
/// Every window-header icon control uses this square.
pub(crate) struct FramedIconButtonInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub icon: &'a str,
    pub tip: &'a str,
    pub selected: bool,
    pub tint: Color32,
}

pub fn framed_icon_button(input: FramedIconButtonInput<'_>) -> egui::Response {
    let FramedIconButtonInput {
        ui,
        icon,
        tip,
        selected,
        tint,
    } = input;
    let response = header_square(FramedIconButtonInput {
        ui: &mut *ui,
        icon,
        tip,
        selected,
        tint,
    });
    accessible_icon(ui, &response, tip);
    response
}

/// One [`HEADER_BUTTON`] square: hover or selected fill, then the glyph.
fn header_square(input: FramedIconButtonInput<'_>) -> egui::Response {
    let FramedIconButtonInput {
        ui,
        icon,
        tip,
        selected,
        tint,
    } = input;
    let response = ui
        .allocate_response(egui::Vec2::splat(HEADER_BUTTON), egui::Sense::click())
        .on_hover_text(tip);
    if response.hovered() || selected {
        ui.painter().rect_filled(
            response.rect,
            WIDGET_RADIUS,
            if selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.hovered.bg_fill
            },
        );
    }
    paint_centered_icon(PaintCenteredIconInput {
        ui,
        rect: response.rect,
        icon,
        size: TOOLBAR_ICON,
        tint,
    });
    response
}

/// Frameless glyph-and-text button, such as the header branch or the
/// status-bar bell count. The glyph keeps [`TOOLBAR_ICON`], and `height` is
/// the row's button square, so it centers beside the icon buttons.
pub(crate) struct IconLabelButtonInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub icon: &'a str,
    pub label: &'a str,
    pub height: f32,
}

pub fn icon_label_button(input: IconLabelButtonInput<'_>) -> egui::Response {
    let IconLabelButtonInput {
        ui,
        icon,
        label,
        height,
    } = input;
    icon_label(
        ui,
        (icon, icon_color()),
        egui::RichText::new(label),
        (height, None),
    )
}

/// An [`icon_label_button`] in one tint whose label ends with an ellipsis
/// past `max_width`, such as the status-bar message chip.
pub(crate) struct IconLabelChipInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub icon: &'a str,
    pub label: &'a str,
    pub tint: Color32,
    pub height: f32,
    pub max_width: f32,
}

pub fn icon_label_chip(input: IconLabelChipInput<'_>) -> egui::Response {
    let IconLabelChipInput {
        ui,
        icon,
        label,
        tint,
        height,
        max_width,
    } = input;
    let text = egui::RichText::new(label).color(tint);
    icon_label(ui, (icon, tint), text, (height, Some(max_width)))
}

/// `height` is the row's button square; `max_width` truncates the label.
fn icon_label(
    ui: &mut egui::Ui,
    (icon, tint): (&str, Color32),
    label: egui::RichText,
    (height, max_width): (f32, Option<f32>),
) -> egui::Response {
    // `Button::new` keeps the exact glyph size; `image_and_text` caps it to
    // the font height.
    let image = egui::Image::new(crate::icons::source(icon))
        .tint(tint)
        .fit_to_exact_size(egui::Vec2::splat(TOOLBAR_ICON));
    let response = ui
        .scope(|ui| {
            // The ambient interact size would make the button taller than its row.
            ui.spacing_mut().interact_size.y = height;
            let button = egui::Button::new((image, label)).frame(false);
            match max_width {
                Some(width) => {
                    ui.set_max_width(width);
                    ui.add(button.truncate())
                }
                None => ui.add(button),
            }
        })
        .inner;
    paint_focus(ui, &response);
    response
}

/// Name an icon-only control for screen readers and outline it while it has
/// keyboard focus; the frameless paint otherwise shows hover state only.
fn accessible_icon(ui: &egui::Ui, response: &egui::Response, tip: &str) {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tip));
    paint_focus(ui, response);
}

/// Outline a control while it has keyboard focus: a 2 px ring in the text
/// color, which reads on every fill (an accent ring can vanish on a red
/// or blue control).
pub fn paint_focus(ui: &egui::Ui, response: &egui::Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            WIDGET_RADIUS,
            focus_ring(ui.visuals()),
            egui::StrokeKind::Inside,
        );
    }
}

/// The keyboard focus ring.
pub fn focus_ring(visuals: &egui::Visuals) -> egui::Stroke {
    egui::Stroke::new(2.0, visuals.strong_text_color())
}

/// Single-line field. Placeholder and typed text share a vertical center,
/// including when the field is stretched to a toolbar row.
pub fn singleline(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text).align(egui::Align2::LEFT_CENTER)
}

/// TCP port field where empty means "not set" (`None`). Input that is not a
/// port in 1–65535 is rejected and the previous value stays.
pub fn optional_port(ui: &mut egui::Ui, port: &mut Option<u16>, hint: &str) -> egui::Response {
    let mut text = port.map_or_else(String::new, |p| p.to_string());
    let response = ui.add(singleline(&mut text).hint_text(hint).desired_width(80.0));
    if response.changed() {
        let text = text.trim();
        if text.is_empty() {
            *port = None;
        } else if let Some(value) = text.parse::<u16>().ok().filter(|p| *p > 0) {
            *port = Some(value);
        }
    }
    response
}

/// Allocate a frameless control at an exact size. `Ui::add_sized` still lets
/// `spacing.interact_size` inflate the response, which is what pushed toolbar
/// icons to different heights; pin the spacing for the allocation.
fn exact_button(ui: &mut egui::Ui, size: egui::Vec2) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size = size;
        ui.add_sized(size, egui::Button::new("").frame(false))
    })
    .inner
}

pub fn sidebar_action(ui: &mut egui::Ui, icon: &str, tip: &str) -> egui::Response {
    let response = exact_button(ui, egui::Vec2::splat(TOOLBAR_BUTTON));
    paint_action_icon(ui, &response, icon);
    accessible_icon(ui, &response, tip);
    response.on_hover_text(tip)
}

/// Adds a control that `why` disables, with `why` as its hover text then.
/// The control keeps its size and place; only its input and opacity change.
pub fn enabled_unless(
    ui: &mut egui::Ui,
    why: Option<&str>,
    add: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> egui::Response {
    let response = ui.add_enabled_ui(why.is_none(), add).inner;
    match why {
        Some(why) => response.on_disabled_hover_text(why),
        None => response,
    }
}

/// Exact-size frameless button for toolbar icons with custom paint, such as
/// tinted status glyphs. Shares the action size so it centers on their row.
pub fn toolbar_button(ui: &mut egui::Ui) -> egui::Response {
    let response = exact_button(ui, egui::Vec2::splat(TOOLBAR_BUTTON));
    paint_focus(ui, &response);
    response
}

/// Pixel-snapped centered icon paint, so adjacent icons share one center line.
pub(crate) struct PaintCenteredIconInput<'a> {
    pub ui: &'a egui::Ui,
    pub rect: egui::Rect,
    pub icon: &'a str,
    pub size: f32,
    pub tint: Color32,
}

pub fn paint_centered_icon(input: PaintCenteredIconInput<'_>) {
    let PaintCenteredIconInput {
        ui,
        rect,
        icon,
        size,
        tint,
    } = input;
    use egui::emath::GuiRounding;
    let icon_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(size, size))
        .round_to_pixels(ui.pixels_per_point());
    egui::Image::new(crate::icons::source(icon))
        .tint(tint)
        .paint_at(ui, icon_rect);
}

/// Hover fill plus a pixel-snapped 14pt icon, so adjacent actions line up.
fn paint_action_icon(ui: &egui::Ui, response: &egui::Response, icon: &str) {
    let rect = response.rect;
    if response.hovered() || response.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, WIDGET_RADIUS, ui.visuals().widgets.hovered.bg_fill);
    }
    paint_centered_icon(PaintCenteredIconInput {
        ui,
        rect,
        icon,
        size: TOOLBAR_ICON,
        tint: icon_color(),
    });
}

/// Frameless icon button that opens a menu, matching `sidebar_action`.
pub fn icon_menu_button<R>(
    ui: &mut egui::Ui,
    icon: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let config = egui::menu::MenuConfig::new().style(menu_style);
    let size = egui::Vec2::splat(TOOLBAR_BUTTON);
    let (response, inner) = ui
        .scope(|ui| {
            ui.spacing_mut().interact_size = size;
            let button = egui::Button::new("").frame(false).min_size(size);
            egui::menu::MenuButton::from_button(button)
                .config(config)
                .ui(ui, add_contents)
        })
        .inner;
    paint_action_icon(ui, &response, icon);
    paint_focus(ui, &response);
    egui::InnerResponse::new(inner.map(|shown| shown.inner), response)
}

pub struct SelectableIconInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub icon: &'a str,
    pub tip: &'a str,
    pub selected: bool,
}

/// Icon-only selectable control (tabs, toggles). The hover overlay carries the
/// text label and the selected state fills the frame.
pub fn selectable_icon(input: SelectableIconInput<'_>) -> egui::Response {
    let SelectableIconInput {
        ui,
        icon,
        tip,
        selected,
    } = input;
    let response = exact_button(ui, egui::Vec2::splat(TOOLBAR_BUTTON)).on_hover_text(tip);
    if response.hovered() || selected {
        ui.painter().rect_filled(
            response.rect,
            WIDGET_RADIUS,
            if selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.hovered.bg_fill
            },
        );
    }
    paint_centered_icon(PaintCenteredIconInput {
        ui,
        rect: response.rect,
        icon,
        size: TOOLBAR_ICON,
        tint: icon_color(),
    });
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, tip));
    paint_focus(ui, &response);
    response
}

/// Tool-window stripe button: one quiet square, with an accent at selection.
/// `tip` is the hover text (the tool name and its key).
/// `badge` paints a waiting dot in the top-right corner (agent attention).
pub(crate) struct StripeButtonInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub icon: &'a str,
    pub label: &'a str,
    pub tip: &'a str,
    pub selected: bool,
    pub badge: Option<Color32>,
}

pub fn stripe_button(input: StripeButtonInput<'_>) -> egui::Response {
    let StripeButtonInput {
        ui,
        icon,
        label,
        tip,
        selected,
        badge,
    } = input;
    let response = ui
        .allocate_response(egui::vec2(40.0, 40.0), egui::Sense::click())
        .on_hover_text(tip);
    if response.hovered() || selected {
        ui.painter().rect_filled(
            response.rect,
            WIDGET_RADIUS,
            if selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.hovered.bg_fill
            },
        );
    }
    if selected {
        let marker = egui::Rect::from_min_max(
            egui::pos2(response.rect.left(), response.rect.top() + 7.0),
            egui::pos2(response.rect.left() + 2.0, response.rect.bottom() - 7.0),
        );
        ui.painter()
            .rect_filled(marker, RADIUS_XSMALL, ui.visuals().selection.stroke.color);
    }
    paint_centered_icon(PaintCenteredIconInput {
        ui,
        rect: response.rect,
        icon,
        size: TOOLBAR_ICON,
        tint: if selected || response.hovered() {
            icon_color()
        } else {
            ui.visuals().weak_text_color()
        },
    });
    if let Some(dot) = badge {
        ui.painter().circle_filled(
            egui::pos2(response.rect.right() - 7.0, response.rect.top() + 7.0),
            3.5,
            dot,
        );
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    paint_focus(ui, &response);
    response
}

/// Sidebar lists stay wheel/trackpad-scrollable without a visible bar.
pub fn sidebar_scroll(salt: &'static str) -> egui::ScrollArea {
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, true])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
}
