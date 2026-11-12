//! Problems sidebar: workspace-wide language-server diagnostics. Jumps
//! go through the ordinary file-open path, so focus, splits, and the
//! pending-line handoff behave exactly like an Explorer open.

use crate::app::OpenFileInput;
#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::{App, appearance};
use eframe::egui;
use std::path::PathBuf;
use terminator_native_edit::lsp::Severity;

/// Panel title with the manual re-index action on the right. Returns
/// true when the button was clicked.
pub(super) fn lsp_panel_header(ui: &mut egui::Ui, title: &str, tip: &str) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.strong(title);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let button = appearance::sidebar_action(ui, "RefreshCw", tip);
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), &format!("lsp-restart:{title}"), button.rect);
            clicked = button.clicked();
        });
    });
    ui.separator();
    clicked
}

/// The first problem row on screen takes a tool window key's focus
/// (Cmd+6), so arrow keys and Enter work at once; a focused row shows an
/// outline.
fn problem_row_focus(ui: &egui::Ui, response: &egui::Response) {
    crate::shortcuts::tool_row_focus(ui, response);
    appearance::paint_focus(ui, response);
}

/// Sort rank: errors first, then warnings, then the rest.
fn severity_rank(severity: Option<Severity>) -> u8 {
    match severity {
        Some(Severity::Error) => 0,
        Some(Severity::Warning) => 1,
        _ => 2,
    }
}

const ROW_HEIGHT: f32 = 22.0;

