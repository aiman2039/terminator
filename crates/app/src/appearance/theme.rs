use eframe::egui::{self, Color32, FontFamily, FontId, TextStyle};
use std::sync::atomic::{AtomicU32, Ordering};
use terminator_core::appearance::{AppearanceConfig, rgb};
use terminator_native_edit::view::{BOLD_FAMILY, ITALIC_FAMILY};

/// Chrome tints that follow the theme: dark themes keep the fixed Orca
/// values below, light themes derive them from the palette. Set by
/// [`apply`]; every viewport shares one theme.
struct Chrome {
    icon: AtomicU32,
    menu_fill: AtomicU32,
    menu_text: AtomicU32,
    menu_muted: AtomicU32,
    menu_line: AtomicU32,
}

/// Keep small navigation/action glyphs legible independently of secondary text.
const DARK_ICON: Color32 = Color32::from_rgb(242, 244, 248);
// No font-atlas cap: the renderer limit (16384 on Metal) applies to every
// viewport. A parent-only `max_texture_side` cannot reach immediate
// (floating) viewports — eframe builds their input without calling the
// app's `raw_input_hook` — so capping only the main window alternates the
// shared font atlas between two sizes every frame and damages its text.

/// Orca dark context menu fill: `--background` `#0a0a0a`, used as the solid
/// stand-in for `dark:bg-[rgba(0,0,0,0.12)]` + `backdrop-blur-2xl`.
const DARK_MENU_FILL: Color32 = Color32::from_rgb(10, 10, 10);
/// Orca `--popover-foreground` `#fafafa`.
const DARK_MENU_TEXT: Color32 = Color32::from_rgb(250, 250, 250);
/// Orca `--muted-foreground` `#a1a1a1` (shortcuts use this at 85% in CSS).
const DARK_MENU_MUTED: Color32 = Color32::from_rgb(161, 161, 161);
/// Orca `dark:border-white/14` and `dark:focus:bg-white/14`.
const DARK_MENU_LINE: Color32 = Color32::from_rgba_premultiplied(36, 36, 36, 36);

const fn packed(color: Color32) -> u32 {
    u32::from_le_bytes(color.to_array())
}

static CHROME: Chrome = Chrome {
    icon: AtomicU32::new(packed(DARK_ICON)),
    menu_fill: AtomicU32::new(packed(DARK_MENU_FILL)),
    menu_text: AtomicU32::new(packed(DARK_MENU_TEXT)),
    menu_muted: AtomicU32::new(packed(DARK_MENU_MUTED)),
    menu_line: AtomicU32::new(packed(DARK_MENU_LINE)),
};

fn load(slot: &AtomicU32) -> Color32 {
    let [r, g, b, a] = slot.load(Ordering::Relaxed).to_le_bytes();
    Color32::from_rgba_premultiplied(r, g, b, a)
}

fn store(slot: &AtomicU32, color: Color32) {
    slot.store(packed(color), Ordering::Relaxed);
}

/// Tint for small navigation and action glyphs.
pub fn icon_color() -> Color32 {
    load(&CHROME.icon)
}
/// Context menu and popup fill.
pub fn menu_fill() -> Color32 {
    load(&CHROME.menu_fill)
}
/// Context menu text.
pub fn menu_text() -> Color32 {
    load(&CHROME.menu_text)
}
/// Context menu secondary text (shortcuts).
pub(super) fn menu_muted() -> Color32 {
    load(&CHROME.menu_muted)
}
/// Context menu border and hover fill.
pub(super) fn menu_line() -> Color32 {
    load(&CHROME.menu_line)
}

/// Icon, menu fill, menu text, menu muted, and menu line colors for `theme`.
fn chrome_for(theme: &AppearanceConfig) -> [Color32; 5] {
    if theme.is_light() {
        [
            color(&theme.text),
            color(&theme.surface),
            color(&theme.text),
            color(&theme.secondary),
            Color32::from_black_alpha(20),
        ]
    } else {
        [
            DARK_ICON,
            DARK_MENU_FILL,
            DARK_MENU_TEXT,
            DARK_MENU_MUTED,
            DARK_MENU_LINE,
        ]
    }
}

fn set_chrome(theme: &AppearanceConfig) {
    let [icon, fill, text, muted, line] = chrome_for(theme);
    store(&CHROME.icon, icon);
    store(&CHROME.menu_fill, fill);
    store(&CHROME.menu_text, text);
    store(&CHROME.menu_muted, muted);
    store(&CHROME.menu_line, line);
}

/// Black or white, whichever reads on `background`.
pub fn contrast_text(background: Color32) -> Color32 {
    let [r, g, b, _] = background.to_array();
    let luma = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
    if luma > 140.0 {
        Color32::from_rgb(20, 20, 22)
    } else {
        Color32::WHITE
    }
}

pub fn color(value: &str) -> Color32 {
    let [r, g, b] = rgb(value).unwrap_or([209, 211, 217]);
    Color32::from_rgb(r, g, b)
}

/// `JetBrains` New UI geometry: inset rounded panels, softer widgets/menus.
pub const PANEL_RADIUS: u8 = 10;
pub const WIDGET_RADIUS: u8 = 6;
pub const MENU_RADIUS: u8 = 10;

