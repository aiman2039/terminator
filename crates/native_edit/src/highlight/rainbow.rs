//! Rainbow brackets: each `{[()]}` in code gets a color by its nesting
//! depth, laid over the syntax spans. Strings and comments are skipped by
//! the same lexer the fold regions use.
use super::{Span, SpanStyle};
use crate::fold::BracketLexer;

/// Bracket colors by depth (the VS Code bracket pair defaults).
const DARK: [[u8; 3]; 3] = [[0xFF, 0xD7, 0x00], [0xDA, 0x70, 0xD6], [0x17, 0x9F, 0xFF]];
const LIGHT: [[u8; 3]; 3] = [[0x04, 0x31, 0xFA], [0x31, 0x93, 0x31], [0x7B, 0x38, 0x14]];

/// Recolor the brackets of `text` in `lines` (one span list per line).
pub(super) fn apply(lines: &mut Vec<Vec<Span>>, text: &str, dark: bool) {
    let colors = if dark { &DARK } else { &LIGHT };
    let mut lexer = BracketLexer::default();
    let mut depth = 0usize;
    for (index, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        let mut marks = Vec::new();
        lexer.line(&chars, |column, bracket| {
            let level = if matches!(bracket, '{' | '[' | '(') {
                let level = depth;
                depth = depth.saturating_add(1);
                level
            } else {
                depth = depth.saturating_sub(1);
                depth
            };
            let color = colors
                .get(level.checked_rem(colors.len()).unwrap_or_default())
                .copied()
                .unwrap_or_default();
            marks.push((column, color));
        });
        if marks.is_empty() {
            continue;
        }
        if lines.len() <= index {
            lines.resize_with(index.saturating_add(1), Vec::new);
        }
        if let Some(spans) = lines.get_mut(index) {
            *spans = overlay(spans, &marks);
        }
    }
}

/// `spans` with a one-character span at each `(column, color)` mark.
/// Spans are sorted and do not overlap; marks are sorted by column.
fn overlay(spans: &[Span], marks: &[(usize, [u8; 3])]) -> Vec<Span> {
    let mark = |column: usize, color, style| Span {
        start: column,
        end: column.saturating_add(1),
        color,
        style,
    };
    let mut out = Vec::with_capacity(spans.len().saturating_add(marks.len()));
    let mut marks = marks.iter().copied().peekable();
    for span in spans {
        while let Some((column, color)) = marks.next_if(|(column, _)| *column < span.start) {
            out.push(mark(column, color, SpanStyle::default()));
        }
        let mut from = span.start;
        while let Some((column, color)) = marks.next_if(|(column, _)| *column < span.end) {
            if column > from {
                out.push(Span {
                    start: from,
                    end: column,
                    ..*span
                });
            }
            out.push(mark(column, color, span.style));
            from = column.saturating_add(1);
        }
        if from < span.end {
            out.push(Span {
                start: from,
                ..*span
            });
        }
    }
    out.extend(marks.map(|(column, color)| mark(column, color, SpanStyle::default())));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors_at(text: &str) -> Vec<Vec<(usize, [u8; 3])>> {
        let mut lines = Vec::new();
        apply(&mut lines, text, true);
        lines
            .iter()
            .map(|spans| spans.iter().map(|s| (s.start, s.color)).collect())
            .collect()
    }

    #[test]
    fn nested_brackets_cycle_colors_by_depth() {
        let lines = colors_at("f(a[b{c}])");
        assert_eq!(
            lines,
            vec![vec![
                (1, DARK[0]),
                (3, DARK[1]),
                (5, DARK[2]),
                (7, DARK[2]),
                (8, DARK[1]),
                (9, DARK[0]),
            ]]
        );
    }

    #[test]
    fn brackets_in_strings_and_comments_keep_their_color() {
        let lines = colors_at("x(\"(\") // )\n/* { */ y");
        assert_eq!(lines, vec![vec![(1, DARK[0]), (5, DARK[0])]]);
    }

    #[test]
    fn unmatched_closer_does_not_underflow() {
        let lines = colors_at(")(");
        assert_eq!(lines, vec![vec![(0, DARK[0]), (1, DARK[0])]]);
    }

    #[test]
    fn overlay_splits_a_covering_span_and_keeps_its_style() {
        let style = SpanStyle {
            italic: true,
            bold: false,
        };
        let spans = [Span {
            start: 0,
            end: 4,
            color: [1, 1, 1],
            style,
        }];
        let out = overlay(&spans, &[(2, [9, 9, 9])]);
        let ranges: Vec<_> = out.iter().map(|s| (s.start, s.end, s.color)).collect();
        assert_eq!(
            ranges,
            [(0, 2, [1, 1, 1]), (2, 3, [9, 9, 9]), (3, 4, [1, 1, 1])]
        );
        assert!(out.iter().all(|s| s.style == style));
    }
}
