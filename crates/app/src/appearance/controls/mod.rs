use super::theme::{WIDGET_RADIUS, icon_color};
use eframe::egui::{self, Color32, FontId};

mod menus;
pub use menus::*;

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
/// The project header uses one size for hide, Player, Agents, and the overflow menu.
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
    let response = ui
        .allocate_response(egui::vec2(28.0, 28.0), egui::Sense::click())
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
    egui::Image::new(crate::icons::source(icon))
        .tint(tint)
        .paint_at(
            ui,
            egui::Rect::from_center_size(response.rect.center(), egui::vec2(14.0, 14.0)),
        );
    accessible_icon(ui, &response, tip);
    response
}

/// Name an icon-only control for screen readers and outline it while it has
/// keyboard focus; the frameless paint otherwise shows hover state only.
fn accessible_icon(ui: &egui::Ui, response: &egui::Response, tip: &str) {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tip));
    paint_focus(ui, response);
}

fn paint_focus(ui: &egui::Ui, response: &egui::Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            WIDGET_RADIUS,
            ui.visuals().selection.stroke,
            egui::StrokeKind::Inside,
        );
    }
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

/// Sidebar toolbar glyph size. Every frameless icon control shares it so the
/// icons sit on one row regardless of the ambient [`egui::Style`] spacing.
pub const TOOLBAR_BUTTON: f32 = 22.0;
const TOOLBAR_ICON: f32 = 14.0;

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

/// Exact-size frameless button for toolbar icons with custom paint, such as
/// tinted status glyphs. Shares the action size so it centers on their row.
pub fn toolbar_button(ui: &mut egui::Ui) -> egui::Response {
    exact_button(ui, egui::Vec2::splat(TOOLBAR_BUTTON))
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

/// Tool-window stripe button: icon over a tiny label, pill when selected.
/// `badge` paints a waiting dot in the top-right corner (agent attention).
pub(crate) struct StripeButtonInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub icon: &'a str,
    pub label: &'a str,
    pub selected: bool,
    pub badge: Option<Color32>,
}

pub fn stripe_button(input: StripeButtonInput<'_>) -> egui::Response {
    let StripeButtonInput {
        ui,
        icon,
        label,
        selected,
        badge,
    } = input;
    let response = ui
        .allocate_response(egui::vec2(40.0, 46.0), egui::Sense::click())
        .on_hover_text(label);
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
        rect: egui::Rect::from_min_size(
            egui::pos2(response.rect.left(), response.rect.top() + 3.0),
            egui::vec2(40.0, 20.0),
        ),
        icon,
        size: 16.0,
        tint: icon_color(),
    });
    ui.painter()
        .with_clip_rect(ui.clip_rect().intersect(response.rect))
        .text(
            egui::pos2(response.rect.center().x, response.rect.bottom() - 11.0),
            egui::Align2::CENTER_CENTER,
            label,
            FontId::proportional(9.0),
            if selected {
                icon_color()
            } else {
                ui.visuals().weak_text_color()
            },
        );
    if let Some(dot) = badge {
        ui.painter().circle_filled(
            egui::pos2(response.rect.right() - 7.0, response.rect.top() + 7.0),
            3.5,
            dot,
        );
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    response
}

/// Sidebar lists stay wheel/trackpad-scrollable without a visible bar.
pub fn sidebar_scroll(salt: &'static str) -> egui::ScrollArea {
    egui::ScrollArea::vertical()
        .id_salt(salt)
        .auto_shrink([false, true])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
}

pub struct ToolButtonInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub tool: crate::preferences::SidebarTool,
    pub label: &'a str,
    pub active: bool,
}

pub fn tool_button(input: ToolButtonInput<'_>) -> egui::Response {
    let ToolButtonInput {
        ui,
        tool,
        label,
        active,
    } = input;
    use crate::preferences::SidebarTool;
    let response = ui
        .add_sized([36.0, 32.0], egui::Button::new("").frame(false))
        .on_hover_text(label);
    if active || response.hovered() {
        ui.painter().rect_filled(
            response.rect,
            WIDGET_RADIUS,
            if active {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.hovered.bg_fill
            },
        );
    }
    let tint = icon_color();
    let icon = match tool {
        SidebarTool::Explorer => "Files",
        SidebarTool::Agents => "Bell",
        SidebarTool::Git => "GitBranch",
        SidebarTool::History => "History",
        SidebarTool::Info => "Info",
        SidebarTool::Problems => "CircleAlert",
        SidebarTool::Structure => "ListTree",
        SidebarTool::Vnc => "Monitor",
        SidebarTool::Hosts => "Globe",
    };
    egui::Image::new(crate::icons::source(icon))
        .tint(tint)
        .paint_at(
            ui,
            egui::Rect::from_center_size(response.rect.center(), egui::vec2(16.0, 16.0)),
        );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, label));
    response
}
