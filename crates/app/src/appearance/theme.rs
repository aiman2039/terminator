use super::scale::{MENU_RADIUS, PANEL_RADIUS, WIDGET_RADIUS, apply_text_styles, semibold};
use eframe::egui::{self, Color32, FontFamily};
use std::sync::atomic::{AtomicU32, Ordering};
use terminator_core::appearance::{AppearanceConfig, rgb};
use terminator_native_edit::view::{BOLD_FAMILY, ITALIC_FAMILY};

/// Chrome tints that follow the theme: popups and menus derive from the
/// palette (a lifted `JetBrains` popup layer on dark themes), dark themes
/// keep the fixed bright icon. Set by [`apply`]; every viewport shares one
/// theme.
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

/// `JetBrains` Islands Dark popup layer (`popup-bg` `#26282C`), used until
/// the first [`apply`] derives the menu tints from the theme.
const DARK_MENU_FILL: Color32 = Color32::from_rgb(0x26, 0x28, 0x2C);
/// Islands `text-default`.
const DARK_MENU_TEXT: Color32 = Color32::from_rgb(0xD1, 0xD3, 0xD9);
/// Islands `text-muted` (shortcuts, details).
const DARK_MENU_MUTED: Color32 = Color32::from_rgb(0x9F, 0xA2, 0xA8);
/// Islands `popup-border` `#33353B`; also the menu hover fill.
const DARK_MENU_LINE: Color32 = Color32::from_rgb(0x33, 0x35, 0x3B);
/// How far a dark popup lifts from the editor color toward the text color:
/// Islands `#191A1C` editor to `#26282C` popup.
const DARK_POPUP_LIFT: f32 = 0.07;
/// The popup border lifts a little further than its fill.
const DARK_POPUP_LINE_LIFT: f32 = 0.12;

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
/// Dark popups sit one layer above the editor, like `JetBrains` popups.
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
        let window = color(&theme.window);
        let text = color(&theme.text);
        [
            DARK_ICON,
            window.lerp_to_gamma(text, DARK_POPUP_LIFT),
            text,
            color(&theme.secondary),
            window.lerp_to_gamma(text, DARK_POPUP_LINE_LIFT),
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

/// Flush island panel: same fill as the main pane (`window`), as in JetBrains
/// Islands; `surface` stays on the frame. Rounding lives on widgets and menus.
pub fn panel_frame(theme: &AppearanceConfig) -> egui::Frame {
    egui::Frame::new()
        .fill(color(&theme.window))
        .stroke(egui::Stroke::new(theme.border_width, color(&theme.border)))
        .inner_margin(egui::Margin::same(8))
}
/// Split panes sit flush with a 1px-style splitter and no per-pane outline.
pub fn dock_style(ui_style: &egui::Style, theme: &AppearanceConfig) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(ui_style);
    style.tab_bar.height = 32.0;
    style.buttons.add_tab_align = egui_dock::style::TabAddAlign::Left;
    style.separator.width = theme.pane_divider_width;
    style.separator.extra_interact_width = 6.0;
    style.separator.color_idle = color(&theme.border);
    style.main_surface_border_rounding = egui::CornerRadius::ZERO;
    style.tab.tab_body.corner_radius = egui::CornerRadius::ZERO;
    style.tab.tab_body.stroke = egui::Stroke::NONE;
    style
}

pub fn install(ctx: &egui::Context) {
    ctx.set_fonts(bundled_fonts());
    egui_extras::install_image_loaders(ctx);
    apply(ctx, &AppearanceConfig::default());
}

/// The bundled font chain: Inter, `JetBrains Mono`, then the Noto fallbacks.
pub(super) fn bundled_fonts() -> egui::FontDefinitions {
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
            semibold(),
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
    let terminal = [
        FontFamily::Monospace,
        FontFamily::Name(BOLD_FAMILY.into()),
        FontFamily::Name(ITALIC_FAMILY.into()),
        FontFamily::Proportional,
    ];
    push_fallback(
        &mut fonts,
        (
            "Noto Sans Symbols 2",
            include_bytes!("../../assets/fonts/NotoSansSymbols2-Subset.ttf"),
        ),
        terminal.clone(),
    );
    push_fallback(
        &mut fonts,
        (
            "Noto Sans Hebrew",
            include_bytes!("../../assets/fonts/NotoSansHebrew-Regular.ttf"),
        ),
        terminal.into_iter().chain([semibold()]),
    );
    fonts
}
/// Append a font as the last fallback of each family.
fn push_fallback(
    fonts: &mut egui::FontDefinitions,
    (name, bytes): (&str, &'static [u8]),
    families: impl IntoIterator<Item = FontFamily>,
) {
    fonts
        .font_data
        .insert(name.into(), egui::FontData::from_static(bytes).into());
    for family in families {
        fonts.families.entry(family).or_default().push(name.into());
    }
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
    fn dark_popups_lift_above_the_editor_and_light_themes_follow_the_palette() {
        for dark in [
            AppearanceConfig::islands_dark(),
            AppearanceConfig::intellij_dark(),
            AppearanceConfig::high_contrast(),
        ] {
            let [icon, fill, text, muted, line] = chrome_for(&dark);
            assert_eq!(icon, DARK_ICON);
            assert_eq!(text, color(&dark.text));
            assert_eq!(muted, color(&dark.secondary));
            let window = color(&dark.window);
            let luma = |c: Color32| u32::from(c.r()) + u32::from(c.g()) + u32::from(c.b());
            assert!(luma(fill) > luma(window), "popup lifts: {dark:?}");
            assert!(luma(line) > luma(fill), "border reads on the popup");
        }
        // Islands Dark: `#191A1C` editor gives the `#26282C` popup layer.
        let [_, fill, _, _, _] = chrome_for(&AppearanceConfig::islands_dark());
        for (got, want) in fill.to_array().iter().zip([0x26u8, 0x28, 0x2C]) {
            assert!(got.abs_diff(want) <= 3, "{fill:?}");
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
