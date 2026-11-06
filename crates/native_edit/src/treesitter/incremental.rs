//! Incremental highlighting: the buffer keeps its last text and syntax
//! tree, so after an edit tree-sitter re-parses only what changed.
//! `tree_sitter_highlight::Highlighter` always parses the whole text, so
//! this runs the same highlight query on the kept tree instead, with the
//! same rules (captures in start order, the last pattern on a node wins,
//! nested captures stack). Our queries have no injections and no locals.
use super::{CAPTURES, CapturePalette, MAX_HIGHLIGHT_BYTES, SpanRows, config_for};
use crate::highlight::{Language, Span};
use syntect::highlighting::Theme;
use tree_sitter::{InputEdit, Parser, Point, QueryCursor, StreamingIterator, Tree};

/// The text and syntax tree of the last highlight of one buffer.
pub struct SyntaxState {
    language: Language,
    text: String,
    tree: Tree,
}

/// [`super::highlight_text`] that re-parses only the edit since the text in
/// `state`, and keeps the new text and tree there. `None` (and no state)
/// where `highlight_text` gives `None`.
pub fn highlight_incremental(
    language: Language,
    text: &str,
    theme: &Theme,
    state: &mut Option<SyntaxState>,
) -> Option<Vec<Vec<Span>>> {
    let previous = state.take().filter(|state| state.language == language);
    if text.len() > MAX_HIGHLIGHT_BYTES {
        return None;
    }
    let config = config_for(language)?;
    let mut parser = Parser::new();
    parser.set_language(&config.language).ok()?;
    let old_tree = previous.map(|mut previous| {
        previous.tree.edit(&edit_between(&previous.text, text));
        previous.tree
    });
    let tree = parser.parse(text, old_tree.as_ref())?;
    let spans = spans_from_tree(&config.query, &tree, text, theme);
    *state = Some(SyntaxState {
        language,
        text: text.to_owned(),
        tree,
    });
    Some(spans)
}

/// The single edit that turns `old` into `new`: the bytes between their
/// common prefix and common suffix.
fn edit_between(old: &str, new: &str) -> InputEdit {
    let (old_bytes, new_bytes) = (old.as_bytes(), new.as_bytes());
    let prefix = old_bytes
        .iter()
        .zip(new_bytes)
        .take_while(|(a, b)| a == b)
        .count();
    let room = old_bytes.len().min(new_bytes.len()).saturating_sub(prefix);
    let suffix = old_bytes
        .iter()
        .rev()
        .zip(new_bytes.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    let old_end = old_bytes.len().saturating_sub(suffix);
    let new_end = new_bytes.len().saturating_sub(suffix);
    InputEdit {
        start_byte: prefix,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point_at(old_bytes, prefix),
        old_end_position: point_at(old_bytes, old_end),
        new_end_position: point_at(new_bytes, new_end),
    }
}

/// Row and byte column of `byte` in `text`.
fn point_at(text: &[u8], byte: usize) -> Point {
    let before = text.get(..byte).unwrap_or(text);
    let row = before.iter().filter(|b| **b == b'\n').count();
    let column = before
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(before.len(), |newline| {
            before.len().saturating_sub(newline).saturating_sub(1)
        });
    Point { row, column }
}

/// Per-line spans from the highlight query on `tree`, like the highlighter
/// events: open a capture at its start, close it at its end.
fn spans_from_tree(
    query: &tree_sitter::Query,
    tree: &Tree,
    text: &str,
    theme: &Theme,
) -> Vec<Vec<Span>> {
    let palette = CapturePalette::new(theme);
    // Query capture index -> index into `CAPTURES`.
    let highlights: Vec<Option<usize>> = query
        .capture_names()
        .iter()
        .map(|name| CAPTURES.iter().position(|capture| capture == name))
        .collect();
    // (start, end, node, highlight); later patterns on one node win.
    let mut found: Vec<(usize, usize, usize, Option<usize>)> = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut captures = cursor.captures(query, tree.root_node(), text.as_bytes());
    while let Some((matched, index)) = captures.next() {
        let Some(capture) = matched.captures().get(*index) else {
            continue;
        };
        let node = capture.node;
        let highlight = usize::try_from(capture.index)
            .ok()
            .and_then(|index| highlights.get(index).copied().flatten());
        let entry = (node.start_byte(), node.end_byte(), node.id(), highlight);
        match found.last_mut() {
            Some(last) if last.2 == entry.2 => *last = entry,
            _ => found.push(entry),
        }
    }
    let mut rows = SpanRows::default();
    let mut ends: Vec<usize> = Vec::new();
    let mut active: Vec<usize> = Vec::new();
    let mut offset = 0usize;
    let emit = |rows: &mut SpanRows, offset: &mut usize, to: usize, active: &[usize]| {
        if to > *offset {
            let piece = text.get(*offset..to).unwrap_or("");
            rows.push_source(piece, palette.active(active));
            *offset = to;
        }
    };
    for (start, end, _, highlight) in found {
        while let Some(&close) = ends.last().filter(|close| **close <= start) {
            emit(&mut rows, &mut offset, close, &active);
            ends.pop();
            active.pop();
        }
        let Some(highlight) = highlight else {
            continue;
        };
        emit(&mut rows, &mut offset, start, &active);
        ends.push(end);
        active.push(highlight);
    }
    while let Some(close) = ends.pop() {
        emit(&mut rows, &mut offset, close, &active);
        active.pop();
    }
    emit(&mut rows, &mut offset, text.len(), &active);
    rows.finish(text)
}
