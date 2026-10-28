//! The editable source view. [`show_source`] runs one flat frame:
//! geometry, layout, pointer and keyboard input, painting and
//! scroll-follow, each step in its own file below.
mod canvas;
mod geometry;
mod gutter;
mod helpers;
mod keyboard;
mod marks;
mod menu;
mod options;
mod pointer;
mod row_paint;
mod rows;
mod scroll;
mod sticky;
#[cfg(test)]
mod tests;
mod typing;

#[cfg(test)]
pub(super) use helpers::RenderHelpers;
pub use helpers::status_line;
pub use marks::{Annotation, ChangeKind, ChangeMark, DiagnosticMark, Squiggle, squiggles};
pub use options::{ShowSourceInput, SourceOptions, SourceOutcome};

use crate::doc::Buffer;
use crate::highlight::{HighlightCache, Language};
use crate::view::IdeEdit;
use crate::vim::ModalEngine;
use crate::vim::{Effect, VimEngine};
use geometry::{Geometry, MeasureInput, Viewport};

/// Widest annotation column, in cells.
const ANNOTATION_CELLS: usize = 28;

/// Width of the gutter band that shows and accepts clicks on changes.
const CHANGE_BAND: f32 = 8.0;

/// Render an editable source view. Returns engine effects (`Save`/`Quit`/…)
/// for the session layer to act on. Highlighting derives from the buffer
/// revision into the caller's [`HighlightCache`]: frames without edits
/// repaint from the cache, edits re-derive once.
pub fn show_source<B: Buffer>(input: ShowSourceInput<'_, B>) -> SourceOutcome {
    let ShowSourceInput {
        ui,
        doc,
        engine,
        options,
        highlight,
        language,
        id,
    } = input;
    let geometry = Geometry::measure(MeasureInput {
        ui,
        doc: &*doc,
        options,
        id,
    });
    // Vim off is modeless: IDE caret and selection semantics.
    engine.set_modeless(!options.vim);
    let mut view = SourceView {
        doc,
        engine,
        options,
        geometry: &geometry,
        language,
        id,
        out: FrameOut::new(),
    };
    view.derive_layout(ui, highlight);
    let viewport = Viewport::measure(ui, &geometry, highlight);
    let scroll_output = viewport
        .scroll_area(id, options.scroll_fraction)
        .show(ui, |ui| view.show_frame(ui, &viewport, highlight));
    view.apply_folds(highlight);
    view.paint_squiggle_rail(ui, scroll_output.inner_rect);
    let scroll_fraction = Viewport::scroll_fraction(&scroll_output);
    let footer_rect = view.paint_footer(ui);
    view.out
        .into_outcome(footer_rect, scroll_fraction, viewport.row_count)
}

/// One frame of one source view: the buffer and engine it edits, the
/// host's options, this frame's geometry, and what the frame produced.
struct SourceView<'a, B> {
    doc: &'a mut B,
    engine: &'a mut VimEngine,
    options: &'a SourceOptions,
    geometry: &'a Geometry,
    language: Language,
    id: &'a str,
    out: FrameOut,
}

impl<B: Buffer> SourceView<'_, B> {
    /// Highlight derive, once per buffer revision: unchanged frames repaint
    /// straight from the cache without rebuilding the text. The row layout
    /// is cached on the same key, so an idle frame does no per-line work.
    fn derive_layout(&self, ui: &egui::Ui, highlight: &mut HighlightCache) {
        let doc = &*self.doc;
        let dark = ui.visuals().dark_mode;
        if !highlight.is_current(self.language, dark, doc.revision()) {
            let mut text = String::new();
            for line in 0..doc.line_count() {
                text.push_str(&doc.line_text(line));
                text.push('\n');
            }
            highlight.ensure(crate::highlight::EnsureInput {
                language: self.language,
                dark,
                revision: doc.revision(),
                text: &text,
            });
        }
        highlight.ensure_rows(doc, self.geometry.wrap_columns);
    }

    /// Everything inside the scroll area, in egui call order: allocation,
    /// focus target, pointer input, context menu, focus, row painting,
    /// sticky headers, keyboard input, clipboard sync and scroll-follow.
    fn show_frame(&mut self, ui: &mut egui::Ui, viewport: &Viewport, highlight: &HighlightCache) {
        let canvas = self.open_canvas(ui, viewport, highlight);
        self.register_focus_target(&canvas);
        let caret_before = self.engine.cursor();
        self.handle_clicks(&canvas);
        self.handle_multi_clicks(&canvas);
        self.handle_drag(&canvas);
        self.show_context_menu(&canvas);
        let focused = self.update_focus(&canvas);
        let marks = self.row_marks(&canvas);
        self.paint_rows(&canvas, &marks);
        self.paint_sticky_headers(&canvas);
        self.handle_caret_chords(&canvas, focused);
        self.handle_keyboard(&canvas, focused);
        self.sync_clipboard(&canvas);
        self.follow_caret(&canvas, caret_before);
    }
}

/// What one frame's input and painting produced, folded into
/// [`SourceOutcome`] once the frame ends.
struct FrameOut {
    effects: Vec<Effect>,
    /// Fold changes wait until the row layout is no longer borrowed; they
    /// re-lay the rows next frame.
    fold_actions: Vec<IdeEdit>,
    fold_toggles: Vec<usize>,
    /// Direction of a vertical caret move this frame (`true` = down), so a
    /// caret landing in a folded block hops over it instead of opening it.
    vertical_move: Option<bool>,
    content_rect: egui::Rect,
    activated: bool,
    context_menu_opened: bool,
    rename_action_rect: Option<egui::Rect>,
    hover_action_rect: Option<egui::Rect>,
    caret_rect: Option<egui::Rect>,
    change_clicked: Option<(usize, egui::Pos2)>,
    annotation_clicked: Option<usize>,
    toggle_blame: bool,
}

impl FrameOut {
    fn new() -> Self {
        Self {
            effects: Vec::new(),
            fold_actions: Vec::new(),
            fold_toggles: Vec::new(),
            vertical_move: None,
            content_rect: egui::Rect::NOTHING,
            activated: false,
            context_menu_opened: false,
            rename_action_rect: None,
            hover_action_rect: None,
            caret_rect: None,
            change_clicked: None,
            annotation_clicked: None,
            toggle_blame: false,
        }
    }

    fn into_outcome(
        self,
        footer_rect: Option<egui::Rect>,
        scroll_fraction: f32,
        visual_rows: usize,
    ) -> SourceOutcome {
        let Self {
            effects,
            fold_actions: _,
            fold_toggles: _,
            vertical_move: _,
            content_rect,
            activated,
            context_menu_opened,
            rename_action_rect,
            hover_action_rect,
            caret_rect,
            change_clicked,
            annotation_clicked,
            toggle_blame,
        } = self;
        SourceOutcome {
            effects,
            content_rect,
            activated,
            context_menu_opened,
            rename_action_rect,
            hover_action_rect,
            footer_rect,
            scroll_fraction,
            visual_rows,
            caret_rect,
            change_clicked,
            annotation_clicked,
            toggle_blame,
        }
    }
}
