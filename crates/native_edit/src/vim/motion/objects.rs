//! Text objects (`iw`, `a(`, `i"`, `it`, `ip`, …) as buffer spans.
use super::super::super::doc::Buffer;
use super::super::state::VimEngine;
use super::super::words::{CharSource, WordClass, class_of};
use super::delims::{enclosing, pair_for, quote_span};
use super::stream::Stream;

/// A text object: chars `from..to`, or when `linewise` the lines whose
/// starts are `from` and `to` (both included).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) linewise: bool,
}

impl VimEngine {
    /// The span of text object `obj` (`w`, `(`, `"`, `t`, `p`, …) at the
    /// cursor; `inner` for `i`, else `a`.
    pub(crate) fn text_object<B: Buffer>(
        &self,
        doc: &B,
        (obj, inner): (char, bool),
        count: usize,
    ) -> Option<Span> {
        let count = count.max(1);
        match obj {
            'w' | 'W' => self.word_object(doc, (obj == 'W', inner), count),
            'p' => self.paragraph_object(doc, inner, count),
            '"' | '\'' | '`' => self.quote_object(doc, obj, inner),
            't' => tag_object(self, doc, inner, count),
            _ => self.bracket_object(doc, pair_for(obj)?, (inner, count)),
        }
    }

    fn word_object<B: Buffer>(
        &self,
        doc: &B,
        (big, inner): (bool, bool),
        count: usize,
    ) -> Option<Span> {
        let line: Vec<char> = doc.line_text(self.cursor.line).chars().collect();
        let base = doc.char_at_line_col(self.cursor.line, 0);
        if line.is_empty() {
            return Some(Span {
                from: base,
                to: base,
                linewise: false,
            });
        }
        let class = |i: usize| line.get(i).map(|c| class_of(*c, big));
        let col = self.cursor.col.min(line.len().saturating_sub(1));
        let run_end = |mut i: usize| {
            let cls = class(i);
            while i < line.len() && class(i) == cls {
                i = i.saturating_add(1);
            }
            i
        };
        let mut start = col;
        let cls = class(col);
        while start > 0 && class(start.saturating_sub(1)) == cls {
            start = start.saturating_sub(1);
        }
        let on_blank = cls == Some(WordClass::Blank);
        let len = line.len();
        let mut end = start;
        for _ in 0..count {
            if end >= len {
                break;
            }
            let was_blank = class(end) == Some(WordClass::Blank);
            end = run_end(end);
            // `aw`: a word takes its trailing blanks, blanks take the
            // word after them.
            if !inner && end < len && (was_blank || class(end) == Some(WordClass::Blank)) {
                end = run_end(end);
            }
        }
        let trailing_blank = end
            .checked_sub(1)
            .and_then(class)
            .is_some_and(|c| c == WordClass::Blank);
        // `aw` with no trailing blank takes the blank before the word.
        if !inner && !on_blank && !trailing_blank {
            while start > 0 && class(start.saturating_sub(1)) == Some(WordClass::Blank) {
                start = start.saturating_sub(1);
            }
        }
        Some(Span {
            from: base.saturating_add(start),
            to: base.saturating_add(end),
            linewise: false,
        })
    }

    fn paragraph_object<B: Buffer>(&self, doc: &B, inner: bool, count: usize) -> Option<Span> {
        let last = Self::last_addressable(doc);
        let blank = |line: usize| doc.line_text(line).trim().is_empty();
        let run_end = |mut line: usize| {
            let kind = blank(line);
            while line < last && blank(line.saturating_add(1)) == kind {
                line = line.saturating_add(1);
            }
            line
        };
        let mut top = self.cursor.line;
        let kind = blank(top);
        while top > 0 && blank(top.saturating_sub(1)) == kind {
            top = top.saturating_sub(1);
        }
        let mut bottom = top;
        for i in 0..count {
            let start = if i == 0 {
                top
            } else {
                bottom.saturating_add(1)
            };
            if i > 0 && bottom >= last {
                break;
            }
            bottom = run_end(start);
            if !inner && bottom < last {
                bottom = run_end(bottom.saturating_add(1));
            }
        }
        // `ap` with no blank lines after it takes the ones before.
        if !inner && !kind && !blank(bottom) {
            while top > 0 && blank(top.saturating_sub(1)) {
                top = top.saturating_sub(1);
            }
        }
        Some(Span {
            from: doc.char_at_line_col(top, 0),
            to: doc.char_at_line_col(bottom, 0),
            linewise: true,
        })
    }

