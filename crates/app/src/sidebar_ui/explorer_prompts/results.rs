//! Explorer Contents results, painted like Find in Files: a file header
//! with its hit count, then one row per hit with a right-aligned line
//! number and the line, its match marked and always in view.
use super::skip_clipped_row;
#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::{App, appearance, files::search::Hit, workspace_ops};
use eframe::egui;
use std::path::{Path, PathBuf};

const ROW_HEIGHT: f32 = 20.0;
/// Right edge of the line-number column, from the row's left edge.
const NUMBER_RIGHT: f32 = 40.0;
/// Characters kept before the first match when a line is cut.
const LEAD: usize = 12;

/// "N results in M files"; `2000+` at the search limit.
pub(crate) fn results_summary(hits: &[Hit]) -> String {
    let mut files: Vec<&Path> = hits.iter().map(|hit| hit.path.as_path()).collect();
    files.dedup();
    let count = if hits.len() >= crate::files::search::MAX_HITS {
        format!("{}+", crate::files::search::MAX_HITS)
    } else {
        hits.len().to_string()
    };
    let noun = if hits.len() == 1 { "result" } else { "results" };
    format!(
        "{count} {noun} in {}",
        appearance::count_noun(files.len(), "file")
    )
}

/// The shown text and the char range of the match in it. A match far into
/// the line starts the text a few characters before it, after `…`.
pub(crate) fn match_preview(
    text: &str,
    query: &str,
    match_case: bool,
) -> (String, Option<(usize, usize)>) {
    let chars: Vec<char> = text.chars().collect();
    let fold = |c: char| {
        if match_case {
            c
        } else {
            c.to_lowercase().next().unwrap_or(c)
        }
    };
    let needle: Vec<char> = query.chars().map(fold).collect();
    let hay: Vec<char> = chars.iter().copied().map(fold).collect();
    let start = (!needle.is_empty() && needle.len() <= hay.len())
        .then(|| {
            hay.windows(needle.len())
                .position(|window| window == needle.as_slice())
        })
        .flatten();
    let Some(start) = start else {
        return (text.to_owned(), None);
    };
    let end = start.saturating_add(needle.len());
    if start <= LEAD {
        return (text.to_owned(), Some((start, end)));
    }
    let from = start.saturating_sub(LEAD);
    let shown: String = std::iter::once('…')
        .chain(chars.iter().skip(from).copied())
        .collect();
    let offset = start.saturating_sub(from).saturating_add(1);
    (shown, Some((offset, offset.saturating_add(needle.len()))))
}

