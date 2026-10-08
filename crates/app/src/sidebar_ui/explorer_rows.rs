use super::git_panel::git_color;
#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::{
    appearance,
    file_actions::{self, FileAction},
    icons,
    preferences::ExplorerSearchMode,
};
use eframe::egui::{self, Color32, RichText};
use std::path::Path;
use terminator_core::appearance::AppearanceConfig;

pub(super) fn skip_clipped_row(ui: &mut egui::Ui, height: f32) -> bool {
    let rect = egui::Rect::from_min_size(
        ui.next_widget_position(),
        egui::vec2(ui.available_width(), height),
    );
    if ui.is_rect_visible(rect) {
        false
    } else {
        // egui scopes and allocate_space each consume one automatic ID.
        ui.allocate_space(rect.size());
        true
    }
}

#[derive(Clone)]
struct CardLayout {
    width: f32,
    style: std::sync::Arc<egui::Style>,
    height: f32,
}

/// Variable-height terminal messages are measured once per content/style/width.
/// A stable outer ID keeps subsequent row controls unchanged when clipped.
pub(super) fn cached_variable_card(
    ui: &mut egui::Ui,
    key: u64,
    draw: impl FnOnce(&mut egui::Ui) -> egui::Rect,
) -> Option<egui::Rect> {
    ui.push_id(("terminal-card", key), |ui| {
        let id = ui.make_persistent_id("height");
        let width = ui.available_width();
        let style = std::sync::Arc::clone(ui.style());
        let previous = ui.ctx().data_mut(|data| data.get_temp::<CardLayout>(id));
        if let Some(previous) = previous
            && previous.width == width
            && (std::sync::Arc::ptr_eq(&previous.style, &style) || previous.style == style)
            && skip_clipped_row(ui, previous.height)
        {
            return None;
        }
        let rect = draw(ui);
        ui.ctx().data_mut(|data| {
            data.insert_temp(
                id,
                CardLayout {
                    width,
                    style,
                    height: rect.height(),
                },
            )
        });
        Some(rect)
    })
    .inner
}

/// Names | Contents segmented control for the Explorer search bar.
/// Small text toggle used by the Contents search bar (`Aa`, whole word, `.*`).
pub(super) fn explorer_toggle(
    ui: &mut egui::Ui,
    text: &str,
    tip: &str,
    selected: bool,
    target: &str,
) -> egui::Response {
    let response = ui
        .add_sized(
            [22.0, 22.0],
            egui::Button::new(RichText::new(text).small()).frame(false),
        )
        .on_hover_text(tip);
    if selected || response.hovered() {
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
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, tip)
    });
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), target, response.rect);
    let _ = target;
    response
}

pub(super) fn explorer_segmented(ui: &mut egui::Ui, mode: &mut ExplorerSearchMode) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 26.0), egui::Sense::hover());
    let base_id = response.id;
    ui.painter()
        .rect_filled(rect, 6, ui.visuals().faint_bg_color);
    let half = rect.width() / 2.0;
    for (index, (label, value, target)) in [
        ("Names", ExplorerSearchMode::Names, "explorer-mode-names"),
        (
            "Contents",
            ExplorerSearchMode::Contents,
            "explorer-mode-contents",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let cell = egui::Rect::from_min_size(
            egui::pos2(
                rect.left() + half * f32::from(u16::try_from(index).unwrap_or(u16::MAX)),
                rect.top(),
            ),
            egui::vec2(half, rect.height()),
        );
        let response = ui
            .interact(cell, base_id.with(label), egui::Sense::click())
            .on_hover_text(label);
        let selected = *mode == value;
        if selected {
            ui.painter()
                .rect_filled(cell.shrink(3.0), 5, ui.visuals().widgets.active.bg_fill);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(cell.shrink(3.0), 5, ui.visuals().widgets.hovered.bg_fill);
        }
        ui.painter().text(
            cell.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.0),
            if selected {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            },
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, label)
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), target, cell);
        let _ = target;
        if response.clicked() {
            *mode = value;
        }
    }
}

/// What an explorer file row asks `App` to do after painting.
pub enum ExplorerLocal {
    Reveal,
    CopyRelative,
    Rename,
    Duplicate,
    Delete,
}

#[derive(Default)]
pub struct ExplorerRowOutcome {
    pub clicked: Option<FileAction>,
    pub menu: Option<FileAction>,
    pub local: Option<ExplorerLocal>,
}

