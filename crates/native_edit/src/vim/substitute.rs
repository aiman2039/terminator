//! `:s` substitution over a line range, with vim patterns and
//! replacement escapes.
use super::super::doc::Buffer;
use super::mode::{Cursor, Effect};
use super::state::VimEngine;
use regex::Captures;

/// A parsed `s/pat/rep/flags`, before the pattern falls back to the last
/// search.
struct SubstituteSpec {
    pattern: String,
    replacement: String,
    global: bool,
    /// `Some(true)` for `i`, `Some(false)` for `I`.
    case: Option<bool>,
    /// `n`: count matches, change nothing.
    count_only: bool,
}

impl VimEngine {
    /// `s/pat/rep/[flags]` on lines `first..=last`. An empty pattern
    /// reuses the last search. One undo unit; zero matches and unknown
    /// flags (including `c`, not supported) bell. Never `.`-repeatable.
    pub(super) fn substitute<B: Buffer>(
        &mut self,
        doc: &mut B,
        (first, last): (usize, usize),
        body: &str,
    ) -> Vec<Effect> {
        let Some(mut spec) = Self::parse_substitute(body) else {
            return vec![Effect::Bell];
        };
        if spec.pattern.is_empty() {
            spec.pattern.clone_from(&self.search_last);
        }
        self.search_last.clone_from(&spec.pattern);
        let pattern = match spec.case {
            Some(true) => format!("\\c{}", spec.pattern),
            Some(false) => format!("\\C{}", spec.pattern),
            None => spec.pattern.clone(),
        };
        let Some(re) = self.search_regex(&pattern) else {
            return vec![Effect::Bell];
        };
        let mut first_hit: Option<Cursor> = None;
        let mut edits: Vec<(usize, String)> = Vec::new();
        for line in first..=last.min(Self::last_addressable(doc)) {
            let text = doc.line_text(line);
            let mut hit = false;
            let take = if spec.global { usize::MAX } else { 1 };
            let new = re.replacen(&text, take, |caps: &Captures<'_>| {
                if !hit && first_hit.is_none() {
                    let start = caps.get(0).map_or(0, |m| m.start());
                    first_hit = Some(Cursor {
                        line,
                        col: text.get(..start).map_or(0, |p| p.chars().count()),
                    });
                }
                hit = true;
                expand_replacement(&spec.replacement, caps)
            });
            if hit {
                edits.push((line, new.into_owned()));
            }
        }
        let Some(cursor) = first_hit else {
            return vec![Effect::Bell];
        };
        if spec.count_only {
            return Vec::new();
        }
        doc.begin_undo_group();
        for (line, text) in edits.iter().rev() {
            let from = doc.char_at_line_col(*line, 0);
            let to = doc.char_at_line_col(*line, doc.line_text(*line).chars().count());
            doc.delete(from..to);
            doc.insert(from, text);
        }
        doc.end_undo_group();
        self.cursor = cursor;
        self.scroll_request = true;
        self.clamp(doc);
        doc.end_run();
        Vec::new()
    }

    /// Split `<delim>pat<delim>rep[<delim>flags]` on unescaped delimiters
    /// (`\<delim>` is the delimiter itself; other escapes stay for the
    /// pattern and replacement); `None` for malformed commands and flags
    /// other than `g`, `i`, `I`, `n`, `e`.
    fn parse_substitute(body: &str) -> Option<SubstituteSpec> {
        let mut chars = body.chars();
        let delim = chars.next()?;
        if delim.is_alphanumeric() || delim.is_whitespace() || delim == '\\' {
            return None;
        }
        let mut parts: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut escaped = false;
        for c in chars {
            if escaped {
                if c != delim {
                    current.push('\\');
                }
                current.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == delim && parts.len() < 2 {
                parts.push(std::mem::take(&mut current));
            } else {
                current.push(c);
            }
        }
        // A trailing backslash escapes nothing.
        if escaped {
            return None;
        }
        parts.push(current);
        if parts.len() < 2 {
            return None;
        }
        let flags = parts.get(2).map_or("", String::as_str).trim();
        if !flags
            .chars()
            .all(|c| matches!(c, 'g' | 'i' | 'I' | 'n' | 'e'))
        {
            return None;
        }
        let case = flags.chars().rev().find_map(|c| match c {
            'i' => Some(true),
            'I' => Some(false),
            _ => None,
        });
        let (global, count_only) = (flags.contains('g'), flags.contains('n'));
        let mut parts = parts.into_iter();
        Some(SubstituteSpec {
            pattern: parts.next()?,
            replacement: parts.next()?,
            global,
            case,
            count_only,
        })
    }
}

/// Case change pending from `\u`, `\l` (one char) or `\U`, `\L` (until
/// `\E`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum CaseMod {
    None,
    UpperOne,
    LowerOne,
    Upper,
    Lower,
}

/// Vim replacement: `&` and `\0` are the match, `\1`..`\9` groups, `\&`
/// a literal `&`, `\n`/`\r` a newline, `\t` a tab, `\u\l\U\L\E` case.
fn expand_replacement(replacement: &str, caps: &Captures<'_>) -> String {
    let mut out = String::new();
    let mut mode = CaseMod::None;
    let mut chars = replacement.chars();
    while let Some(c) = chars.next() {
        let piece: String = match c {
            '&' => caps.get(0).map_or("", |m| m.as_str()).into(),
            '\\' => match chars.next() {
                Some(d @ '0'..='9') => {
                    let group = d
                        .to_digit(10)
                        .and_then(|n| usize::try_from(n).ok())
                        .unwrap_or(0);
                    caps.get(group).map_or("", |m| m.as_str()).into()
                }
                Some('n' | 'r') => "\n".into(),
                Some('t') => "\t".into(),
                Some('u') => {
                    mode = CaseMod::UpperOne;
                    continue;
                }
                Some('l') => {
                    mode = CaseMod::LowerOne;
                    continue;
                }
                Some('U') => {
                    mode = CaseMod::Upper;
                    continue;
                }
                Some('L') => {
                    mode = CaseMod::Lower;
                    continue;
                }
                Some('E' | 'e') => {
                    mode = CaseMod::None;
                    continue;
                }
                Some(other) => other.to_string(),
                None => String::new(),
            },
            _ => c.to_string(),
        };
        for ch in piece.chars() {
            match mode {
                CaseMod::UpperOne => {
                    out.extend(ch.to_uppercase());
                    mode = CaseMod::None;
                }
                CaseMod::LowerOne => {
                    out.extend(ch.to_lowercase());
                    mode = CaseMod::None;
                }
                CaseMod::Upper => out.extend(ch.to_uppercase()),
                CaseMod::Lower => out.extend(ch.to_lowercase()),
                CaseMod::None => out.push(ch),
            }
        }
    }
    out
}