/// Flush panel: surface fill and a hairline edge, no gap between sections.
/// Rounding lives on widgets, menus, and popups, not on abutting panels.
pub fn panel_frame(theme: &AppearanceConfig) -> egui::Frame {
    egui::Frame::new()
        .fill(color(&theme.surface))
        .stroke(egui::Stroke::new(theme.border_width, color(&theme.border)))
        .inner_margin(egui::Margin::same(8))
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
            "JetBrains Mono Italic",
            include_bytes!("../../assets/fonts/JetBrainsMono-Italic.ttf").as_slice(),
            FontFamily::Name(ITALIC_FAMILY.into()),
        ),
        (
            "JetBrains Mono Bold",
            include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf").as_slice(),
            FontFamily::Name(BOLD_FAMILY.into()),
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
        FontFamily::Name(BOLD_FAMILY.into()),
        FontFamily::Name(ITALIC_FAMILY.into()),
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
        FontFamily::Name(BOLD_FAMILY.into()),
        FontFamily::Name(ITALIC_FAMILY.into()),
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
    let light = theme.is_light();
    set_chrome(theme);
    super::editor::set_editor_style(theme);
    let mut style = egui::Style {
        visuals: if light {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        },
        ..Default::default()
    };
    apply_text_styles(&mut style);
    apply_visuals(&mut style.visuals, theme);
    style.spacing.scroll = overlay_scroll();
    apply_spacing(&mut style.spacing, theme.compact());
    ctx.set_style_of(egui::Theme::Dark, style);
    ctx.set_theme(egui::Theme::Dark);
}

fn apply_text_styles(style: &mut egui::Style) {
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
}

fn apply_visuals(visuals: &mut egui::Visuals, theme: &AppearanceConfig) {
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    visuals.panel_fill = color(&theme.surface);
    visuals.window_fill = color(&theme.window);
    visuals.extreme_bg_color = color(&theme.surface);
    visuals.faint_bg_color = color(&theme.hover);
    visuals.override_text_color = Some(color(&theme.text));
    visuals.error_fg_color = color(&theme.status_failed);
    visuals.weak_text_color = Some(color(&theme.secondary));
    visuals.selection.bg_fill = color(&theme.selection);
    visuals.selection.stroke = egui::Stroke::new(1.0, color(&theme.accent));
    visuals.window_stroke = egui::Stroke::new(theme.border_width, color(&theme.border));
    visuals.window_corner_radius = egui::CornerRadius::same(PANEL_RADIUS);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.bg_fill = color(&theme.surface);
        widget.weak_bg_fill = color(&theme.surface);
        widget.bg_stroke = egui::Stroke::new(theme.border_width, color(&theme.border));
        widget.fg_stroke = egui::Stroke::new(1.0, color(&theme.text));
        widget.corner_radius = egui::CornerRadius::same(WIDGET_RADIUS);
    }
    visuals.widgets.hovered.bg_fill = color(&theme.hover);
    visuals.widgets.hovered.weak_bg_fill = color(&theme.hover);
    visuals.widgets.active.bg_stroke.color = color(&theme.accent);
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if theme.is_light() { 30 } else { 80 }),
    };
    visuals.popup_shadow = visuals.window_shadow;
    visuals.indent_has_left_vline = false;
    visuals.menu_corner_radius = egui::CornerRadius::same(MENU_RADIUS);
}

/// Shared by every app-owned ScrollArea: overlay handles stay hidden at rest.
fn overlay_scroll() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
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
    }
}

fn apply_spacing(spacing: &mut egui::Spacing, compact: bool) {
    spacing.button_padding = if compact {
        egui::vec2(6.0, 4.0)
    } else {
        egui::vec2(8.0, 5.0)
    };
    spacing.text_edit_width = 220.0;
    spacing.indent = 14.0;
    spacing.item_spacing = if compact {
        egui::vec2(6.0, 6.0)
    } else {
        egui::vec2(8.0, 8.0)
    };
    spacing.interact_size.y = if compact { 24.0 } else { 28.0 };
}

#[cfg(test)]
mod chrome_tests {
    use super::*;

    #[test]
    fn dark_themes_keep_the_fixed_chrome_and_light_themes_follow_the_palette() {
        for dark in [
            AppearanceConfig::intellij_dark(),
            AppearanceConfig::high_contrast(),
        ] {
            assert_eq!(
                chrome_for(&dark),
                [
                    DARK_ICON,
                    DARK_MENU_FILL,
                    DARK_MENU_TEXT,
                    DARK_MENU_MUTED,
                    DARK_MENU_LINE
                ]
            );
        }
        let light = AppearanceConfig::intellij_light();
        let [icon, fill, text, ..] = chrome_for(&light);
        assert_eq!(icon, color(&light.text));
        assert_eq!(fill, color(&light.surface));
        assert_eq!(text, color(&light.text));
    }

    #[test]
    fn packed_colors_round_trip() {
        let slot = AtomicU32::new(0);
        store(&slot, DARK_MENU_LINE);
        assert_eq!(load(&slot), DARK_MENU_LINE);
    }
}
