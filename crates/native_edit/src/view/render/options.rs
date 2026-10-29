//! Host-facing options, inputs and results of one source-view frame.
use super::marks::{Annotation, ChangeMark, DiagnosticMark, LineFill, RangeFill, Squiggle};
use super::palette::EditorPalette;
use crate::highlight::{HighlightCache, Language};
use crate::vim::{Effect, VimEngine};
use egui::Color32;

pub type MenuItemRenderer = fn(&mut egui::Ui, &str, &str, &str) -> egui::Response;

pub struct SourceOptions {
    pub font_size: f32,
    /// Row pitch as a multiple of the font's row height (1.0..=2.0);
    /// text stays vertically centered in the taller row.
    pub line_height: f32,
    /// Blink the caret (530 ms on/off) while the view has focus.
    pub caret_blink: bool,
    /// Host theme colors for the editor surface; `None` derives them
    /// from the egui visuals.
    pub palette: Option<EditorPalette>,
    /// Extra status-bar cells after `Ln, Col` (line ending, encoding,
    /// indent), right-aligned like the `JetBrains` status bar.
    pub status_items: Vec<String>,
    pub show_line_numbers: bool,
    pub show_status: bool,
    /// Gutter marks, sorted by row; binary-searched per painted row, so
    /// an empty vec costs nothing on large files.
    pub diagnostics: Vec<DiagnosticMark>,
    /// Underlined diagnostic ranges, sorted (see [`super::squiggles`]); also
    /// drive the scrollbar error stripe and F8 navigation.
    pub squiggles: Vec<Squiggle>,
    /// Find-bar matches `(line, start, end)` in char columns, sorted, and
    /// the index of the selected one.
    pub find_matches: std::sync::Arc<Vec<(usize, usize, usize)>>,
    pub find_current: Option<usize>,
    /// A popup (completion) owns Up/Down/Enter/Tab/Escape: the editor
    /// leaves those keys in the input for the host to read.
    pub popup_keys: bool,
    /// VCS change markers for the gutter, sorted by `start`.
    pub change_marks: std::sync::Arc<Vec<ChangeMark>>,
    /// Theme colors for added / modified / deleted markers.
    pub change_colors: [Color32; 3],
    /// Gutter annotations by buffer line (blame), or `None` for no
    /// column. Lines past the end get a blank cell.
    pub annotations: Option<std::sync::Arc<Vec<Annotation>>>,
    /// Offer "Annotate with Git Blame" in the context menu: `Some(on)`
    /// when the file is under Git, `on` when annotations are showing.
    pub blame: Option<bool>,
    /// The file as "Copy reference" names it (project-relative path):
    /// copies `reference:line` (`IntelliJ` Copy Reference).
    pub reference: Option<String>,
    /// Inlay hints, sorted by line and column (see [`crate::inlay`]).
    pub inlay_hints: std::sync::Arc<Vec<crate::inlay::InlayHint>>,
    /// Take keyboard focus when nothing else holds it, so an opened file is
    /// immediately typeable (`IntelliJ` focuses the editor on open).
    pub autofocus: bool,
    /// Vim mode: Ctrl-V/Ctrl-H belong to vim in its modes, and printable
    /// keys are vim commands in Normal mode. Off is fully modeless:
    /// printable keys (and Enter/Backspace/…) always edit, and Ctrl-V/H
    /// always paste/prompt.
    pub vim: bool,
    /// A filesystem operation owns this buffer; navigation and copying
    /// stay available, while edits and saves wait for its result.
    pub read_only: bool,
    /// Host menu widgets keep the editor menu consistent with other panes.
    pub menu_style: Option<fn(&mut egui::Style)>,
    pub menu_header: Option<fn(&mut egui::Ui, &str, &str)>,
    pub menu_item: Option<MenuItemRenderer>,
    /// Wrap long source lines inside the pane while keeping logical cursor columns.
    pub soft_wrap: bool,
    /// Set the visible position as a fraction of the scrollable document.
    pub scroll_fraction: Option<f32>,
    /// Set the vertical scroll offset in points (a pane scrolled in step
    /// with another one); wins over `scroll_fraction`.
    pub scroll_offset: Option<f32>,
    /// Host filler rows `(line, rows)` before buffer lines, sorted by line
    /// (see [`crate::spacers`]); lines past the end fill after the last.
    pub spacers: std::sync::Arc<Vec<(usize, usize)>>,
    /// Background of filler rows; `None` leaves them blank.
    pub spacer_fill: Option<Color32>,
    /// Full-row backgrounds, sorted by line.
    pub line_fills: std::sync::Arc<Vec<LineFill>>,
    /// Char-range backgrounds, sorted by line and start.
    pub range_fills: std::sync::Arc<Vec<RangeFill>>,
    /// Pin the headers of enclosing blocks at the top while scrolling.
    pub sticky_headers: bool,
}

