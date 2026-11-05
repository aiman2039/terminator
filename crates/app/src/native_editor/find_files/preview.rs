//! The preview under the Find in Files results: the lines around the
//! cursor's hit (as the search kept them), syntax colored, with the
//! matches highlighted and the hit's line marked.
use super::search::CONTEXT_LINES;
use super::*;
use terminator_native_edit::highlight::{Span, highlight_text};

/// Height of one preview line.
const LINE_HEIGHT: f32 = 17.0;

/// One preview line: 0-based number, text, syntax spans, and the char
/// ranges of its matches.
pub(crate) struct PreviewLine {
    pub number: u32,
    pub text: String,
    pub spans: Vec<Span>,
    pub matches: Vec<(usize, usize)>,
}

/// The cursor's preview, kept until the cursor, results, or theme change.
pub(crate) struct Preview {
    key: (u64, usize, bool),
    pub path: PathBuf,
    /// The hit's 0-based line.
    pub line: u32,
    pub lines: Vec<PreviewLine>,
}

/// The kept lines around `hit`. Each hit line gets its matches, moved
/// from the trimmed list text onto the full line.
pub(crate) fn preview_lines(file: &FileHits, hit: &LineHit) -> Vec<PreviewLine> {
    let from = hit.line.saturating_sub(CONTEXT_LINES);
    let to = hit.line.saturating_add(CONTEXT_LINES);
    file.lines
        .range(from..=to)
        .map(|(&number, text)| {
            let indent = text.chars().take_while(|c| c.is_whitespace()).count();
            let matches = file
                .hits
                .binary_search_by_key(&number, |hit| hit.line)
                .ok()
                .and_then(|at| file.hits.get(at))
                .map(|hit| {
                    hit.matches
                        .iter()
                        .map(|&(start, end)| {
                            (start.saturating_add(indent), end.saturating_add(indent))
                        })
                        .collect()
                })
                .unwrap_or_default();
            PreviewLine {
                number,
                text: text.clone(),
                spans: Vec::new(),
                matches,
            }
        })
        .collect()
}

/// Syntax colors for the preview lines, highlighted as one block.
fn highlight(path: &Path, lines: &mut [PreviewLine], dark: bool) {
    let text = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let language = language_for_path(&path.to_string_lossy());
    let Some(spans) = highlight_text(language, &text, dark) else {
        return;
    };
    for (line, spans) in lines.iter_mut().zip(spans) {
        line.spans = spans;
    }
}

impl FindInFiles {
    /// The cursor's highlighted preview, built again only when it moved.
    pub fn preview(&mut self, dark: bool) -> Option<&Preview> {
        let key = (self.generation, self.selected, dark);
        if self
            .preview
            .as_ref()
            .is_none_or(|preview| preview.key != key)
        {
            let (file, hit) = self.selected_hit()?;
            let mut lines = preview_lines(file, hit);
            highlight(&file.path, &mut lines, dark);
            let built = Preview {
                key,
                path: file.path.clone(),
                line: hit.line,
                lines,
            };
            self.preview = Some(built);
        }
        self.preview.as_ref()
    }
}

/// Colors for [`line_job`]: plain text, and the background of matches.
pub(super) struct JobColors {
    pub plain: egui::Color32,
    pub found: egui::Color32,
}

/// One line as a layout job: syntax colors from `spans` over the plain
/// color, and the `matches` char ranges on the found background.
pub(super) fn line_job(
    text: &str,
    spans: &[Span],
    matches: &[(usize, usize)],
    colors: JobColors,
) -> egui::text::LayoutJob {
    let font = egui::FontId::monospace(appearance::FONT_SMALL);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut cuts: Vec<usize> = spans
        .iter()
        .flat_map(|span| [span.start, span.end])
        .chain(matches.iter().flat_map(|&(start, end)| [start, end]))
        .chain([0, len])
        .map(|at| at.min(len))
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut job = egui::text::LayoutJob::default();
    for pair in cuts.windows(2) {
        let (Some(&from), Some(&to)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let piece: String = chars.get(from..to).unwrap_or_default().iter().collect();
        let color = spans
            .iter()
            .find(|span| span.start <= from && to <= span.end)
            .map_or(colors.plain, |span| {
                let [r, g, b] = span.color;
                egui::Color32::from_rgb(r, g, b)
            });
        let found = matches
            .iter()
            .any(|&(start, end)| start <= from && to <= end);
        let format = egui::TextFormat {
            font_id: font.clone(),
            color,
            background: if found {
                colors.found
            } else {
                egui::Color32::TRANSPARENT
            },
            ..Default::default()
        };
        job.append(&piece, 0.0, format);
    }
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job
}

/// The preview pane: a `path:line` caption and the lines, or a hint.
pub(super) fn preview_pane(ui: &mut egui::Ui, find: &mut FindInFiles) {
    ui.separator();
    let rows = CONTEXT_LINES.saturating_mul(2).saturating_add(1);
    let height = LINE_HEIGHT * rows as f32 + 20.0;
    let root = find.root.clone();
    let dark = ui.visuals().dark_mode;
    ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
        ui.set_min_height(height);
        ui.spacing_mut().item_spacing.y = 0.0;
        let Some(preview) = find.preview(dark) else {
            ui.weak("Pick a result to preview it");
            return;
        };
        ui.weak(format!(
            "{}:{}",
            crate::workspace_ops::relative_display(&root, &preview.path),
            preview.line.saturating_add(1)
        ));
        ui.add_space(4.0);
        for line in &preview.lines {
            preview_row(ui, line, line.number == preview.line);
        }
    });
}

/// One preview line: number, then the colored text; the hit's own line
/// is banded and its matches stand out more.
fn preview_row(ui: &mut egui::Ui, line: &PreviewLine, current: bool) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), LINE_HEIGHT),
        egui::Sense::hover(),
    );
    if !ui.is_rect_visible(rect) {
        return;
    }
    let palette = appearance::editor_style().palette;
    let painter = ui.painter_at(rect);
    if current {
        painter.rect_filled(rect, egui::CornerRadius::ZERO, palette.current_line);
    }
    painter.text(
        egui::pos2(rect.left() + 40.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        line.number.saturating_add(1).to_string(),
        egui::FontId::monospace(appearance::FONT_SMALL),
        if current {
            palette.current_line_number
        } else {
            palette.line_number
        },
    );
    let left = rect.left() + 50.0;
    let colors = JobColors {
        plain: ui.visuals().text_color(),
        found: if current {
            palette.find_current
        } else {
            palette.find_match
        },
    };
    let mut job = line_job(&line.text, &line.spans, &line.matches, colors);
    job.wrap.max_width = (rect.right() - 6.0 - left).max(0.0);
    let galley = ui.painter().layout_job(job);
    painter.galley(
        egui::pos2(left, rect.center().y - galley.size().y * 0.5),
        galley,
        ui.visuals().text_color(),
    );
}
