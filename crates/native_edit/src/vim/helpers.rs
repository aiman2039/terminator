use super::super::doc::Buffer;
pub(crate) fn doc_is_empty<B: Buffer>(doc: &B) -> bool {
    doc.len_chars() == 0
}

/// The break after `line` as the rope holds it: `\r\n` (two chars) in a
/// mixed-ending buffer, else one char read as `\n`. Char offsets of text
/// rebuilt from lines then match [`Buffer::char_at_line_col`].
pub(crate) fn line_break<B: Buffer>(doc: &B, line: usize) -> &'static str {
    let end = doc.char_at_line_col(line, usize::MAX);
    let next = doc.char_at_line_col(line.saturating_add(1), 0);
    if next.saturating_sub(end) == 2 {
        "\r\n"
    } else {
        "\n"
    }
}

pub(crate) fn range_text<B: Buffer>(doc: &B, from: usize, to: usize) -> String {
    let (from, to) = (from.min(to), from.max(to).min(doc.len_chars()));
    // Reconstruct from lines to avoid another full-text accessor on the trait.
    let mut out = String::new();
    let mut idx = 0;
    for line in 0..doc.line_count() {
        if line > 0 {
            for c in line_break(doc, line.saturating_sub(1)).chars() {
                if idx >= from && idx < to {
                    out.push(c);
                }
                idx = idx.checked_add(1).unwrap_or(idx);
            }
        }
        for c in doc.line_text(line).chars() {
            if idx >= from && idx < to {
                out.push(c);
            }
            idx = idx.checked_add(1).unwrap_or(idx);
            if idx >= to {
                return out;
            }
        }
    }
    out
}

pub(crate) use crate::doc::normalize_breaks as normalize_newlines;

/// Char column where each grapheme cluster of `text` starts, so `l`, `h`
/// and `x` step over a decomposed `é` or an emoji sequence as one cell.
fn grapheme_starts(text: &str) -> impl Iterator<Item = usize> + '_ {
    use unicode_segmentation::UnicodeSegmentation;
    let mut col = 0usize;
    text.graphemes(true).map(move |g| {
        let at = col;
        col = col.saturating_add(g.chars().count());
        at
    })
}

/// Column after `count` graphemes right of `col` (at most the line end).
pub(crate) fn grapheme_right(text: &str, col: usize, count: usize) -> usize {
    let len = text.chars().count();
    grapheme_starts(text)
        .filter(|start| *start > col)
        .nth(count.saturating_sub(1))
        .unwrap_or(len)
}

/// Column `count` graphemes left of `col` (at least 0).
pub(crate) fn grapheme_left(text: &str, col: usize, count: usize) -> usize {
    let starts: Vec<usize> = grapheme_starts(text)
        .take_while(|start| *start < col)
        .collect();
    starts
        .len()
        .checked_sub(count.max(1))
        .and_then(|i| starts.get(i).copied())
        .unwrap_or(0)
}

/// Start of the grapheme that holds `col` (a cursor never rests inside one).
pub(crate) fn grapheme_floor(text: &str, col: usize) -> usize {
    grapheme_starts(text)
        .take_while(|start| *start <= col)
        .last()
        .unwrap_or(0)
}

/// Column of the last grapheme start (Normal mode's rightmost cell).
pub(crate) fn last_grapheme(text: &str) -> usize {
    grapheme_starts(text).last().unwrap_or(0)
}
