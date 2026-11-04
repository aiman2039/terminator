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

/// Word class under `big` (WORD) rules: only blanks separate WORDs.
pub(crate) fn class_of(c: char, big: bool) -> WordClass {
    if big {
        if c.is_whitespace() {
            WordClass::Blank
        } else {
            WordClass::Word
        }
    } else {
        word_class(c)
    }
}

/// Random access to buffer chars by char index (newlines included).
pub(crate) trait CharSource {
    fn at(&mut self, i: usize) -> Option<char>;
}

impl CharSource for &[char] {
    fn at(&mut self, i: usize) -> Option<char> {
        self.get(i).copied()
    }
}

fn is_blank<S: CharSource>(s: &mut S, i: usize, big: bool) -> bool {
    s.at(i).is_none_or(|c| class_of(c, big) == WordClass::Blank)
}

/// `i` is the newline of an empty line: vim stops `w`/`b`/`ge` there.
fn empty_line_at<S: CharSource>(s: &mut S, i: usize) -> bool {
    s.at(i) == Some('\n') && i.checked_sub(1).is_none_or(|prev| s.at(prev) == Some('\n'))
}

/// `w` / `W`: start of the `count`th next word; empty lines are words.
pub(crate) fn word_forward<S: CharSource>(
    s: &mut S,
    mut i: usize,
    count: usize,
    big: bool,
) -> usize {
    for _ in 0..count {
        let Some(ch) = s.at(i) else { break };
        let cls = class_of(ch, big);
        if cls != WordClass::Blank {
            while s.at(i).is_some_and(|c| class_of(c, big) == cls) {
                i = i.saturating_add(1);
            }
        }
        while let Some(c) = s.at(i) {
            if c == '\n' {
                i = i.saturating_add(1);
                if s.at(i) == Some('\n') {
                    break;
                }
            } else if class_of(c, big) == WordClass::Blank {
                i = i.saturating_add(1);
            } else {
                break;
            }
        }
    }
    i
}

/// `b` / `B`: start of the `count`th previous word; empty lines are words.
pub(crate) fn word_backward<S: CharSource>(
    s: &mut S,
    mut i: usize,
    count: usize,
    big: bool,
) -> usize {
    for _ in 0..count {
        let Some(mut j) = i.checked_sub(1) else { break };
        while !empty_line_at(s, j) && is_blank(s, j, big) && j > 0 {
            j = j.saturating_sub(1);
        }
        if empty_line_at(s, j) || is_blank(s, j, big) {
            i = j;
            continue;
        }
        let cls = s.at(j).map_or(WordClass::Blank, |c| class_of(c, big));
        while let Some(prev) = j.checked_sub(1)
            && s.at(prev).is_some_and(|c| class_of(c, big) == cls)
        {
            j = prev;
        }
        i = j;
    }
    i
}

/// `e` / `E`: end of the `count`th next word (empty lines are skipped).
pub(crate) fn word_end<S: CharSource>(s: &mut S, mut i: usize, count: usize, big: bool) -> usize {
    for _ in 0..count {
        let mut j = i.saturating_add(1);
        while s
            .at(j)
            .is_some_and(|c| class_of(c, big) == WordClass::Blank)
        {
            j = j.saturating_add(1);
        }
        let Some(ch) = s.at(j) else { break };
        let cls = class_of(ch, big);
        while s
            .at(j.saturating_add(1))
            .is_some_and(|c| class_of(c, big) == cls)
        {
            j = j.saturating_add(1);
        }
        i = j;
    }
    i
}

/// `ge` / `gE`: end of the `count`th previous word.
pub(crate) fn word_end_back<S: CharSource>(
    s: &mut S,
    mut i: usize,
    count: usize,
    big: bool,
) -> usize {
    for _ in 0..count {
        let cls = s.at(i).map_or(WordClass::Blank, |c| class_of(c, big));
        let mut j = i;
        if cls != WordClass::Blank {
            while let Some(prev) = j.checked_sub(1)
                && s.at(prev).is_some_and(|c| class_of(c, big) == cls)
            {
                j = prev;
            }
        }
        let Some(mut k) = j.checked_sub(1) else {
            i = 0;
            break;
        };
        while !empty_line_at(s, k) && is_blank(s, k, big) && k > 0 {
            k = k.saturating_sub(1);
        }
        i = k;
    }
    i
}

/// `cw` on a word: through the end of the current word (even on its
/// last char), then `count - 1` more word ends.
pub(crate) fn change_word_end<S: CharSource>(
    s: &mut S,
    mut i: usize,
    count: usize,
    big: bool,
) -> usize {
    if let Some(ch) = s.at(i) {
        let cls = class_of(ch, big);
        while s
            .at(i.saturating_add(1))
            .is_some_and(|c| class_of(c, big) == cls)
        {
            i = i.saturating_add(1);
        }
    }
    word_end(s, i, count.saturating_sub(1), big)
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

/// Characters vim's default `isfname` accepts in a file name.
fn is_file_name_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(
            c,
            '/' | '.' | '-' | '_' | '+' | ',' | '#' | '$' | '%' | '~' | '=' | '@'
        )
}

/// The file name under (or right after) char column `col` of `line`, as
/// `gf` reads it. A trailing `:line[:col]` is kept for the GUI to parse.
pub(super) fn file_name_at(line: &str, col: usize) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    let start_at =
        (col..chars.len()).find(|&i| chars.get(i).copied().is_some_and(is_file_name_char))?;
    let name_start = |mut start: usize| {
        while let Some(prev) = start.checked_sub(1)
            && chars.get(prev).copied().is_some_and(is_file_name_char)
        {
            start = prev;
        }
        start
    };
    let mut start = name_start(start_at);
    // On the `12` of `main.rs:12[:7]`, step back over `:digits` parts to
    // the file name itself.
    while let Some(colon) = start.checked_sub(1)
        && chars.get(colon) == Some(&':')
        && chars
            .get(start..)
            .unwrap_or_default()
            .iter()
            .take_while(|c| **c != ':' && is_file_name_char(**c))
            .all(char::is_ascii_digit)
        && colon
            .checked_sub(1)
            .and_then(|prev| chars.get(prev))
            .is_some_and(|c| is_file_name_char(*c))
    {
        start = name_start(colon);
    }
    let mut end = start_at;
    while chars
        .get(end)
        .copied()
        .is_some_and(|c| is_file_name_char(c) || c == ':')
    {
        end = end.saturating_add(1);
    }
    let name: String = chars.get(start..end)?.iter().collect();
    let name = name.trim_end_matches([':', '.', ',']).to_owned();
    (!name.is_empty()).then_some(name)
}
