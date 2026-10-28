//! Stateless helpers for the source view: selection spans, word matches,
//! key classes and small shapes.
use crate::view::mode_name;
use crate::vim::{Cursor, ModalEngine, VimEngine};
use egui::{Color32, Pos2, Rect};

/// Multi-caret chord (`IntelliJ` occurrence carets).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CaretChord {
    /// Add a caret at the next occurrence.
    AddNext,
    /// Unselect the last added occurrence.
    RemoveLast,
    /// Add carets at every occurrence.
    AddAll,
    /// Escape back to one caret.
    Collapse,
}

pub(super) struct SelectionSpanForLineInput {
    pub a: Cursor,
    pub b: Cursor,
    pub line: usize,
    pub line_len: usize,
    pub inclusive: bool,
}

pub(in crate::view) struct RenderHelpers;

impl RenderHelpers {
    /// Keys a host popup owns while `SourceOptions::popup_keys` is set.
    pub(super) fn popup_key(key: egui::Key) -> bool {
        matches!(
            key,
            egui::Key::ArrowUp
                | egui::Key::ArrowDown
                | egui::Key::Enter
                | egui::Key::Tab
                | egui::Key::Escape
                | egui::Key::PageUp
                | egui::Key::PageDown
        )
    }

    /// Occurrence-caret chord for one key press: Ctrl+G, Ctrl+Shift+G,
    /// Ctrl+Cmd+G on macOS; Alt+J, Alt+Shift+J, Ctrl+Alt+Shift+J elsewhere.
    pub(super) fn caret_chord(
        mac: bool,
        key: egui::Key,
        modifiers: egui::Modifiers,
    ) -> Option<CaretChord> {
        if mac {
            (key == egui::Key::G && modifiers.ctrl && !modifiers.alt)
                .then_some(match (modifiers.shift, modifiers.mac_cmd) {
                    (false, false) => Some(CaretChord::AddNext),
                    (true, false) => Some(CaretChord::RemoveLast),
                    (false, true) => Some(CaretChord::AddAll),
                    _ => None,
                })
                .flatten()
        } else {
            (key == egui::Key::J && modifiers.alt)
                .then_some(match (modifiers.shift, modifiers.ctrl) {
                    (false, false) => Some(CaretChord::AddNext),
                    (true, false) => Some(CaretChord::RemoveLast),
                    (true, true) => Some(CaretChord::AddAll),
                    _ => None,
                })
                .flatten()
        }
    }

    /// Theme color for a diagnostic: errors shout, warnings caution,
    /// lower severities stay quiet.
    pub(super) fn severity_color(
        visuals: &egui::Visuals,
        severity: crate::lsp::Severity,
    ) -> Color32 {
        match severity {
            crate::lsp::Severity::Error => visuals.error_fg_color,
            crate::lsp::Severity::Warning => visuals.warn_fg_color,
            _ => visuals.weak_text_color(),
        }
    }

    /// The menu's command and shift-command shortcut prefixes.
    pub(super) fn menu_modifiers() -> (&'static str, &'static str) {
        if cfg!(target_os = "macos") {
            ("⌘", "⇧⌘")
        } else {
            ("Ctrl+", "Ctrl+Shift+")
        }
    }

    pub(super) fn line_col_ord(c: Cursor) -> (usize, usize) {
        (c.line, c.col)
    }

    /// `reference:line` for the caret (1-based line), or
    /// `reference:first-last` for a selection spanning several lines.
    pub(in crate::view) fn caret_reference(reference: &str, engine: &VimEngine) -> String {
        let caret = engine.cursor().line.saturating_add(1);
        match engine.selection() {
            Some((start, end)) if start.line != end.line => format!(
                "{reference}:{}-{}",
                start.line.saturating_add(1),
                end.line.saturating_add(1)
            ),
            _ => format!("{reference}:{caret}"),
        }
    }

    /// Char span of the selection on one visual line, end-exclusive.
    pub(super) fn selection_span_for_line(
        input: SelectionSpanForLineInput,
    ) -> Option<(usize, usize)> {
        let SelectionSpanForLineInput {
            a,
            b,
            line,
            line_len,
            inclusive,
        } = input;
        if line < a.line || line > b.line {
            return None;
        }
        let start = if line == a.line { a.col } else { 0 };
        let end = if line == b.line && !inclusive {
            b.col.min(line_len)
        } else if line == b.line {
            b.col
                .checked_add(1)
                .map_or(line_len, |end| end.min(line_len))
        } else {
            line_len
        };
        if start >= end {
            return None;
        }
        Some((start, end))
    }

    /// A wavy underline along the bottom of `rect`.
    pub(super) fn paint_squiggle(painter: &egui::Painter, rect: Rect, color: Color32) {
        const STEP: f32 = 2.0;
        let base = rect.bottom() - 2.0;
        let mut points = Vec::new();
        let mut x = rect.left();
        let mut up = false;
        while x <= rect.right() {
            points.push(Pos2::new(x, if up { base - 1.5 } else { base }));
            up = !up;
            x += STEP;
        }
        if points.len() > 1 {
            painter.add(egui::Shape::line(points, egui::Stroke::new(1.0, color)));
        }
    }

    /// Whole-word occurrences of `word` in `text`, as (char start, char len).
    pub(super) fn word_occurrences(text: &str, word: &str) -> Vec<(usize, usize)> {
        let chars: Vec<char> = text.chars().collect();
        let needle: Vec<char> = word.chars().collect();
        let is_word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '_');
        let mut out = Vec::new();
        if needle.is_empty() || needle.len() > chars.len() {
            return out;
        }
        let mut i = 0usize;
        while i.saturating_add(needle.len()) <= chars.len() {
            let end = i.saturating_add(needle.len());
            if chars.get(i..end) == Some(needle.as_slice())
                && !is_word(i.checked_sub(1).and_then(|p| chars.get(p)))
                && !is_word(chars.get(end))
            {
                out.push((i, needle.len()));
                i = end;
            } else {
                i = i.saturating_add(1);
            }
        }
        out
    }
}

/// `-- NORMAL --` style status plus 1-based cursor position.
pub fn status_line(engine: &impl ModalEngine) -> String {
    let cursor = engine.cursor();
    format!(
        "-- {} -- {}:{}",
        mode_name(engine.mode()),
        cursor.line.saturating_add(1),
        cursor.col.saturating_add(1)
    )
}
