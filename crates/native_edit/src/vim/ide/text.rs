//! Stateless text scans for IDE editing: bracket pairs, selection growth
//! candidates, and indentation.

pub(super) struct IdeText;

impl IdeText {
    pub fn closer_for(open: char) -> Option<char> {
        match open {
            '(' => Some(')'),
            '[' => Some(']'),
            '{' => Some('}'),
            _ => None,
        }
    }

    pub fn is_closer(c: char) -> bool {
        matches!(c, ')' | ']' | '}')
    }

    /// A pair may open before end of line, whitespace, a closer, or a
    /// separator; before a word it would wrap text the user did not mean
    /// to wrap.
    pub fn pair_may_open_before(next: Option<char>) -> bool {
        next.is_none_or(|c| c.is_whitespace() || Self::is_closer(c) || matches!(c, ',' | ';' | ':'))
    }

    pub fn leading_whitespace(text: &str) -> String {
        text.chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    }

    /// The smallest unit strictly containing `(from, to)` in `text`.
    pub fn extend_candidate(text: &[char], (from, to): (usize, usize)) -> Option<(usize, usize)> {
        let len = text.len();
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let mut candidates: Vec<(usize, usize)> = Vec::new();
        // Word around the selection.
        let (mut start, mut end) = (from, to);
        while start > 0 && text.get(start.saturating_sub(1)).copied().is_some_and(word) {
            start = start.saturating_sub(1);
        }
        while text.get(end).copied().is_some_and(word) {
            end = end.saturating_add(1);
        }
        candidates.push((start, end));
        // Enclosing brackets, innermost first: contents, then with brackets.
        let mut open_from = from;
        for _ in 0..32 {
            let Some((open, close)) = Self::enclosing_pair(text, open_from, to) else {
                break;
            };
            candidates.push((open.saturating_add(1), close));
            candidates.push((open, close.saturating_add(1)));
            open_from = open;
        }
        // String contents on the line.
        let line_start = text
            .get(..from)
            .and_then(|head| head.iter().rposition(|c| *c == '\n'))
            .map_or(0, |index| index.saturating_add(1));
        let line_end = text
            .get(to..)
            .and_then(|tail| tail.iter().position(|c| *c == '\n'))
            .map_or(len, |index| to.saturating_add(index));
        let quotes: Vec<usize> = (line_start..line_end)
            .filter(|index| text.get(*index) == Some(&'"'))
            .collect();
        for pair in quotes.chunks(2) {
            if let [open, close] = pair
                && *open < from
                && to <= *close
            {
                candidates.push((open.saturating_add(1), *close));
                candidates.push((*open, close.saturating_add(1)));
            }
        }
        // The lines without their indentation, the whole lines, the buffer.
        let indent = text.get(line_start..line_end).map_or(0, |line| {
            line.iter().take_while(|c| c.is_whitespace()).count()
        });
        candidates.push((line_start.saturating_add(indent), line_end));
        candidates.push((line_start, line_end));
        candidates.push((0, len));
        candidates
            .into_iter()
            .filter(|(a, b)| *a <= from && to <= *b && (*a, *b) != (from, to) && a < b)
            .min_by_key(|(a, b)| b.saturating_sub(*a))
    }

    /// The innermost bracket pair around `from..to`: an unmatched opener
    /// before `from` and its matching closer at or after `to`.
    fn enclosing_pair(text: &[char], from: usize, to: usize) -> Option<(usize, usize)> {
        let mut depth: Vec<char> = Vec::new();
        let mut index = from;
        while index > 0 {
            index = index.saturating_sub(1);
            let c = *text.get(index)?;
            match c {
                ')' | ']' | '}' => depth.push(c),
                '(' | '[' | '{' if depth.pop().is_none() => {
                    let at = Self::closer_after(text, index, c)?;
                    return (at >= to).then_some((index, at));
                }
                _ => {}
            }
        }
        None
    }

    /// The closer matching the opener `open` at `index`, skipping nested
    /// pairs of the same kind.
    fn closer_after(text: &[char], index: usize, open: char) -> Option<usize> {
        let close = Self::closer_for(open)?;
        let mut nested = 0usize;
        let mut at = index.saturating_add(1);
        while let Some(next) = text.get(at) {
            if *next == open {
                nested = nested.saturating_add(1);
            } else if *next == close {
                if nested == 0 {
                    return Some(at);
                }
                nested = nested.saturating_sub(1);
            }
            at = at.saturating_add(1);
        }
        None
    }
}
