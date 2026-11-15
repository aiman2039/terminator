//! The compact `JetBrains` change-tree row: an icon, the colored name, a
//! dim hint on the same line (`2 files`, a folder path), and a trailer at
//! the right edge: `+n` in the added color, `−n` in the deleted color, and
//! the status letter as a small badge tinted in its Git status color.
use super::*;

/// Left edge of the name: the icon column plus a gap.
const NAME_LEFT: f32 = 22.0;
/// Gap between the name, its hint, and the trailer.
const TEXT_GAP: f32 = 6.0;
/// Gap between the parts of the trailer.
const PART_GAP: f32 = 4.0;
/// Fill of the status badge: its letter color at this opacity.
pub(super) const BADGE_TINT: f32 = 0.16;
/// Side of the square status badge.
const BADGE_SIDE: f32 = 15.0;
/// Room each side of a wide badge letter.
const BADGE_PAD: f32 = 2.0;

/// One part of the trailer, left to right.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum TrailPart {
    /// A count (`+3`, `−2`) in its color.
    Count(String, Color32),
    /// The status letter, painted on a tinted badge.
    Badge(char, Color32),
}

/// `+added`, `−deleted` (only when some lines went), and the status
/// letter as a badge. No counts when the refresh carried no diffstat.
pub(super) fn change_trailer(
    theme: &AppearanceConfig,
    stat: Option<(u32, u32)>,
    letter: char,
) -> Vec<TrailPart> {
    let mut parts = Vec::new();
    if let Some((added, deleted)) = stat {
        parts.push(TrailPart::Count(
            format!("+{added}"),
            appearance::color(&theme.git_added),
        ));
        if deleted > 0 {
            parts.push(TrailPart::Count(
                format!("−{deleted}"),
                appearance::color(&theme.git_deleted),
            ));
        }
    }
    parts.push(TrailPart::Badge(letter, git_color(theme, letter)));
    parts
}

pub(super) struct NameRowInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) icon: &'a str,
    /// Tint of a monochrome glyph; language file glyphs keep their colors.
    pub(super) icon_tint: Color32,
    pub(super) name: &'a str,
    pub(super) color: Color32,
    pub(super) hint: &'a str,
    pub(super) trailing: &'a [TrailPart],
}

/// One full-width clickable row, [`GIT_TREE_ROW_HEIGHT`] tall. Name and
/// hint are separate texts; the trailing note is dropped before the name
/// is cut.
pub(super) fn name_row(input: NameRowInput<'_>) -> egui::Response {
    let NameRowInput {
        ui,
        icon,
        icon_tint,
        name,
        color,
        hint,
        trailing,
    } = input;
    let size = egui::vec2(ui.available_width(), GIT_TREE_ROW_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    if !ui.is_rect_visible(rect) {
        return response;
    }
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            appearance::RADIUS_SMALL,
            ui.visuals().widgets.hovered.bg_fill,
        );
    }
    appearance::paint_focus(ui, &response);
    appearance::paint_status_icon(appearance::PaintStatusIconInput {
        ui,
        rect: egui::Rect::from_center_size(
            egui::pos2(rect.left() + 9.0, rect.center().y),
            egui::vec2(16.0, 16.0),
        ),
        icon,
        tint: icon_tint,
        spin: false,
    });
    RowTexts {
        rect,
        name,
        color,
        hint,
        trailing,
    }
    .paint(ui);
    response
}

struct RowTexts<'a> {
    rect: egui::Rect,
    name: &'a str,
    color: Color32,
    hint: &'a str,
    trailing: &'a [TrailPart],
}

/// A laid-out trailer part and its width.
struct LaidPart {
    galley: std::sync::Arc<egui::Galley>,
    color: Color32,
    badge: bool,
}

impl LaidPart {
    fn width(&self) -> f32 {
        if self.badge {
            BADGE_SIDE.max(self.galley.size().x + 2.0 * BADGE_PAD)
        } else {
            self.galley.size().x
        }
    }

    fn paint(self, painter: &egui::Painter, left: f32, center: f32) {
        let width = self.width();
        if self.badge {
            let badge = egui::Rect::from_min_size(
                egui::pos2(left, center - BADGE_SIDE / 2.0),
                egui::vec2(width, BADGE_SIDE),
            );
            painter.rect_filled(
                badge,
                appearance::RADIUS_XSMALL,
                self.color.gamma_multiply(BADGE_TINT),
            );
        }
        let size = self.galley.size();
        let at = egui::pos2(left + (width - size.x) / 2.0, center - size.y / 2.0);
        painter.galley(at, self.galley, self.color);
    }
}

/// Lays out the trailer parts in [`appearance::FONT_SMALL`]; the badge
/// letter is one step smaller.
fn lay_out_trailer(painter: &egui::Painter, parts: &[TrailPart]) -> Vec<LaidPart> {
    parts
        .iter()
        .map(|part| match part {
            TrailPart::Count(text, color) => LaidPart {
                galley: painter.layout_no_wrap(
                    text.clone(),
                    egui::FontId::proportional(appearance::FONT_SMALL),
                    *color,
                ),
                color: *color,
                badge: false,
            },
            TrailPart::Badge(letter, color) => LaidPart {
                galley: painter.layout_no_wrap(
                    letter.to_string(),
                    egui::FontId::proportional(appearance::FONT_CAPTION),
                    *color,
                ),
                color: *color,
                badge: true,
            },
        })
        .collect()
}

impl RowTexts<'_> {
    fn paint(self, ui: &egui::Ui) {
        let painter = ui.painter();
        let weak = ui.visuals().weak_text_color();
        let body = egui::TextStyle::Body.resolve(ui.style());
        let center = self.rect.center().y;
        let left = self.rect.left() + NAME_LEFT;
        let mut right = self.rect.right() - 4.0;
        let name = painter.layout_no_wrap(self.name.to_owned(), body.clone(), self.color);
        if !self.trailing.is_empty() {
            let parts = lay_out_trailer(painter, self.trailing);
            let width = parts
                .iter()
                .map(|part| part.width() + PART_GAP)
                .sum::<f32>()
                - PART_GAP;
            let start = right - width;
            // The trailer is dropped before the name is cut.
            if start - TEXT_GAP >= left + name.size().x {
                let mut x = start;
                for part in parts {
                    let width = part.width();
                    part.paint(painter, x, center);
                    x += width + PART_GAP;
                }
                right = start - TEXT_GAP;
            }
        }
        let clip = painter.with_clip_rect(egui::Rect::from_min_max(
            egui::pos2(left, self.rect.top()),
            egui::pos2(right.max(left), self.rect.bottom()),
        ));
        let name_end = left + name.size().x;
        clip.galley(
            egui::pos2(left, center - name.size().y / 2.0),
            name,
            self.color,
        );
        if !self.hint.is_empty() && name_end + TEXT_GAP < right {
            let hint = painter.layout_no_wrap(self.hint.to_owned(), body, weak);
            let at = egui::pos2(name_end + TEXT_GAP, center - hint.size().y / 2.0);
            clip.galley(at, hint, weak);
        }
    }
}

/// `crates/app/x.rs` → (`crates/app`, `x.rs`): a flat row shows the name,
/// then its folder as the dim hint (`JetBrains` style).
pub(super) fn split_label(label: &str) -> (&str, &str) {
    label.rsplit_once(['/', '\\']).unwrap_or(("", label))
}

/// `1 file` / `N files`, the dim count after a group or folder name.
pub(super) fn files_hint(count: usize) -> String {
    if count == 1 {
        "1 file".to_owned()
    } else {
        format!("{count} files")
    }
}
