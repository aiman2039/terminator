use super::super::doc::Buffer;
use super::mode::Cursor;
// --- flat-stream word helpers (operate on char indices, cross lines) ---

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WordClass {
    Blank,
    Word,
    Punct,
}

pub(crate) fn word_class(c: char) -> WordClass {
    if c.is_whitespace() {
        WordClass::Blank
    } else if c.is_alphanumeric() || c == '_' {
        WordClass::Word
    } else {
        WordClass::Punct
    }
}

pub(crate) fn word_forward(text: &[char], mut i: usize, count: usize) -> usize {
    for _ in 0..count {
        if i >= text.len() {
            break;
        }
        let Some(&ch) = text.get(i) else {
            break;
        };
        let cls = word_class(ch);
        if cls != WordClass::Blank {
            while text.get(i).is_some_and(|ch| word_class(*ch) == cls) {
                i = i.checked_add(1).unwrap_or(text.len());
            }
        }
        while text
            .get(i)
            .is_some_and(|ch| word_class(*ch) == WordClass::Blank)
        {
            i = i.checked_add(1).unwrap_or(text.len());
        }
    }
    i.min(text.len())
}

pub(crate) fn word_backward(text: &[char], mut i: usize, count: usize) -> usize {
    for _ in 0..count {
        if i == 0 {
            break;
        }
        let mut j = i.saturating_sub(1);
        while j > 0
            && text
                .get(j)
                .is_some_and(|ch| word_class(*ch) == WordClass::Blank)
        {
            j = j.saturating_sub(1);
        }
        let cls = word_class(text.get(j).copied().unwrap_or(' '));
        while j > 0
            && j.checked_sub(1)
                .and_then(|prev| text.get(prev).copied())
                .is_some_and(|ch| word_class(ch) == cls)
        {
            j = j.saturating_sub(1);
        }
        i = j;
    }
    i
}

pub(crate) fn word_end(text: &[char], mut i: usize, count: usize) -> usize {
    for _ in 0..count {
        if text.is_empty() {
            break;
        }
        i = i
            .checked_add(1)
            .unwrap_or(i)
            .min(text.len().saturating_sub(1));
        while text
            .get(i)
            .is_some_and(|ch| word_class(*ch) == WordClass::Blank)
        {
            i = i.checked_add(1).unwrap_or(text.len());
        }
        if i >= text.len() {
            break;
        }
        let Some(ch) = text.get(i).copied() else {
            break;
        };
        let cls = word_class(ch);
        while i
            .checked_add(1)
            .and_then(|next| text.get(next).copied())
            .is_some_and(|ch| word_class(ch) == cls)
        {
            i = i.checked_add(1).unwrap_or(i);
        }
    }
    i.min(text.len().saturating_sub(1))
}

// --- cursor plumbing ---

pub(crate) fn line_len<B: Buffer>(doc: &B, line: usize) -> usize {
    doc.line_text(line).chars().count()
}

pub(crate) fn current_char<B: Buffer>(doc: &B, cursor: Cursor) -> char {
    doc.line_text(cursor.line)
        .chars()
        .nth(cursor.col)
        .unwrap_or('\n')
}

pub(crate) fn flat_text<B: Buffer>(doc: &B) -> Vec<char> {
    let mut out = Vec::new();
    for line in 0..doc.line_count() {
        if line > 0 {
            out.push('\n');
        }
        out.extend(doc.line_text(line).chars());
    }
    out
}
