//! The app's one type scale and one corner-radius scale.
//!
//! Font roles step from [`FONT_BASE`] like `JetBrains` `JBFont` (`small` −2,
//! `medium` −1, `regular`, `h4` +1, `h3` +3, `h2` +5), so a new base moves
//! every role together. The UI size is not a user preference: terminal and
//! editor text sizes are, and they stay outside this scale.
//! [`apply_text_styles`] registers the roles as egui text styles, so
//! `ui.heading()`, `RichText::small()`, and `TextStyle::resolve` agree with
//! the constants. `scale_tests` fails on a literal font size or corner
//! radius anywhere else in `crates/app/src`, apart from its listed
//! exceptions.
use eframe::egui::{self, FontFamily, FontId, TextStyle};

/// Base UI text size: body text, buttons, and monospace.
pub const FONT_BASE: f32 = 13.0;
/// Tool-window stripe captions and equalizer labels.
pub const FONT_MINI: f32 = FONT_BASE - 4.0;
/// Hints, captions, secondary lines, and count or kind badges.
pub const FONT_CAPTION: f32 = FONT_BASE - 2.0;
/// Dense rows, tab and bar titles, menus, status text, and line numbers;
/// egui's [`TextStyle::Small`].
pub const FONT_SMALL: f32 = FONT_BASE - 1.0;
/// Body text, buttons, and monospace; egui's [`TextStyle::Body`].
pub const FONT_BODY: f32 = FONT_BASE;
/// Names that lead a row or a view: project rows and the Player title.
pub const FONT_LARGE: f32 = FONT_BASE + 1.0;
/// Dialog and section headings; egui's [`TextStyle::Heading`] (semibold).
pub const FONT_HEADING: f32 = FONT_BASE + 3.0;
/// Page titles in the main pane.
pub const FONT_TITLE: f32 = FONT_BASE + 5.0;

/// Marks: change markers, pick boxes, underlines, meter and slider parts,
/// gutter buttons, and pane drop or focus outlines.
pub const RADIUS_XSMALL: u8 = 2;
/// Rows, chips, badges, checkboxes, and sidebar drop outlines.
pub const RADIUS_SMALL: u8 = 4;
/// Buttons, fields, toolbar squares, and segmented controls.
pub const RADIUS_MEDIUM: u8 = 6;
/// Windows, cards, menus, and popups.
pub const RADIUS_LARGE: u8 = 10;

/// `JetBrains` New UI geometry: inset rounded panels, softer widgets/menus.
pub const PANEL_RADIUS: u8 = RADIUS_LARGE;
pub const WIDGET_RADIUS: u8 = RADIUS_MEDIUM;
pub const MENU_RADIUS: u8 = RADIUS_LARGE;

/// The bundled Inter Semibold family that headings use.
pub(super) fn semibold() -> FontFamily {
    FontFamily::Name("Semibold".into())
}

/// The app roles as egui text styles.
pub(super) fn apply_text_styles(style: &mut egui::Style) {
    for role in [TextStyle::Body, TextStyle::Button] {
        style
            .text_styles
            .insert(role, FontId::proportional(FONT_BODY));
    }
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(FONT_SMALL));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::new(FONT_HEADING, semibold()));
    style.text_styles.insert(
        TextStyle::Name("Section".into()),
        FontId::new(FONT_BODY, semibold()),
    );
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(FONT_BODY));
}

/// Menus and popups sit one step below the app: body and buttons at
/// [`FONT_SMALL`], small text at [`FONT_CAPTION`].
pub(super) fn apply_menu_text_styles(style: &mut egui::Style) {
    for role in [TextStyle::Body, TextStyle::Button] {
        style
            .text_styles
            .insert(role, FontId::proportional(FONT_SMALL));
    }
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(FONT_CAPTION));
}
