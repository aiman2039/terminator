//! `JetBrains`-style popup lists (code completion, Find Usages): a lifted
//! popup layer with a hairline border, full-width selection rows, and the
//! typed prefix painted in the popup match blue.
use super::super::scale::{MENU_RADIUS, RADIUS_SMALL};
use super::super::theme::{menu_fill, menu_line, menu_muted, menu_text};
use eframe::egui::{self, Color32, FontId, text::LayoutJob};

/// Islands Dark `popup-completion-match-text` (`blue-90`).
const MATCH_DARK: Color32 = Color32::from_rgb(0x53, 0x8A, 0xF9);
/// `JetBrains` light popup match blue.
const MATCH_LIGHT: Color32 = Color32::from_rgb(0x35, 0x74, 0xF0);

/// Frame for a popup list or popup window: the menu layer fill, a 1px
/// border, rounded corners, and the popup shadow.
pub fn popup_frame(style: &egui::Style) -> egui::Frame {
    egui::Frame::popup(style)
        .fill(menu_fill())
        .stroke(egui::Stroke::new(1.0, menu_line()))
        .corner_radius(egui::CornerRadius::same(MENU_RADIUS))
        .inner_margin(egui::Margin::same(4))
}

/// Text colors inside a popup list: popup text and muted secondary text.
pub fn popup_text_style(style: &mut egui::Style) {
    style.visuals.override_text_color = Some(menu_text());
    style.visuals.weak_text_color = Some(menu_muted());
    style.visuals.window_fill = menu_fill();
    style.visuals.window_stroke = egui::Stroke::new(1.0, menu_line());
}

/// Typed-prefix color in popup lists.
pub fn popup_match_color(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode {
        MATCH_DARK
    } else {
        MATCH_LIGHT
    }
}

/// One full-width popup row: allocates `height`, paints the selection (or
/// hover) fill, and returns the row response.
pub fn popup_row(ui: &mut egui::Ui, height: f32, selected: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    if selected || response.hovered() {
        let fill = if selected {
            ui.visuals().selection.bg_fill
        } else {
            menu_line()
        };
        ui.painter().rect_filled(rect, RADIUS_SMALL, fill);
    }
    response
}

/// Char indices of `text` that the typed `query` matched: a
/// case-insensitive prefix first, otherwise a greedy subsequence. Empty
/// when `query` does not match.
pub fn match_positions(text: &str, query: &str) -> Vec<usize> {
    let wanted: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();
    if chars.starts_with(&wanted) {
        return (0..wanted.len()).collect();
    }
    let mut positions = Vec::with_capacity(wanted.len());
    let mut next = wanted.iter().peekable();
    for (index, c) in chars.iter().enumerate() {
        if next.peek().is_some_and(|w| *w == c) {
            positions.push(index);
            next.next();
        }
    }
    if next.peek().is_some() {
        return Vec::new();
    }
    positions
}

/// Inputs for [`highlighted_label`].
pub struct HighlightedLabelInput<'a> {
    pub text: &'a str,
    pub query: &'a str,
    pub font: FontId,
    pub color: Color32,
    pub matched: Color32,
}

/// `text` as one layout job, with the chars matched by `query` in the
/// popup match color.
pub fn highlighted_label(input: HighlightedLabelInput<'_>) -> LayoutJob {
    let HighlightedLabelInput {
        text,
        query,
        font,
        color,
        matched,
    } = input;
    let positions = match_positions(text, query);
    let mut job = LayoutJob::default();
    let mut run = String::new();
    let mut run_tint = color;
    for (index, c) in text.chars().enumerate() {
        let tint = if positions.binary_search(&index).is_ok() {
            matched
        } else {
            color
        };
        if tint != run_tint && !run.is_empty() {
            job.append(&run, 0.0, egui::TextFormat::simple(font.clone(), run_tint));
            run.clear();
        }
        run_tint = tint;
        run.push(c);
    }
    if !run.is_empty() {
        job.append(&run, 0.0, egui::TextFormat::simple(font, run_tint));
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_matches_win_over_subsequence() {
        assert_eq!(match_positions("partition_kind_update", "pu"), [0, 15]);
        assert_eq!(match_positions("pub", "pu"), [0, 1]);
        assert_eq!(match_positions("Pub", "pU"), [0, 1]);
        assert!(match_positions("scope", "xz").is_empty());
        assert!(match_positions("scope", "").is_empty());
    }

    #[test]
    fn highlighted_label_paints_only_matched_chars() {
        let job = highlighted_label(HighlightedLabelInput {
            text: "pub(crate)",
            query: "pu",
            font: FontId::monospace(crate::appearance::FONT_BODY),
            color: Color32::WHITE,
            matched: MATCH_DARK,
        });
        assert_eq!(job.text, "pub(crate)");
        // `pu` in the match color, then `b(crate)` in the plain color.
        let colors: Vec<Color32> = job
            .sections
            .iter()
            .map(|section| section.format.color)
            .collect();
        assert_eq!(colors, [MATCH_DARK, Color32::WHITE]);
        let split = highlighted_label(HighlightedLabelInput {
            text: "partition_kind_update",
            query: "pu",
            font: FontId::monospace(crate::appearance::FONT_BODY),
            color: Color32::WHITE,
            matched: MATCH_DARK,
        });
        // p | artition_kind_ | u | pdate
        assert_eq!(split.sections.len(), 4);
        assert_eq!(job.sections.len(), 2, "one section per color run");
    }

    #[test]
    fn popup_frame_uses_the_menu_layer() {
        let style = egui::Style::default();
        let frame = popup_frame(&style);
        assert_eq!(frame.fill, menu_fill());
        assert_eq!(frame.stroke.color, menu_line());
        assert_eq!(frame.corner_radius, egui::CornerRadius::same(MENU_RADIUS));
    }
}
