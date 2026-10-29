//! Chunk rows: Ours | gutter | Result | gutter | Theirs. Every chunk is
//! one row as tall as its longest side, so the panes stay aligned; only
//! rows inside the viewport are laid out.
use super::super::super::*;
use super::text::{self, ResultSpansInput, SpanCache};
use crate::merge::{Chunk, ChunkKind, MergeDoc, MergeTab, Pick};
use std::path::Path;

/// Gutter width: two icon buttons side by side.
const GUTTER: f32 = 2.0 * appearance::TOOLBAR_BUTTON + 6.0;

#[derive(Clone, Copy)]
pub(super) struct MergeColors {
    conflict: Color32,
    change: Color32,
    same: Color32,
    resolved: Color32,
    text: Color32,
    current: Color32,
}

impl MergeColors {
    pub(super) fn new(theme: &AppearanceConfig, ui: &egui::Ui) -> Self {
        let tint =
            |color: Color32| Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 48);
        Self {
            conflict: tint(appearance::color(&theme.git_deleted)),
            change: tint(appearance::color(&theme.git_modified)),
            same: tint(appearance::color(&theme.git_added)),
            resolved: tint(ui.visuals().weak_text_color()),
            text: ui.visuals().text_color(),
            current: ui.visuals().selection.stroke.color,
        }
    }

    fn fill(self, chunk: &Chunk) -> Color32 {
        if chunk.kind == ChunkKind::Stable {
            Color32::TRANSPARENT
        } else if chunk.resolved {
            self.resolved
        } else {
            match chunk.kind {
                ChunkKind::Conflict => self.conflict,
                ChunkKind::Same => self.same,
                _ => self.change,
            }
        }
    }
}

pub(super) struct MergeBodyInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) key: &'a str,
    pub(super) path: &'a Path,
    pub(super) state: &'a mut MergeTab,
    pub(super) colors: MergeColors,
}

/// Screen columns of one row.
#[derive(Clone, Copy)]
struct Columns {
    ours: egui::Rect,
    left: egui::Rect,
    result: egui::Rect,
    right: egui::Rect,
    theirs: egui::Rect,
}

impl Columns {
    fn new(row: egui::Rect) -> Self {
        let column = ((row.width() - 2.0 * GUTTER) / 3.0).max(1.0);
        let cut = |from: f32, width: f32| {
            egui::Rect::from_min_max(
                egui::pos2(row.left() + from, row.top()),
                egui::pos2(row.left() + from + width, row.bottom()),
            )
        };
        Self {
            ours: cut(0.0, column),
            left: cut(column, GUTTER),
            result: cut(column + GUTTER, column),
            right: cut(2.0 * column + GUTTER, GUTTER),
            theirs: cut(2.0 * (column + GUTTER), column),
        }
    }
}

pub(super) fn merge_body(input: MergeBodyInput<'_>) {
    let MergeBodyInput {
        ui,
        key,
        path,
        state,
        colors,
    } = input;
    let Some(doc) = state.doc.as_mut() else {
        return;
    };
    headers(ui, doc);
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    let heights: Vec<f32> = doc
        .chunks
        .iter()
        .map(|chunk| chunk_height(doc, chunk, row_height))
        .collect();
    let mut area = egui::ScrollArea::vertical()
        .id_salt((key, "merge-rows"))
        .auto_shrink([false; 2]);
    if let Some(index) = state.scroll_to.take() {
        let top: f32 = heights.iter().take(index).sum();
        area = area.vertical_scroll_offset((top - 3.0 * row_height).max(0.0));
    }
    let mut pass = RowPass {
        doc,
        cache: &mut state.highlights,
        current: &mut state.current,
        dirty: &mut state.dirty,
        key,
        path,
        colors,
    };
    area.show_viewport(ui, |ui, viewport| {
        let width = ui.available_width();
        let total: f32 = heights.iter().sum();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, total), egui::Sense::hover());
        let mut top = 0.0f32;
        for (index, height) in heights.iter().enumerate() {
            if top + height >= viewport.min.y && top <= viewport.max.y {
                let row = egui::Rect::from_min_size(
                    egui::pos2(rect.min.x, rect.min.y + top),
                    egui::vec2(width, *height),
                );
                pass.row(ui, index, Columns::new(row));
            }
            top += height;
        }
    });
}

fn headers(ui: &mut egui::Ui, doc: &MergeDoc) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width(),
            ui.text_style_height(&egui::TextStyle::Body),
        ),
        egui::Sense::hover(),
    );
    let columns = Columns::new(rect);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().strong_text_color();
    for (column, label) in [
        (columns.ours, super::side_label(doc, true)),
        (columns.result, "Result".to_owned()),
        (columns.theirs, super::side_label(doc, false)),
    ] {
        ui.painter().with_clip_rect(column).text(
            column.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            font.clone(),
            color,
        );
    }
}

fn chunk_height(doc: &MergeDoc, chunk: &Chunk, row_height: f32) -> f32 {
    let lines = [
        text::line_count(doc.ours.slice(&chunk.ours)),
        text::line_count(doc.theirs.slice(&chunk.theirs)),
        text::line_count(&chunk.result),
        1,
    ]
    .into_iter()
    .max()
    .unwrap_or(1);
    let height = lines as f32 * row_height;
    if chunk.kind == ChunkKind::Stable {
        height
    } else {
        height.max(appearance::TOOLBAR_BUTTON)
    }
}