impl Default for SourceOptions {
    fn default() -> Self {
        Self {
            font_size: 13.0,
            line_height: 1.2,
            caret_blink: true,
            palette: None,
            status_items: Vec::new(),
            show_line_numbers: true,
            show_status: true,
            autofocus: true,
            vim: true,
            read_only: false,
            menu_style: None,
            menu_header: None,
            menu_item: None,
            soft_wrap: false,
            scroll_fraction: None,
            scroll_offset: None,
            spacers: std::sync::Arc::new(Vec::new()),
            spacer_fill: None,
            line_fills: std::sync::Arc::new(Vec::new()),
            range_fills: std::sync::Arc::new(Vec::new()),
            sticky_headers: true,
            diagnostics: Vec::new(),
            squiggles: Vec::new(),
            find_matches: std::sync::Arc::new(Vec::new()),
            find_current: None,
            popup_keys: false,
            change_marks: std::sync::Arc::new(Vec::new()),
            change_colors: [
                Color32::from_rgb(0x4C, 0x9A, 0x52),
                Color32::from_rgb(0x4A, 0x7F, 0xC1),
                Color32::from_rgb(0x8C, 0x8C, 0x8C),
            ],
            inlay_hints: std::sync::Arc::new(Vec::new()),
            reference: None,
            annotations: None,
            blame: None,
        }
    }
}

pub struct SourceOutcome {
    pub effects: Vec<Effect>,
    /// Painted file area, for fixture targeting and reveal requests.
    pub content_rect: egui::Rect,
    /// A primary or secondary click made this source view the input target.
    pub activated: bool,
    /// True while the editor context menu is open.
    pub context_menu_opened: bool,
    /// Rename action geometry for an optional GUI fixture target.
    pub rename_action_rect: Option<egui::Rect>,
    /// Hover action geometry for an optional GUI fixture target.
    pub hover_action_rect: Option<egui::Rect>,
    /// Command/search row, fixed to the pane bottom when enabled.
    pub footer_rect: Option<egui::Rect>,
    /// Current vertical position in the scrollable document, from 0 to 1.
    pub scroll_fraction: f32,
    /// Number of painted rows after optional source wrapping.
    pub visual_rows: usize,
    /// Screen rect of the caret cell when it was painted, for anchoring
    /// popups (completion) at the caret.
    pub caret_rect: Option<egui::Rect>,
    /// A gutter change marker was clicked: its index in
    /// `SourceOptions::change_marks` and where, for the rollback popup.
    pub change_clicked: Option<(usize, egui::Pos2)>,
    /// A gutter annotation was clicked: its buffer line.
    pub annotation_clicked: Option<usize>,
    /// The context menu asked to show or hide blame annotations.
    pub toggle_blame: bool,
    /// The source scroll area this frame: id, offset, viewport and
    /// content size, for hosts that keep another pane in step.
    pub scroll: egui::scroll_area::ScrollAreaOutput<()>,
    /// Painted row pitch, filler rows included.
    pub row_height: f32,
}

pub struct ShowSourceInput<'a, B> {
    pub ui: &'a mut egui::Ui,
    pub doc: &'a mut B,
    pub engine: &'a mut VimEngine,
    pub options: &'a SourceOptions,
    pub highlight: &'a mut HighlightCache,
    pub language: Language,
    pub id: &'a str,
}
