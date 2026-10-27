use super::rows::row;
use super::theme::{
    MENU_RADIUS, WIDGET_RADIUS, icon_color, menu_fill, menu_line, menu_muted, menu_text,
};
use eframe::egui::{self, Color32, FontId, TextStyle};

/// Title-bar sidebar glyph. Resting state has no frame; hover fills only.
pub fn framed_icon(ui: &mut egui::Ui, icon: &str, tip: &str) -> egui::Response {
    framed_icon_button(ui, icon, tip, false, icon_color())
}

/// Same square as [`framed_icon`], with a selected fill and a custom glyph tint.
/// The project header uses one size for hide, Player, Agents, and the overflow menu.
pub fn framed_icon_button(
    ui: &mut egui::Ui,
    icon: &str,
    tip: &str,
    selected: bool,
    tint: Color32,
) -> egui::Response {
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
pub fn paint_centered_icon(ui: &egui::Ui, rect: egui::Rect, icon: &str, size: f32, tint: Color32) {
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
    paint_centered_icon(ui, rect, icon, TOOLBAR_ICON, icon_color());
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

/// Icon-only selectable control (tabs, toggles). The hover overlay carries the
/// text label and the selected state fills the frame.
pub fn selectable_icon(ui: &mut egui::Ui, icon: &str, tip: &str, selected: bool) -> egui::Response {
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
    paint_centered_icon(ui, response.rect, icon, TOOLBAR_ICON, icon_color());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, tip));
    paint_focus(ui, &response);
    response
}

/// Tool-window stripe button: icon over a tiny label, pill when selected.
/// `badge` paints a waiting dot in the top-right corner (agent attention).
pub fn stripe_button(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    selected: bool,
    badge: Option<Color32>,
) -> egui::Response {
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
    paint_centered_icon(
        ui,
        egui::Rect::from_min_size(
            egui::pos2(response.rect.left(), response.rect.top() + 3.0),
            egui::vec2(40.0, 20.0),
        ),
        icon,
        16.0,
        icon_color(),
    );
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

pub fn tool_button(
    ui: &mut egui::Ui,
    tool: crate::preferences::SidebarTool,
    label: &str,
    active: bool,
) -> egui::Response {
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
/// Orca dark menu recipe from `ui/context-menu.tsx` and `.dark` tokens in `main.css`.
pub fn menu_style(style: &mut egui::Style) {
    egui::menu::menu_style(style);
    for role in [TextStyle::Body, TextStyle::Button] {
        style.text_styles.insert(role, FontId::proportional(12.0));
    }
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(11.0));
    style.spacing.menu_margin = egui::Margin::same(4);
    style.visuals.window_fill = menu_fill();
    style.visuals.override_text_color = Some(menu_text());
    style.visuals.weak_text_color = Some(menu_muted());
    style.visuals.window_stroke = egui::Stroke::new(1.0, menu_line());
    style.visuals.menu_corner_radius = egui::CornerRadius::same(MENU_RADIUS);
    style.visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 16],
        blur: 34,
        spread: 0,
        color: Color32::from_black_alpha(102),
    };
    style.visuals.widgets.hovered.bg_fill = menu_line();
    style.visuals.widgets.hovered.weak_bg_fill = menu_line();
    style.visuals.widgets.open.bg_fill = menu_line();
    style.visuals.widgets.open.weak_bg_fill = menu_line();
    for widget in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.fg_stroke = egui::Stroke::new(1.0, menu_text());
    }
}

pub fn context_menu(
    response: &egui::Response,
    add_contents: impl FnOnce(&mut egui::Ui),
) -> Option<egui::InnerResponse<()>> {
    egui::Popup::context_menu(response)
        .style(menu_style)
        .show(add_contents)
}

pub fn menu_button<R>(
    ui: &mut egui::Ui,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let config = egui::menu::MenuConfig::new().style(menu_style);
    let (response, inner) = if egui::menu::is_in_menu(ui) {
        egui::menu::SubMenuButton::new(title).ui(ui, add_contents)
    } else {
        egui::menu::MenuButton::new(title)
            .config(config)
            .ui(ui, add_contents)
    };
    #[cfg(feature = "test-support")]
    crate::diagnostics::record_ui(ui, title, response.rect);
    egui::InnerResponse::new(inner.map(|shown| shown.inner), response)
}

/// Frameless text menu button for the flat sidebar toolbar. Unlike
/// [`menu_button`], it draws no border or background at rest, so it lines up
/// with the icon actions beside it.
pub fn text_menu_button<R>(
    ui: &mut egui::Ui,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = TOOLBAR_BUTTON;
        let visuals = ui.visuals_mut();
        for widget in [&mut visuals.widgets.inactive, &mut visuals.widgets.active] {
            widget.bg_fill = Color32::TRANSPARENT;
            widget.weak_bg_fill = Color32::TRANSPARENT;
            widget.bg_stroke = egui::Stroke::NONE;
        }
        visuals.widgets.hovered.bg_stroke = egui::Stroke::NONE;
        visuals.widgets.open.bg_stroke = egui::Stroke::NONE;
        menu_button(ui, title, add_contents)
    })
    .inner
}

/// Frameless square menu button holding a short text glyph (for example `…`),
/// matching the icon actions beside it.
pub fn compact_menu_button<R>(
    ui: &mut egui::Ui,
    label: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<Option<R>> {
    let size = egui::Vec2::splat(TOOLBAR_BUTTON);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    if response.hovered() || response.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, WIDGET_RADIUS, ui.visuals().widgets.hovered.bg_fill);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(14.0),
        icon_color(),
    );
    let popup = egui::Popup::menu(&response)
        .style(menu_style)
        .show(add_contents);
    egui::InnerResponse::new(popup.map(|shown| shown.inner), response)
}

/// Hover text for an action, with its bound shortcut when there is one.
pub fn shortcut_tip(label: &str, keys: &str) -> String {
    if keys.is_empty() {
        label.to_owned()
    } else {
        format!("{label} ({keys})")
    }
}

/// Flat full-width action row with a fixed icon column and optional shortcut.
pub fn menu_item(ui: &mut egui::Ui, label: &str, icon: &str, shortcut: &str) -> egui::Response {
    ui.set_min_width(220.0);
    ui.spacing_mut().item_spacing.y = 2.0;
    let destructive = label.starts_with("Close ")
        || label.starts_with("Remove ")
        || label.starts_with("Clear ")
        || label.starts_with("Delete");
    let previous = ui.visuals().override_text_color;
    let tint = if destructive {
        ui.visuals().error_fg_color
    } else {
        ui.visuals().weak_text_color()
    };
    if destructive {
        ui.visuals_mut().override_text_color = Some(tint);
    }
    let response = row(ui, label, icon, false, 26.0, shortcut, tint);
    ui.visuals_mut().override_text_color = previous;
    #[cfg(feature = "test-support")]
    crate::diagnostics::record_ui(ui, label, response.rect);
    response
}