/// Everything one frame's rows read and change.
struct RowPass<'a> {
    doc: &'a mut MergeDoc,
    cache: &'a mut SpanCache,
    current: &'a mut Option<usize>,
    dirty: &'a mut bool,
    key: &'a str,
    path: &'a Path,
    colors: MergeColors,
}

impl RowPass<'_> {
    fn row(&mut self, ui: &mut egui::Ui, index: usize, columns: Columns) {
        let Some(chunk) = self.doc.chunks.get(index) else {
            return;
        };
        let fill = self.colors.fill(chunk);
        let (ours_changed, theirs_changed) = sides_changed(chunk.kind);
        let change = chunk.kind != ChunkKind::Stable;
        let open = change && !chunk.resolved;
        let painter = ui.painter();
        for (rect, tinted) in [
            (columns.ours, ours_changed),
            (columns.result, change),
            (columns.theirs, theirs_changed),
        ] {
            if tinted {
                painter.rect_filled(rect, 0.0, fill);
            }
        }
        if *self.current == Some(index) {
            painter.rect_stroke(
                columns.result,
                0.0,
                egui::Stroke::new(1.0, self.colors.current),
                egui::StrokeKind::Inside,
            );
        }
        self.side_texts(ui, index, columns);
        self.result_editor(ui, columns.result, index);
        if open {
            self.gutters(ui, index, columns);
        }
    }

    /// Ours and theirs text of chunk `index`, read-only.
    fn side_texts(&self, ui: &egui::Ui, index: usize, columns: Columns) {
        let Some(chunk) = self.doc.chunks.get(index) else {
            return;
        };
        for (rect, side, lines) in [
            (columns.ours, &self.doc.ours, &chunk.ours),
            (columns.theirs, &self.doc.theirs, &chunk.theirs),
        ] {
            let content = side.slice(lines);
            if content.is_empty() {
                continue;
            }
            let font = egui::TextStyle::Monospace.resolve(ui.style());
            let job = text::job(
                content,
                text::side_spans(side, lines),
                font,
                self.colors.text,
            );
            let galley = ui.ctx().fonts_mut(|fonts| fonts.layout_job(job));
            ui.painter()
                .with_clip_rect(rect.intersect(ui.clip_rect()))
                .galley(rect.min, galley, self.colors.text);
        }
    }

    fn gutters(&mut self, ui: &mut egui::Ui, index: usize, columns: Columns) {
        for (rect, left) in [(columns.left, true), (columns.right, false)] {
            self.gutter(ui, index, (rect, left));
        }
    }

    fn result_editor(&mut self, ui: &mut egui::Ui, rect: egui::Rect, index: usize) {
        let Some(chunk) = self.doc.chunks.get(index) else {
            return;
        };
        let spans = text::result_spans(ResultSpansInput {
            doc: self.doc,
            chunk,
            cache: self.cache,
            path: self.path,
        })
        .as_slice()
        .to_vec();
        let font = egui::TextStyle::Monospace.resolve(ui.style());
        let fallback = self.colors.text;
        let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _wrap: f32| {
            let job = text::job(buffer.as_str(), &spans, font.clone(), fallback);
            ui.ctx().fonts_mut(|fonts| fonts.layout_job(job))
        };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        let Some(chunk) = self.doc.chunks.get_mut(index) else {
            return;
        };
        let response = child.add(
            egui::TextEdit::multiline(&mut chunk.result)
                .id(egui::Id::unique((self.key, "merge-result", index)))
                .font(egui::TextStyle::Monospace)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .desired_width(rect.width())
                .desired_rows(1)
                .layouter(&mut layouter),
        );
        if response.changed() {
            self.doc.edited(index);
            *self.dirty = true;
        }
    }

    /// Chunk actions: accept ours / ignore on the left, accept theirs /
    /// both on the right.
    fn gutter(&mut self, ui: &mut egui::Ui, index: usize, (rect, left): (egui::Rect, bool)) {
        let Some(chunk) = self.doc.chunks.get(index) else {
            return;
        };
        let (ours_changed, theirs_changed) = sides_changed(chunk.kind);
        let buttons: &[(&str, &str, Pick)] = match (left, ours_changed, theirs_changed) {
            (true, true, _) => &[
                ("ChevronRight", "Accept ours", Pick::Ours),
                ("X", "Ignore both changes (keep base)", Pick::Base),
            ],
            (false, true, true) => &[
                ("ChevronLeft", "Accept theirs", Pick::Theirs),
                ("Plus", "Accept both: ours, then theirs", Pick::Both),
            ],
            (false, false, true) => &[
                ("ChevronLeft", "Accept theirs", Pick::Theirs),
                ("X", "Ignore the change (keep base)", Pick::Base),
            ],
            _ => &[],
        };
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(egui::vec2(2.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Min)),
        );
        child.spacing_mut().item_spacing.x = 2.0;
        for (icon, tip, pick) in buttons {
            if appearance::sidebar_action(&mut child, icon, tip).clicked() {
                self.doc.accept(index, *pick);
                *self.current = Some(index);
                *self.dirty = true;
            }
        }
    }
}

/// Whether ours and theirs differ from the base in a chunk of `kind`.
fn sides_changed(kind: ChunkKind) -> (bool, bool) {
    match kind {
        ChunkKind::Stable => (false, false),
        ChunkKind::Ours => (true, false),
        ChunkKind::Theirs => (false, true),
        ChunkKind::Same | ChunkKind::Conflict => (true, true),
    }
}
