//! Gutter and underline marks the host computes for one buffer.
use crate::doc::Buffer;

/// Worst severity on one buffer row, for the diagnostics gutter.
pub struct DiagnosticMark {
    /// Zero-based buffer row.
    pub line: usize,
    pub severity: crate::lsp::Severity,
}

/// How a block of lines differs from the VCS base (gutter markers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    /// Lines removed just above `start` (the block itself is empty).
    Deleted,
}

/// One changed block in buffer lines `start..end` (empty for deletions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangeMark {
    pub start: usize,
    pub end: usize,
    pub kind: ChangeKind,
}

/// A host background behind one buffer line's text area (diff row tint).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineFill {
    pub line: usize,
    pub color: egui::Color32,
}

/// A host background behind char columns `start..end` of one line (diff
/// word marks), painted over any [`LineFill`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RangeFill {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub color: egui::Color32,
}

/// One line's gutter annotation (blame): `text` in the column, `hover`
/// on hover. Lines of one commit share `group`, which alternates the
/// column shading between commits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annotation {
    pub text: String,
    pub hover: String,
    pub group: u32,
}

/// One diagnostic underline: a char-column span on one line, with the
/// message shown when hovered. Multi-line ranges underline their first
/// line to its end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Squiggle {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub severity: crate::lsp::Severity,
    pub message: String,
}

/// Underlines for a buffer's diagnostics, sorted by position. Empty or
/// zero-width ranges widen to the word (or char) they start on, so every
/// diagnostic has something to hover.
#[must_use]
pub fn squiggles<B: Buffer>(doc: &B, diagnostics: &[crate::lsp::Diagnostic]) -> Vec<Squiggle> {
    let mut out: Vec<Squiggle> = diagnostics
        .iter()
        .take(1000)
        .filter_map(|diagnostic| {
            let line = usize::try_from(diagnostic.range.start.line).ok()?;
            if line >= doc.line_count() {
                return None;
            }
            let text = doc.line_text(line);
            let len = text.chars().count();
            let start =
                crate::lsp::utf16_to_char_col(&text, diagnostic.range.start.character).min(len);
            let mut end = if diagnostic.range.end.line == diagnostic.range.start.line {
                crate::lsp::utf16_to_char_col(&text, diagnostic.range.end.character).min(len)
            } else {
                len
            };
            if end <= start {
                let chars: Vec<char> = text.chars().collect();
                end = start;
                while chars
                    .get(end)
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_')
                {
                    end = end.saturating_add(1);
                }
                end = end.max(start.saturating_add(1));
            }
            Some(Squiggle {
                line,
                start,
                end,
                severity: diagnostic.severity.unwrap_or(crate::lsp::Severity::Hint),
                message: diagnostic.message.clone(),
            })
        })
        .collect();
    out.sort_by_key(|squiggle| (squiggle.line, squiggle.start));
    out
}
