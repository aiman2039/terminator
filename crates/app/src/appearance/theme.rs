use eframe::egui::{self, Color32, FontFamily, FontId, TextStyle};
use terminator_core::appearance::{AppearanceConfig, rgb};

/// Keep small navigation/action glyphs legible independently of secondary text.
pub const ICON_COLOR: Color32 = Color32::from_rgb(242, 244, 248);

// No font-atlas cap: the renderer limit (16384 on Metal) applies to every
// viewport. A parent-only `max_texture_side` cannot reach immediate
// (floating) viewports — eframe builds their input without calling the
// app's `raw_input_hook` — so capping only the main window alternates the
// shared font atlas between two sizes every frame and damages its text.

/// Orca dark context menu fill: `--background` `#0a0a0a`, used as the solid
/// stand-in for `dark:bg-[rgba(0,0,0,0.12)]` + `backdrop-blur-2xl`.
pub const MENU_FILL: Color32 = Color32::from_rgb(10, 10, 10);
/// Orca `--popover-foreground` `#fafafa`.
pub const MENU_TEXT: Color32 = Color32::from_rgb(250, 250, 250);
/// Orca `--muted-foreground` `#a1a1a1` (shortcuts use this at 85% in CSS).
pub(super) const MENU_MUTED: Color32 = Color32::from_rgb(161, 161, 161);
/// Orca `dark:border-white/14` and `dark:focus:bg-white/14`.
pub(super) const MENU_LINE: Color32 = Color32::from_rgba_premultiplied(36, 36, 36, 36);

pub fn color(value: &str) -> Color32 {
    let [r, g, b] = rgb(value).unwrap_or([209, 211, 217]);
    Color32::from_rgb(r, g, b)
}
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes, family) in [
        (
            "Inter",
            include_bytes!("../../assets/fonts/Inter-Regular.ttf").as_slice(),
            FontFamily::Proportional,
        ),
        (
            "Inter Semibold",
            include_bytes!("../../assets/fonts/Inter-SemiBold.ttf").as_slice(),
            FontFamily::Name("Semibold".into()),
        ),
        (
            "JetBrains Mono",
            include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf").as_slice(),
            FontFamily::Monospace,
        ),
        (
            "JetBrains Mono Bold",
            include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf").as_slice(),
            FontFamily::Name("Terminal Bold".into()),
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, name.into());
    }
    fonts.font_data.insert(
        "Noto Sans Symbols 2 Braille".into(),
        egui::FontData::from_static(include_bytes!(
            "../../assets/fonts/NotoSansSymbols2-Braille.ttf"
        ))
        .into(),
    );
    for family in [
        FontFamily::Monospace,
        FontFamily::Name("Terminal Bold".into()),
    ] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("Noto Sans Symbols 2 Braille".into());
    }
    fonts.font_data.insert(
        "Noto Sans Hebrew".into(),
        egui::FontData::from_static(include_bytes!(
            "../../assets/fonts/NotoSansHebrew-Regular.ttf"
        ))
        .into(),
    );
    for family in [
        FontFamily::Monospace,
        FontFamily::Name("Terminal Bold".into()),
        FontFamily::Proportional,
        FontFamily::Name("Semibold".into()),
    ] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("Noto Sans Hebrew".into());
    }
    ctx.set_fonts(fonts);
    egui_extras::install_image_loaders(ctx);
    apply(ctx, &AppearanceConfig::default());
}
pub fn apply(ctx: &egui::Context, theme: &AppearanceConfig) {
    let mut style = egui::Style {
        visuals: egui::Visuals::dark(),
        ..Default::default()
    };
    for role in [TextStyle::Body, TextStyle::Button] {
        style.text_styles.insert(role, FontId::proportional(13.0));
    }
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(12.0));
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(16.0, FontFamily::Name("Semibold".into())),
    );
    style.text_styles.insert(
        TextStyle::Name("Section".into()),
        FontId::new(13.0, FontFamily::Name("Semibold".into())),
    );
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(13.0));
    style.visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    style.visuals.panel_fill = color(&theme.surface);
    style.visuals.window_fill = color(&theme.window);
    style.visuals.extreme_bg_color = color(&theme.surface);
    style.visuals.faint_bg_color = color(&theme.hover);
    style.visuals.override_text_color = Some(color(&theme.text));
    style.visuals.error_fg_color = color(&theme.status_failed);
    style.visuals.weak_text_color = Some(color(&theme.secondary));
    style.visuals.selection.bg_fill = color(&theme.selection);
    style.visuals.selection.stroke = egui::Stroke::new(1.0, color(&theme.accent));
    style.visuals.window_stroke = egui::Stroke::new(theme.border_width, color(&theme.border));
    style.visuals.window_corner_radius = egui::CornerRadius::same(3);
    for widget in [
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.bg_fill = color(&theme.surface);
        widget.weak_bg_fill = color(&theme.surface);
        widget.bg_stroke = egui::Stroke::new(theme.border_width, color(&theme.border));
        widget.fg_stroke = egui::Stroke::new(1.0, color(&theme.text));
        widget.corner_radius = egui::CornerRadius::same(3);
    }
    style.visuals.widgets.hovered.bg_fill = color(&theme.hover);
    style.visuals.widgets.hovered.weak_bg_fill = color(&theme.hover);
    style.visuals.widgets.active.bg_stroke.color = color(&theme.accent);
    style.visuals.window_shadow = egui::epaint::Shadow::NONE;
    style.visuals.popup_shadow = egui::epaint::Shadow::NONE;
    style.visuals.indent_has_left_vline = false;
    style.visuals.menu_corner_radius = egui::CornerRadius::same(4);
    // Shared by every app-owned ScrollArea: overlay handles stay hidden at rest.
    style.spacing.scroll = egui::style::ScrollStyle {
        floating: true,
        bar_width: 8.0,
        floating_width: 4.0,
        floating_allocated_width: 0.0,
        handle_min_length: 28.0,
        bar_inner_margin: 3.0,
        bar_outer_margin: 2.0,
        foreground_color: true,
        dormant_background_opacity: 0.0,
        active_background_opacity: 0.0,
        interact_background_opacity: 0.0,
        dormant_handle_opacity: 0.0,
        active_handle_opacity: 0.0,
        interact_handle_opacity: 0.0,
        ..Default::default()
    };
    let compact = theme.compact();
    style.spacing.button_padding = if compact {
        egui::vec2(6.0, 4.0)
    } else {
        egui::vec2(8.0, 5.0)
    };
    style.spacing.text_edit_width = 220.0;
    style.spacing.indent = 14.0;
    style.spacing.item_spacing = if compact {
        egui::vec2(6.0, 6.0)
    } else {
        egui::vec2(8.0, 8.0)
    };
    style.spacing.interact_size.y = if compact { 24.0 } else { 28.0 };
    ctx.set_style_of(egui::Theme::Dark, style);
    ctx.set_theme(egui::Theme::Dark);
}
