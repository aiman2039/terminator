//! Matching and replacing for the find bar: scope from the selection,
//! regex matches in char columns, stepping, and replace as one undo step.
use super::*;

/// The selection as buffer char offsets `[from, to)`, spanning the text
/// a copy would take (linewise selections cover whole lines).
pub(crate) fn selection_scope(engine: &VimEngine, buffer: &Doc) -> Option<(usize, usize)> {
    let (from, _) = engine.selection()?;
    let col = if engine.mode() == terminator_native_edit::vim::Mode::VisualLine {
        0
    } else {
        from.col
    };
    let start = buffer.char_at_line_col(from.line, col);
    let len = engine.copy_text(buffer).chars().count();
    (len > 0).then(|| (start, start.saturating_add(len).min(buffer.len_chars())))
}

/// Every match of `pattern`, line by line, in char columns, inside
/// `scope` (char offsets) when given. Empty matches are skipped (they
/// would select nothing).
pub(crate) fn find_in(
    buffer: &Doc,
    pattern: &regex::Regex,
    scope: Option<(usize, usize)>,
) -> Vec<FindMatch> {
    let (first, last) = match scope {
        Some((from, to)) => (buffer.line_col_at(from).0, buffer.line_col_at(to).0),
        None => (0, buffer.line_count().saturating_sub(1)),
    };
    let mut out = Vec::new();
    for line in first..=last.min(buffer.line_count().saturating_sub(1)) {
        let text = buffer.line_text(line);
        let base = buffer.char_at_line_col(line, 0);
        for found in pattern.find_iter(&text) {
            if found.start() == found.end() {
                continue;
            }
            let start = text
                .get(..found.start())
                .map_or(0, |head| head.chars().count());
            let end = start.saturating_add(found.as_str().chars().count());
            if let Some((from, to)) = scope
                && (base.saturating_add(start) < from || base.saturating_add(end) > to)
            {
                continue;
            }
            out.push(FindMatch { line, start, end });
            if out.len() >= MAX_MATCHES {
                return out;
            }
        }
    }
    out
}

/// Select match `index` in the editor (caret at its end, IDE-style).
pub(super) fn select(engine: &mut VimEngine, buffer: &mut Doc, found: FindMatch) {
    let _ = engine.place_cursor(buffer, found.line, found.start);
    engine.extend_selection_to(buffer, found.line, found.end);
}

/// The match after (or before) the caret, wrapping around.
pub(super) fn step(matches: &[FindMatch], caret: (usize, usize), forward: bool) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let found = if forward {
        matches
            .iter()
            .position(|m| (m.line, m.start) >= caret)
            .unwrap_or(0)
    } else {
        matches
            .iter()
            .rposition(|m| (m.line, m.end) < caret)
            .unwrap_or(matches.len().saturating_sub(1))
    };
    Some(found)
}

/// Replace `matches` (sorted) with `replacement`, last first so earlier
/// positions stay valid, as one undo step. Regex mode expands `$1`-style
/// groups against each matched text.
pub(crate) struct ReplaceMatchesInput<'a> {
    pub(crate) buffer: &'a mut Doc,
    pub(crate) engine: &'a mut VimEngine,
    pub(crate) pattern: &'a regex::Regex,
    pub(crate) replacement: &'a str,
    pub(crate) matches: &'a [FindMatch],
    pub(crate) regex: bool,
}

pub(super) fn replace_matches(input: ReplaceMatchesInput<'_>) {
    let ReplaceMatchesInput {
        buffer,
        engine,
        pattern,
        replacement,
        matches,
        regex,
    } = input;
    buffer.begin_undo_group();
    for found in matches.iter().rev() {
        let from = buffer.char_at_line_col(found.line, found.start);
        let to = buffer.char_at_line_col(found.line, found.end);
        let line = buffer.line_text(found.line);
        let matched: String = line
            .chars()
            .skip(found.start)
            .take(found.end.saturating_sub(found.start))
            .collect();
        let text = if regex {
            pattern.replace(&matched, replacement).into_owned()
        } else {
            replacement.to_owned()
        };
        buffer.delete(from..to);
        buffer.insert(from, &text);
    }
    buffer.end_undo_group();
    if let Some(first) = matches.first() {
        let _ = engine.place_cursor(buffer, first.line, first.start);
    }
}
