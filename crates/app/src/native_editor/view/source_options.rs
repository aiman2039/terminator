//! The source view options for one frame: the editor style, the per-frame
//! marks drawn over the text, and the problem squiggles.
use super::*;
use crate::native_editor::extras::{DocExtras, SquiggleSets};
use terminator_native_edit::view::ChangeMark;

/// Find matches as `(line, start, end)` for the source view.
pub(super) type FindMatches = Arc<Vec<(usize, usize, usize)>>;

/// Per-frame decorations computed from the buffer for the source view.
pub(super) struct SourceMarks {
    pub(super) mode: markdown::Mode,
    pub(super) read_only: bool,
    pub(super) soft_wrap: bool,
    pub(super) squiggles: SquiggleSets,
    pub(super) inlay_hints: Arc<Vec<terminator_native_edit::inlay::InlayHint>>,
    pub(super) change_marks: Arc<Vec<ChangeMark>>,
    pub(super) popup_keys: bool,
    pub(super) find_matches: FindMatches,
    pub(super) find_current: Option<usize>,
    /// Large-file mode is off (small file, or the user enabled it).
    pub(super) full_features: bool,
    pub(super) status_items: Arc<Vec<String>>,
    /// Text selected in this view's Quick Documentation popup owns Copy.
    pub(super) yield_copy: bool,
    /// A remembered top line to scroll to this frame.
    pub(super) top_line: Option<usize>,
}

/// An editor context menu row, drawn like every other app menu row.
fn menu_item(ui: &mut egui::Ui, label: &str, icon: &str, shortcut: &str) -> egui::Response {
    appearance::menu_item(appearance::MenuItemInput {
        ui,
        label,
        icon,
        shortcut,
    })
}

impl ViewPass<'_> {
    /// Problem squiggles for this frame. A large file gets none.
    pub(super) fn squiggle_sets(
        extras: &mut DocExtras,
        buffer: &Doc,
        frame: &ViewFrame,
        full_features: bool,
    ) -> SquiggleSets {
        if !full_features {
            return SquiggleSets::default();
        }
        extras.squiggles(buffer, (&frame.problems, frame.problems_generation))
    }

    pub(super) fn source_options(
        frame: &ViewFrame,
        decor: SourceDecor,
        marks: SourceMarks,
    ) -> SourceOptions {
        let SourceDecor {
            marks: diagnostics,
            reference,
            annotations,
        } = decor;
        let SourceMarks {
            mode,
            read_only,
            soft_wrap,
            squiggles,
            inlay_hints,
            change_marks,
            popup_keys,
            find_matches,
            find_current,
            full_features,
            status_items,
            yield_copy,
            top_line,
        } = marks;
        let (squiggles, squiggle_tails) = squiggles;
        let style = appearance::editor_style();
        // Markdown keeps its one-point bump over the code font.
        let bump = if frame.is_markdown { 1.0 } else { 0.0 };
        SourceOptions {
            vim: frame.vim,
            read_only,
            menu_style: Some(appearance::menu_style),
            menu_header: Some(appearance::target_header),
            menu_item: Some(menu_item),
            soft_wrap,
            font_size: style.prefs.editor_font_size + bump,
            // Markdown prose reads better with at least 1.3 spacing.
            line_height: if frame.is_markdown {
                style.prefs.editor_line_spacing.max(1.3)
            } else {
                style.prefs.editor_line_spacing
            },
            caret_blink: style.prefs.editor_caret_blink,
            rainbow_brackets: style.prefs.editor_rainbow_brackets,
            jetbrains_keys: style.prefs.editor_jetbrains_keys,
            read_clipboard: Some(crate::clipboard::read_text),
            palette: Some(style.palette),
            status_items,
            large_file_override: full_features,
            error_lens: style.prefs.editor_error_lens,
            squiggle_tails,
            scroll_fraction: if mode == markdown::Mode::Split {
                frame.preview.scroll
            } else {
                None
            },
            top_line,
            diagnostics,
            squiggles,
            find_matches,
            find_current,
            popup_keys,
            change_marks,
            change_colors: frame.change_colors,
            inlay_hints,
            reference: Some(reference),
            annotations,
            blame: frame.blame_offer,
            // Problems and docs under the pointer show in one popup.
            hover_popup: true,
            yield_copy,
            ..SourceOptions::default()
        }
    }
}
