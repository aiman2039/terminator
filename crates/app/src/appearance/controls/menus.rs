//! Context and dropdown menus in the Orca recipe.
use super::super::rows::{RowInput, row};
use super::super::theme::{
    MENU_RADIUS, WIDGET_RADIUS, icon_color, menu_fill, menu_line, menu_muted, menu_text,
};
use super::TOOLBAR_BUTTON;
use eframe::egui::{self, Color32, FontId, TextStyle};

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

pub struct MenuItemInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub label: &'a str,
    pub icon: &'a str,
    pub shortcut: &'a str,
}

/// Flat full-width action row with a fixed icon column and optional shortcut.
pub fn menu_item(input: MenuItemInput<'_>) -> egui::Response {
    let MenuItemInput {
        ui,
        label,
        icon,
        shortcut,
    } = input;
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
    let response = row(RowInput {
        ui,
        label,
        icon,
        selected: false,
        height: 26.0,
        trailing: shortcut,
        tint,
    });
    ui.visuals_mut().override_text_color = previous;
    #[cfg(feature = "test-support")]
    crate::diagnostics::record_ui(ui, label, response.rect);
    response
}