    fn quote_object<B: Buffer>(&self, doc: &B, quote: char, inner: bool) -> Option<Span> {
        let line: Vec<char> = doc.line_text(self.cursor.line).chars().collect();
        let base = doc.char_at_line_col(self.cursor.line, 0);
        let (open, close) = quote_span(&line, self.cursor.col, quote)?;
        let (mut from, mut to) = if inner {
            (open.saturating_add(1), close)
        } else {
            (open, close.saturating_add(1))
        };
        if !inner {
            // Trailing blanks join `a"`, else leading ones.
            let blank = |i: usize| line.get(i).is_some_and(|c| *c == ' ' || *c == '\t');
            if blank(to) {
                while blank(to) {
                    to = to.saturating_add(1);
                }
            } else {
                while from > 0 && blank(from.saturating_sub(1)) {
                    from = from.saturating_sub(1);
                }
            }
        }
        Some(Span {
            from: base.saturating_add(from),
            to: base.saturating_add(to),
            linewise: false,
        })
    }

    fn bracket_object<B: Buffer>(
        &self,
        doc: &B,
        pair: (char, char),
        (inner, count): (bool, usize),
    ) -> Option<Span> {
        let mut s = Stream::new(doc);
        let idx = self.char_idx(doc, self.cursor);
        let (open, close) = enclosing(&mut s, idx, pair, count)?;
        if !inner {
            return Some(Span {
                from: open,
                to: close.saturating_add(1),
                linewise: false,
            });
        }
        let mut from = open.saturating_add(1);
        let mut to = close;
        // A block whose brackets sit on their own lines: the lines between.
        let open_line = Self::cursor_at(doc, open).line;
        let close_at = Self::cursor_at(doc, close);
        let close_indent = Self::first_non_blank(doc, close_at.line) == close_at.col;
        let mut linewise = false;
        if close_at.line > open_line.saturating_add(1)
            && doc.line_text(open_line).chars().count()
                == Self::cursor_at(doc, open).col.saturating_add(1)
            && close_indent
        {
            // Linewise spans name their first and last line starts.
            from = doc.char_at_line_col(open_line.saturating_add(1), 0);
            to = doc.char_at_line_col(close_at.line.saturating_sub(1), 0);
            linewise = true;
        }
        Some(Span { from, to, linewise })
    }
}

/// `it` / `at`: the `count`th enclosing `<tag>…</tag>` pair.
pub(crate) fn tag_object<B: Buffer>(
    engine: &VimEngine,
    doc: &B,
    inner: bool,
    count: usize,
) -> Option<Span> {
    let mut chars = Stream::new(doc);
    let len = chars.len();
    let at = engine.char_idx(doc, engine.cursor);
    let mut stack: Vec<(String, usize, usize)> = Vec::new();
    let mut found: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut i = 0usize;
    while i < len {
        let Some((name, closing, end)) = parse_tag(&mut chars, i) else {
            i = i.saturating_add(1);
            continue;
        };
        if closing {
            if let Some(pos) = stack.iter().rposition(|(open, _, _)| *open == name) {
                let (_, start, open_end) = stack.get(pos).cloned()?;
                stack.truncate(pos);
                if start <= at && at < end {
                    found.push((start, open_end, i, end));
                }
            }
        } else if !name.is_empty() {
            stack.push((name, i, end));
        }
        i = end;
    }
    // Innermost pairs close first: sort by span size.
    found.sort_by_key(|(start, _, _, end)| end.saturating_sub(*start));
    let (start, open_end, close_start, end) = *found.get(count.saturating_sub(1))?;
    Some(if inner {
        Span {
            from: open_end,
            to: close_start,
            linewise: false,
        }
    } else {
        Span {
            from: start,
            to: end,
            linewise: false,
        }
    })
}

/// A tag at `i`: (name, is closing, index past `>`). Self-closing tags,
/// comments and declarations have an empty name.
fn parse_tag<S: CharSource>(chars: &mut S, i: usize) -> Option<(String, bool, usize)> {
    if chars.at(i) != Some('<') {
        return None;
    }
    let mut j = i.saturating_add(1);
    let closing = chars.at(j) == Some('/');
    if closing {
        j = j.saturating_add(1);
    }
    let mut name = String::new();
    let mut k = j;
    while let Some(c) = chars
        .at(k)
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
    {
        name.push(c);
        k = k.saturating_add(1);
    }
    if name.is_empty() && !matches!(chars.at(j), Some('!' | '?')) {
        return None;
    }
    let mut close = j;
    while chars.at(close)? != '>' {
        close = close.saturating_add(1);
    }
    let self_closing = close > 0 && chars.at(close.saturating_sub(1)) == Some('/');
    let name = if self_closing { String::new() } else { name };
    Some((name, closing, close.saturating_add(1)))
}
