use super::rows::row;
use super::theme::{ICON_COLOR, MENU_FILL, MENU_LINE, MENU_MUTED, MENU_TEXT};
use eframe::egui::{self, Color32, FontId, TextStyle};

/// Title-bar sidebar glyph. Resting state has no frame; hover fills only.
pub fn framed_icon(ui: &mut egui::Ui, icon: &str, tip: &str) -> egui::Response {
    framed_icon_button(ui, icon, tip, false, ICON_COLOR)
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
            7.0,
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
    response
}

/// Single-line field. Placeholder and typed text share a vertical center,
/// including when the field is stretched to a toolbar row.
pub fn singleline(text: &mut dyn egui::TextBuffer) -> egui::TextEdit<'_> {
    egui::TextEdit::singleline(text).vertical_align(egui::Align::Center)
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
            .rect_filled(rect, 4.0, ui.visuals().widgets.hovered.bg_fill);
    }
    paint_centered_icon(ui, rect, icon, TOOLBAR_ICON, ICON_COLOR);
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
    egui::InnerResponse::new(inner.map(|shown| shown.inner), response)
}

/// Icon-only selectable control (tabs, toggles). The hover overlay carries the
/// text label and the selected state fills the frame.
pub fn selectable_icon(ui: &mut egui::Ui, icon: &str, tip: &str, selected: bool) -> egui::Response {
    let response = exact_button(ui, egui::Vec2::splat(TOOLBAR_BUTTON)).on_hover_text(tip);
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
    paint_centered_icon(ui, response.rect, icon, TOOLBAR_ICON, ICON_COLOR);
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, tip)
    });
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
    if response.hovered() {
        ui.painter()
            .rect_filled(response.rect, 2, ui.visuals().widgets.hovered.bg_fill);
    }
    let tint = ICON_COLOR;
    let icon = match tool {
        SidebarTool::Explorer => "Files",
        SidebarTool::Agents => "Bell",
        SidebarTool::Git => "GitBranch",
        SidebarTool::History => "History",
        SidebarTool::Info => "Info",
    };
    egui::Image::new(crate::icons::source(icon))
        .tint(tint)
        .paint_at(
            ui,
            egui::Rect::from_center_size(response.rect.center(), egui::vec2(16.0, 16.0)),
        );
    if active {
        ui.painter().line_segment(
            [
                egui::pos2(
                    response.rect.left_bottom().x + 8.0,
                    response.rect.left_bottom().y - 1.0,
                ),
                egui::pos2(
                    response.rect.right_bottom().x - 8.0,
                    response.rect.right_bottom().y - 1.0,
                ),
            ],
            egui::Stroke::new(2.0, tint),
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, active, label)
    });
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
    style.visuals.window_fill = MENU_FILL;
    style.visuals.override_text_color = Some(MENU_TEXT);
    style.visuals.weak_text_color = Some(MENU_MUTED);
    style.visuals.window_stroke = egui::Stroke::new(1.0, MENU_LINE);
    style.visuals.menu_corner_radius = egui::CornerRadius::same(11);
    style.visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 16],
        blur: 34,
        spread: 0,
        color: Color32::from_black_alpha(102),
    };
    style.visuals.widgets.hovered.bg_fill = MENU_LINE;
    style.visuals.widgets.hovered.weak_bg_fill = MENU_LINE;
    style.visuals.widgets.open.bg_fill = MENU_LINE;
    style.visuals.widgets.open.weak_bg_fill = MENU_LINE;
    for widget in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.fg_stroke = egui::Stroke::new(1.0, MENU_TEXT);
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
    crate::diagnostics::record(ui.ctx(), title, response.rect);
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
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if response.hovered() || response.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, 4.0, ui.visuals().widgets.hovered.bg_fill);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(14.0),
        ICON_COLOR,
    );
    let popup = egui::Popup::menu(&response)
        .style(menu_style)
        .show(add_contents);
    egui::InnerResponse::new(popup.map(|shown| shown.inner), response)
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
    crate::diagnostics::record(ui.ctx(), label, response.rect);
    response
}