impl App {
    /// Contents search results, grouped by file. Clicking a hit opens that
    /// file at the match.
    pub(crate) fn explorer_results(&mut self, ui: &mut egui::Ui, cwd: &Path) {
        if let Some(error) = &self.explorer_search_error {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
            return;
        }
        if self.explorer_search.is_empty() {
            ui.weak(if self.explorer_search_pending {
                "Searching…"
            } else if self.explorer_query.trim().is_empty() {
                "Type to search in files"
            } else {
                "No results"
            });
            return;
        }
        ui.weak(if self.explorer_search_pending {
            "Searching…".to_owned()
        } else {
            results_summary(&self.explorer_search)
        });
        let root = self.op_root().unwrap_or_else(|| cwd.to_path_buf());
        let query = if self.preferences.explorer_regex {
            String::new()
        } else {
            self.explorer_query.trim().to_owned()
        };
        let match_case = self.preferences.explorer_match_case;
        let mut open: Option<(PathBuf, u32, u32)> = None;
        let hits = &self.explorer_search;
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, hit) in hits.iter().enumerate() {
            let first = index == 0
                || hits
                    .get(index.wrapping_sub(1))
                    .is_none_or(|prev| prev.path != hit.path);
            if first && !skip_clipped_row(ui, ROW_HEIGHT) {
                let count = hits
                    .iter()
                    .skip(index)
                    .take_while(|other| other.path == hit.path)
                    .count();
                self.result_file_row(ui, &root, (&hit.path, count));
            }
            if skip_clipped_row(ui, ROW_HEIGHT) {
                continue;
            }
            let response = self.result_hit_row(ui, hit, (&query, match_case));
            #[cfg(feature = "test-support")]
            diagnostics::record(
                ui.ctx(),
                &format!("search-hit:{}:{}", hit.path.display(), hit.line),
                response.rect,
            );
            if response.clicked() {
                open = Some((hit.path.clone(), hit.line, hit.column));
            }
        }
        if let Some((path, line, character)) = open {
            let at = terminator_native_edit::lsp::Position {
                line: line.saturating_sub(1),
                character,
            };
            self.activate_file_at(ui, &path, at);
        }
    }

    /// The file header: path below the root, then its hit count.
    fn result_file_row(&self, ui: &mut egui::Ui, root: &Path, (path, count): (&Path, usize)) {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::hover(),
        );
        response.on_hover_text(path.display().to_string());
        let painter = ui.painter_at(rect);
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &workspace_ops::relative_display(root, path),
            0.0,
            egui::TextFormat::simple(
                egui::FontId::proportional(appearance::FONT_SMALL),
                ui.visuals().text_color(),
            ),
        );
        job.append(
            &format!("({count})"),
            6.0,
            egui::TextFormat::simple(
                egui::FontId::proportional(appearance::FONT_SMALL),
                appearance::color(&self.theme.secondary),
            ),
        );
        job.wrap.max_width = rect.width();
        job.wrap.max_rows = 1;
        let galley = ui.painter().layout_job(job);
        painter.galley(
            egui::pos2(rect.left() + 4.0, rect.center().y - galley.size().y * 0.5),
            galley,
            ui.visuals().text_color(),
        );
    }

    /// One hit: muted line number, then the line with its match marked.
    fn result_hit_row(
        &self,
        ui: &mut egui::Ui,
        hit: &Hit,
        (query, match_case): (&str, bool),
    ) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::click(),
        );
        let response = response.on_hover_text(format!("{}:{}", hit.path.display(), hit.line));
        if response.hovered() {
            ui.painter().rect_filled(
                rect,
                appearance::RADIUS_SMALL,
                ui.visuals().widgets.hovered.bg_fill,
            );
        }
        appearance::paint_focus(ui, &response);
        let painter = ui.painter_at(rect);
        let mono = egui::FontId::monospace(appearance::FONT_SMALL);
        painter.text(
            egui::pos2(rect.left() + NUMBER_RIGHT, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            hit.line.to_string(),
            mono.clone(),
            ui.visuals().weak_text_color(),
        );
        let (shown, range) = match_preview(&hit.text, query, match_case);
        let plain = egui::TextFormat::simple(mono.clone(), ui.visuals().text_color());
        let found = egui::TextFormat {
            background: appearance::editor_style().palette.find_match,
            ..egui::TextFormat::simple(mono, ui.visuals().strong_text_color())
        };
        let mut job = egui::text::LayoutJob::default();
        let chars: Vec<char> = shown.chars().collect();
        let (start, end) = range.unwrap_or((chars.len(), chars.len()));
        for (part, format) in [
            (0..start, &plain),
            (start..end.min(chars.len()), &found),
            (end.min(chars.len())..chars.len(), &plain),
        ] {
            let text: String = chars.get(part).unwrap_or_default().iter().collect();
            if !text.is_empty() {
                job.append(&text, 0.0, format.clone());
            }
        }
        let left = rect.left() + NUMBER_RIGHT + 8.0;
        job.wrap.max_width = (rect.right() - 4.0 - left).max(0.0);
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

    fn hit(path: &str, line: u32) -> Hit {
        Hit {
            path: path.into(),
            line,
            column: 0,
            text: String::new(),
        }
    }

    #[test]
    fn the_summary_counts_results_and_files() {
        let hits = [hit("/a", 1), hit("/a", 4), hit("/b", 2)];
        assert_eq!(results_summary(&hits), "3 results in 2 files");
        assert_eq!(results_summary(&hits[..1]), "1 result in 1 file");
    }

    #[test]
    fn a_match_far_into_the_line_stays_in_view() {
        let line = format!("{}needle tail", "x".repeat(150));
        let (shown, range) = match_preview(&line, "NEEDLE", false);
        assert!(shown.starts_with('…'));
        let (start, end) = range.unwrap();
        let marked: String = shown.chars().skip(start).take(end - start).collect();
        assert_eq!(marked, "needle");
        assert_eq!(start, LEAD + 1);
        // A case-sensitive search does not mark a different case.
        assert_eq!(match_preview(&line, "NEEDLE", true).1, None);
        // A match near the start keeps the whole line.
        assert_eq!(
            match_preview("let needle", "needle", true),
            ("let needle".into(), Some((4, 10)))
        );
    }
}