/// Case-insensitive explorer filter over a pre-lowercased query. Directories
/// always pass; files pass when the needle is absent or contained. Hoisting
/// the lowercased query out of the per-file loop avoids one `to_lowercase`
/// allocation per file per scroll frame.
pub(crate) fn explorer_query_keeps_file(
    label: &str,
    directory: bool,
    needle: Option<&str>,
) -> bool {
    if directory {
        return true;
    }
    needle.is_none_or(|needle| label.to_lowercase().contains(needle))
}

#[allow(clippy::too_many_arguments)]
pub fn explorer_file_row(
    ui: &mut egui::Ui,
    path: &Path,
    label: &str,
    status: char,
    ignored: bool,
    theme: &AppearanceConfig,
    open_shortcut: &str,
    split_shortcut: &str,
) -> ExplorerRowOutcome {
    let mut outcome = ExplorerRowOutcome::default();
    let color = if ignored {
        appearance::color(&theme.git_ignored)
    } else {
        git_color(theme, status)
    };
    let r = appearance::file_row(
        ui,
        label,
        icons::file_icon(path),
        false,
        24.0,
        &status.to_string(),
        color,
    );
    // Tooltip text is only needed for the hovered row; formatting it for
    // every row on every scroll frame allocates heavily.
    let r = if r.hovered() {
        r.on_hover_text(format!(
            "{}\n{}",
            path.display(),
            if ignored {
                "Ignored"
            } else {
                terminator_git::status_description(status)
            }
        ))
    } else {
        r
    };
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), &format!("explorer-file:{label}"), r.rect);
    if r.clicked() || r.double_clicked() {
        outcome.clicked = Some(FileAction::Open);
    }
    appearance::context_menu(&r, |ui| {
        if let Some(action) = file_actions::menu_with(
            ui,
            file_actions::FileMenu {
                file: true,
                browser: file_actions::browser_document(path),
                git: None,
                neovim: false,
            },
            open_shortcut,
            split_shortcut,
        ) {
            outcome.menu = Some(action);
        }
        ui.separator();
        if appearance::menu_item(ui, "Reveal in file manager", "FolderOpen", "").clicked() {
            outcome.local = Some(ExplorerLocal::Reveal);
            ui.close();
        }
        if appearance::menu_item(ui, "Copy relative path", "Copy", "").clicked() {
            outcome.local = Some(ExplorerLocal::CopyRelative);
            ui.close();
        }
        ui.separator();
        if appearance::menu_item(ui, "Rename", "Pencil", "").clicked() {
            outcome.local = Some(ExplorerLocal::Rename);
            ui.close();
        }
        if appearance::menu_item(ui, "Duplicate", "Copy", "").clicked() {
            outcome.local = Some(ExplorerLocal::Duplicate);
            ui.close();
        }
        ui.separator();
        if appearance::menu_item(ui, "Delete", "X", "").clicked() {
            outcome.local = Some(ExplorerLocal::Delete);
            ui.close();
        }
    });
    outcome
}

pub(super) fn prompt_button_width(ui: &egui::Ui, label: &str) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER)
        .size()
        .x;
    let padded = text + ui.spacing().button_padding.x + ui.spacing().button_padding.x;
    padded.max(ui.spacing().interact_size.x) + 6.0
}

pub(super) fn paint_header_count(ui: &egui::Ui, rect: egui::Rect, count: usize, color: Color32) {
    let label = if count > 9 {
        "9+".to_string()
    } else {
        count.to_string()
    };
    let galley =
        ui.painter()
            .layout_no_wrap(label, egui::FontId::proportional(9.0), Color32::WHITE);
    let text = galley.size();
    let badge = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 6.0, rect.top() + 7.0),
        egui::vec2(text.x + 4.0, text.y),
    );
    ui.painter().rect_filled(badge, 6.0, color);
    // Waiting gold and the text color are both light, so the digit stays dark.
    ui.painter().galley(
        egui::pos2(
            badge.center().x - text.x * 0.5,
            badge.center().y - text.y * 0.5,
        ),
        galley,
        Color32::from_rgb(20, 20, 22),
    );
}

pub(super) const ATTENTION_ACTION_SIZE: f32 = 22.0;

pub(super) const ATTENTION_ACTION_COUNT: f32 = 3.0;

pub(super) const ATTENTION_ACTION_READ_COUNT: f32 = 4.0;
