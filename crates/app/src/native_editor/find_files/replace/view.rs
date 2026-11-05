//! The replace parts of the Find in Files window: each result line of the
//! cursor's file before (−) and after (+) the replace, the last outcome,
//! and the Replace buttons.
use super::super::cursor::keep_visible;
use super::super::preview::{JobColors, line_job};
use super::super::search::CONTEXT_LINES;
use super::*;

/// Height of one changes-pane line, as in the find preview.
const LINE_HEIGHT: f32 = 17.0;

/// Added and removed text colors from the theme, as the rename preview
/// paints them.
#[derive(Clone, Copy)]
pub(crate) struct DiffColors {
    pub added: egui::Color32,
    pub removed: egui::Color32,
}

/// Which Replace button was pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplaceAsk {
    All,
    SelectedFile,
}

/// Where the changes pane looks: the cursor's file and hit (positions in
/// the results), the preview regex, and the list height.
struct ChangesAt {
    file: usize,
    hit: usize,
    regex: (regex::Regex, bool),
    height: f32,
}

/// The pane under the results in replace mode: for the cursor's file,
/// each result line as it is and as the replace leaves it. The cursor's
/// pair is banded; unchecked lines stay dim, as they are.
pub(crate) fn changes_pane(ui: &mut egui::Ui, find: &mut FindInFiles, colors: DiffColors) {
    ui.separator();
    let rows = CONTEXT_LINES.saturating_mul(2).saturating_add(1);
    let height = LINE_HEIGHT * rows as f32 + 20.0;
    ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
        ui.set_min_height(height);
        ui.spacing_mut().item_spacing.y = 0.0;
        let located = find.locate(find.selected);
        let (Some((file, hit)), Some(regex)) = (located, find.preview_regex()) else {
            ui.weak("Pick a result to preview its change");
            return;
        };
        let at = ChangesAt {
            file,
            hit,
            regex,
            height: height - 20.0,
        };
        changes_list(ui, find, at, colors);
    });
}

fn changes_list(ui: &mut egui::Ui, find: &mut FindInFiles, at: ChangesAt, colors: DiffColors) {
    let key = (find.generation, find.selected);
    let reveal = find.replace.shown_for != Some(key);
    find.replace.shown_for = Some(key);
    let Some(file) = find.files.get(at.file) else {
        return;
    };
    let replace = &find.replace;
    let counted = file
        .hits
        .iter()
        .filter(|hit| replace.line_included(&file.path, hit.line))
        .count();
    ui.weak(format!(
        "{}: {counted} of {} lines change",
        crate::workspace_ops::relative_display(&find.root, &file.path),
        file.hits.len()
    ));
    ui.add_space(4.0);
    let pair = LINE_HEIGHT * 2.0;
    let mut area = egui::ScrollArea::vertical()
        .id_salt("find-in-files-changes")
        .max_height(at.height)
        .auto_shrink([false, false]);
    if reveal {
        let top = at.hit as f32 * pair;
        area = area.vertical_scroll_offset(keep_visible(top, pair, at.height, replace.scroll));
    }
    let how = LineReplace {
        regex: &at.regex.0,
        replacement: &replace.text,
        expand: at.regex.1,
    };
    let rows = file.hits.len().saturating_mul(2);
    let output = area.show_rows(ui, LINE_HEIGHT, rows, |ui, range| {
        for row in range {
            let (Some(index), Some(after)) = (row.checked_div(2), row.checked_rem(2)) else {
                continue;
            };
            let Some(hit) = file.hits.get(index) else {
                continue;
            };
            let text = file.lines.get(&hit.line).unwrap_or(&hit.preview);
            let line = ChangeRow {
                number: hit.line,
                text,
                after: after == 1,
                included: replace.line_included(&file.path, hit.line),
                current: index == at.hit,
            };
            change_row(ui, line, &how, colors);
        }
    });
    find.replace.scroll = output.state.clamped_offset().y;
}

/// One changes-pane line: a result line before the replace, or after it.
struct ChangeRow<'a> {
    number: u32,
    text: &'a str,
    after: bool,
    included: bool,
    current: bool,
}

fn change_row(ui: &mut egui::Ui, row: ChangeRow<'_>, how: &LineReplace<'_>, colors: DiffColors) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), LINE_HEIGHT),
        egui::Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let palette = appearance::editor_style().palette;
    let weak = ui.visuals().weak_text_color();
    let painter = ui.painter_at(rect);
    if row.current {
        painter.rect_filled(rect, egui::CornerRadius::ZERO, palette.current_line);
    }
    let change = change_line(row.text, how);
    let (text, marks, plain) = match (row.after, row.included) {
        (false, true) => (row.text, change.old_marks, colors.removed),
        (true, true) => (change.new.as_str(), change.new_marks, colors.added),
        (_, false) => (row.text, Vec::new(), weak),
    };
    let number = if row.after {
        String::new()
    } else {
        row.number.saturating_add(1).to_string()
    };
    let center = rect.center().y;
    let font = egui::FontId::monospace(appearance::FONT_SMALL);
    painter.text(
        egui::pos2(rect.left() + 40.0, center),
        egui::Align2::RIGHT_CENTER,
        number,
        font.clone(),
        palette.line_number,
    );
    let marker = if row.after { "+" } else { "−" };
    painter.text(
        egui::pos2(rect.left() + 46.0, center),
        egui::Align2::LEFT_CENTER,
        marker,
        font,
        plain,
    );
    let left = rect.left() + 58.0;
    let found = palette.find_match;
    let mut job = line_job(text, &[], &marks, JobColors { plain, found });
    job.wrap.max_width = (rect.right() - 6.0 - left).max(0.0);
    let galley = ui.painter().layout_job(job);
    painter.galley(
        egui::pos2(left, center - galley.size().y * 0.5),
        galley,
        plain,
    );
}

/// The last outcome, then the count (or progress) and the Replace
/// buttons. Returns the button pressed.
pub(crate) fn replace_buttons(
    ui: &mut egui::Ui,
    find: &FindInFiles,
    colors: DiffColors,
) -> Option<ReplaceAsk> {
    if let Some(note) = &find.replace.note {
        let color = if note.error {
            ui.visuals().error_fg_color
        } else {
            colors.added
        };
        ui.colored_label(color, &note.text);
    }
    let blocked = find.replace_blocked();
    let why = blocked.unwrap_or_default();
    let mut ask = None;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let all = ui
                .add_enabled(blocked.is_none(), egui::Button::new("Replace All"))
                .on_disabled_hover_text(why);
            let file = ui
                .add_enabled(
                    blocked.is_none() && find.selected_hit().is_some(),
                    egui::Button::new("Replace in Selected File"),
                )
                .on_disabled_hover_text(why);
            if all.clicked() {
                ask = Some(ReplaceAsk::All);
            }
            if file.clicked() {
                ask = Some(ReplaceAsk::SelectedFile);
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                if find.replace.busy() {
                    ui.spinner();
                    ui.weak("Replacing…");
                } else {
                    ui.weak(change_count(find));
                }
            });
        });
    });
    ask
}

/// "3 lines in 2 files change": what Replace All would touch.
pub(crate) fn change_count(find: &FindInFiles) -> String {
    let (lines, files) = find.included_counts();
    format!(
        "{lines} line{} in {files} file{} change",
        if lines == 1 { "" } else { "s" },
        if files == 1 { "" } else { "s" },
    )
}