/// A file row's name, its folder as a dim hint, and the count at the right.
fn paint_file_label(ui: &egui::Ui, rect: egui::Rect, (name, folder, count): (&str, &str, usize)) {
    let font = |size| egui::FontId::proportional(size);
    let mut job = egui::text::LayoutJob::default();
    job.append(
        name,
        0.0,
        egui::TextFormat::simple(font(appearance::FONT_BODY), ui.visuals().text_color()),
    );
    job.append(
        folder,
        6.0,
        egui::TextFormat::simple(font(appearance::FONT_SMALL), ui.visuals().weak_text_color()),
    );
    job.wrap.max_width = (rect.width() - 80.0).max(0.0);
    job.wrap.max_rows = 1;
    let galley = ui.painter().layout_job(job);
    let painter = ui.painter_at(rect);
    painter.galley(
        egui::pos2(rect.left() + 38.0, rect.center().y - galley.size().y * 0.5),
        galley,
        ui.visuals().text_color(),
    );
    painter.text(
        egui::pos2(rect.right() - 6.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        count.to_string(),
        font(appearance::FONT_SMALL),
        ui.visuals().weak_text_color(),
    );
}

fn errors_only_id() -> egui::Id {
    egui::Id::unique("problems-errors-only")
}

fn collapsed_id(path: &std::path::Path) -> egui::Id {
    egui::Id::unique(("problems-collapsed", path))
}

impl App {
    /// The Problems header: title, Errors only, and restart.
    fn problems_header(ui: &mut egui::Ui) -> (bool, bool) {
        let mut restart = false;
        let mut errors_only = ui.data(|d| d.get_temp(errors_only_id())).unwrap_or(false);
        ui.horizontal(|ui| {
            ui.strong("Problems");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let button = appearance::sidebar_action(
                    ui,
                    "RefreshCw",
                    "Restart language servers and re-index (also retries a missing server)",
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "lsp-restart:Problems", button.rect);
                restart = button.clicked();
                let toggle = appearance::selectable_icon(appearance::SelectableIconInput {
                    ui,
                    icon: "EyeOff",
                    tip: "Errors only",
                    selected: errors_only,
                });
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "problems-errors-only", toggle.rect);
                if toggle.clicked() {
                    errors_only = !errors_only;
                    ui.data_mut(|d| d.insert_temp(errors_only_id(), errors_only));
                }
            });
        });
        ui.separator();
        (restart, errors_only)
    }

    pub(crate) fn problems_panel(&mut self, ui: &mut egui::Ui) {
        let (restart, errors_only) = Self::problems_header(ui);
        if restart {
            self.lsp.restart_all();
        }
        let mut files = self.lsp.all_problems();
        for (_, items) in &mut files {
            if errors_only {
                items.retain(|item| item.severity == Some(Severity::Error));
            }
            items.sort_by_key(|item| severity_rank(item.severity));
        }
        files.retain(|(_, items)| !items.is_empty());
        // Files with errors first; path order otherwise.
        files
            .sort_by_key(|(_, items)| items.first().map_or(2, |item| severity_rank(item.severity)));
        let total: usize = files.iter().map(|(_, items)| items.len()).sum();
        if files.is_empty() {
            ui.weak(if errors_only {
                "No errors reported by language servers."
            } else {
                "No problems reported by language servers."
            });
            return;
        }
        ui.weak(format!(
            "{} in {}",
            appearance::count_noun(total, "problem"),
            appearance::count_noun(files.len(), "file")
        ));
        let root = self.op_root();
        let mut jump: Option<(PathBuf, u32)> = None;
        appearance::sidebar_scroll("problems")
            .max_height(ui.available_height())
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (path, items) in &files {
                    if !self.problem_file_row(ui, path, (root.as_deref(), items.len())) {
                        continue;
                    }
                    for item in items {
                        if self.problem_row(ui, item).clicked() {
                            jump = Some((path.clone(), item.range.start.line.saturating_add(1)));
                        }
                    }
                }
            });
        if let Some((path, line)) = jump {
            self.open_file(OpenFileInput {
                path,
                line: Some(line),
                split: None,
                external: false,
            });
        }
    }

    /// A collapsible file row: chevron, file icon, name, its folder as a
    /// dim hint, and the count. Returns whether the file is expanded.
    fn problem_file_row(
        &self,
        ui: &mut egui::Ui,
        path: &std::path::Path,
        (root, count): (Option<&std::path::Path>, usize),
    ) -> bool {
        let collapsed = ui.data(|d| d.get_temp(collapsed_id(path))).unwrap_or(false);
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::click(),
        );
        let response = response.on_hover_text(path.display().to_string());
        if response.hovered() {
            ui.painter().rect_filled(
                rect,
                appearance::RADIUS_SMALL,
                ui.visuals().widgets.hovered.bg_fill,
            );
        }
        problem_row_focus(ui, &response);
        let icon = |ui: &egui::Ui, x: f32, name: &str, tint| {
            appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
                ui,
                rect: egui::Rect::from_center_size(
                    egui::pos2(rect.left() + x, rect.center().y),
                    egui::Vec2::splat(appearance::TOOLBAR_ICON),
                ),
                icon: name,
                size: appearance::TOOLBAR_ICON,
                tint,
            });
        };
        let chevron = if collapsed {
            "ChevronRight"
        } else {
            "ChevronDown"
        };
        icon(ui, 8.0, chevron, appearance::icon_color());
        icon(
            ui,
            26.0,
            crate::icons::file_icon(path),
            appearance::icon_color(),
        );
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let folder = path
            .parent()
            .map(|dir| {
                root.map_or_else(
                    || dir.display().to_string(),
                    |root| crate::workspace_ops::relative_display(root, dir),
                )
            })
            .unwrap_or_default();
        paint_file_label(ui, rect, (&name, &folder, count));
        if response.clicked() {
            ui.data_mut(|d| d.insert_temp(collapsed_id(path), !collapsed));
        }
        !collapsed
    }

    /// One problem: severity icon, the message's first line, `:line`.
    fn problem_row(
        &self,
        ui: &mut egui::Ui,
        item: &terminator_native_edit::lsp::Diagnostic,
    ) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::click(),
        );
        let response = response.on_hover_text(&item.message);
        if response.hovered() {
            ui.painter().rect_filled(
                rect,
                appearance::RADIUS_SMALL,
                ui.visuals().widgets.hovered.bg_fill,
            );
        }
        problem_row_focus(ui, &response);
        let (glyph, tint) = match item.severity {
            Some(Severity::Error) => ("CircleAlert", appearance::color(&self.theme.status_failed)),
            Some(Severity::Warning) => {
                ("CircleAlert", appearance::color(&self.theme.status_waiting))
            }
            _ => ("Info", appearance::color(&self.theme.secondary)),
        };
        appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
            ui,
            rect: egui::Rect::from_center_size(
                egui::pos2(rect.left() + 44.0, rect.center().y),
                egui::Vec2::splat(appearance::TOOLBAR_ICON),
            ),
            icon: glyph,
            size: appearance::TOOLBAR_ICON,
            tint,
        });
        let line = format!(":{}", item.range.start.line.saturating_add(1));
        let small = egui::FontId::proportional(appearance::FONT_SMALL);
        let painter = ui.painter_at(rect);
        let number = painter.text(
            egui::pos2(rect.right() - 6.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            line,
            small,
            ui.visuals().weak_text_color(),
        );
        let mut job = egui::text::LayoutJob::simple_singleline(
            item.message.lines().next().unwrap_or_default().to_owned(),
            egui::FontId::proportional(appearance::FONT_BODY),
            ui.visuals().text_color(),
        );
        let left = rect.left() + 56.0;
        job.wrap.max_width = (number.left() - 8.0 - left).max(0.0);
        job.wrap.max_rows = 1;
        let galley = ui.painter().layout_job(job);
        painter.galley(
            egui::pos2(left, rect.center().y - galley.size().y * 0.5),
            galley,
            ui.visuals().text_color(),
        );
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_sort_before_warnings_and_hints() {
        let mut ranks = [None, Some(Severity::Warning), Some(Severity::Error)]
            .map(severity_rank)
            .to_vec();
        ranks.sort_unstable();
        assert_eq!(ranks, [0, 1, 2]);
        assert!(severity_rank(Some(Severity::Error)) < severity_rank(Some(Severity::Warning)));
    }
}
