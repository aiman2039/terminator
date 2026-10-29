//! Embeddable native source view over [`crate::doc::Buffer`].
mod keys;
mod render;
#[cfg(test)]
mod view_tests;
pub use keys::{
    IdeAction, IdeEdit, IdeEditKeyInput, ModeColors, command_prompt, comment_token,
    ctrl_h_is_motion, ctrl_v_is_block, echo_of_handled_key, egui_key, ide_edit_in_vim,
    ide_edit_key, ide_navigation_shortcut, ide_shortcut, mode_color, mode_name, source_focus_id,
};
pub use render::{
    Annotation, ChangeKind, ChangeMark, DiagnosticMark, EditorPalette, LineFill, RangeFill,
    ShowSourceInput, SourceOptions, SourceOutcome, Squiggle, show_source, source_row_height,
    squiggles, status_line,
};

/// Display layout helpers, re-exported for hosts that map pointer
/// positions onto buffer columns.
pub use crate::rows::{VisualRow, char_display_width, col_at_display_x, display_x, visual_rows};

/// Font family the host registers for italic theme spans.
pub const ITALIC_FAMILY: &str = "Terminal Italic";
/// Font family the host registers for bold theme spans.
pub const BOLD_FAMILY: &str = "Terminal Bold";

/// Whether the keyboard target belongs to a registered source view, including splits.
#[must_use]
pub fn source_has_focus(ctx: &egui::Context) -> bool {
    ctx.memory(|memory| memory.focused()).is_some_and(|id| {
        ctx.data(|data| data.get_temp::<bool>(id.with("native-source")) == Some(true))
    })
}

/// Symbol occurrences as `(line, start, end)` char columns, sorted.
pub type SymbolRanges = std::sync::Arc<Vec<(usize, usize, usize)>>;

fn symbol_ranges_id(id: &str) -> egui::Id {
    egui::Id::unique(("native-symbol-highlights", id))
}

/// Publish language-server document highlights for the source view `id`.
/// They replace the plain caret-word match while one of them holds the
/// caret; `None` (or ranges away from the caret) keeps the local match.
pub fn set_symbol_highlights(ctx: &egui::Context, id: &str, ranges: Option<SymbolRanges>) {
    ctx.data_mut(|data| match ranges {
        Some(ranges) => {
            data.insert_temp(symbol_ranges_id(id), ranges);
        }
        None => data.remove::<SymbolRanges>(symbol_ranges_id(id)),
    });
}

/// Published highlights for view `id`, only while one contains `cursor`
/// (a stale answer for a symbol the caret left never paints).
pub(crate) fn caret_symbol_highlights(
    ctx: &egui::Context,
    id: &str,
    cursor: crate::vim::Cursor,
) -> Option<SymbolRanges> {
    let ranges = ctx.data(|data| data.get_temp::<SymbolRanges>(symbol_ranges_id(id)))?;
    ranges
        .iter()
        .any(|(line, start, end)| {
            *line == cursor.line && *start <= cursor.col && cursor.col <= *end
        })
        .then_some(ranges)
}
