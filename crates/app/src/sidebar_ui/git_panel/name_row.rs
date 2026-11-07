//! The compact `JetBrains` change-tree row: an icon, the colored name, a
//! dim hint on the same line (`2 files`, a folder path), and a dim note at
//! the right edge (diffstat and status letter).
use super::*;

/// Left edge of the name: the icon column plus a gap.
const NAME_LEFT: f32 = 22.0;
/// Gap between the name, its hint, and the trailing note.
const TEXT_GAP: f32 = 6.0;

pub(super) struct NameRowInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) icon: &'a str,
    /// Tint of a monochrome glyph; language file glyphs keep their colors.
    pub(super) icon_tint: Color32,
    pub(super) name: &'a str,
    pub(super) color: Color32,
    pub(super) hint: &'a str,
    pub(super) trailing: &'a str,
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
    trailing: &'a str,
}

impl RowTexts<'_> {
    fn paint(self, ui: &egui::Ui) {
        let painter = ui.painter();
        let weak = ui.visuals().weak_text_color();
        let body = egui::TextStyle::Body.resolve(ui.style());
        let small = egui::FontId::proportional(appearance::FONT_SMALL);
        let center = self.rect.center().y;
        let left = self.rect.left() + NAME_LEFT;
        let mut right = self.rect.right() - 4.0;
        let name = painter.layout_no_wrap(self.name.to_owned(), body.clone(), self.color);
        if !self.trailing.is_empty() {
            let note = painter.layout_no_wrap(self.trailing.to_owned(), small, weak);
            let start = right - note.size().x;
            if start - TEXT_GAP >= left + name.size().x {
                let at = egui::pos2(start, center - note.size().y / 2.0);
                painter.galley(at, note, weak);
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
