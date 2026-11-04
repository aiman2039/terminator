//! Vim patterns (magic, `\v` very magic, `\V` very nomagic) as `regex`
//! syntax, with `ignorecase`/`smartcase` and the `\c` / `\C` overrides.
use regex::{Regex, RegexBuilder};

/// Compile a vim pattern. An untranslatable pattern falls back to a
/// literal search, so a search never fails just for syntax.
pub(crate) fn compile(pattern: &str, ignorecase: bool, smartcase: bool) -> Option<Regex> {
    if pattern.is_empty() {
        return None;
    }
    let (translated, case) = translate(pattern);
    let insensitive = case
        .unwrap_or_else(|| ignorecase && !(smartcase && pattern.chars().any(char::is_uppercase)));
    RegexBuilder::new(&translated)
        .case_insensitive(insensitive)
        .build()
        .or_else(|_| {
            RegexBuilder::new(&regex::escape(pattern))
                .case_insensitive(insensitive)
                .build()
        })
        .ok()
}

/// Escape `text` so a vim (magic) pattern matches it literally.
pub(crate) fn escape_vim(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if matches!(c, '\\' | '/' | '.' | '*' | '$' | '^' | '~' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Magic {
    Very,
    Normal,
    None,
}

/// Translate to `regex` syntax; the second value is a forced case
/// (`Some(true)` for `\c`: insensitive).
pub(crate) fn translate(pattern: &str) -> (String, Option<bool>) {
    let mut out = String::new();
    let mut case = None;
    let mut magic = Magic::Normal;
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let Some(next) = chars.next() else {
                out.push_str("\\\\");
                break;
            };
            match next {
                'c' => case = Some(true),
                'C' => case = Some(false),
                'v' => magic = Magic::Very,
                'm' => magic = Magic::Normal,
                'V' | 'M' => magic = Magic::None,
                _ => out.push_str(&escaped(next, magic, &mut chars)),
            }
        } else {
            out.push_str(&plain(c, magic, &mut chars));
        }
    }
    (out, case)
}

/// A backslash sequence `\c`.
fn escaped(c: char, magic: Magic, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    match c {
        '<' | '>' => "\\b".into(),
        's' | 'S' | 'd' | 'D' | 'w' | 'W' | 'n' | 't' => format!("\\{c}"),
        'r' => "\\r".into(),
        'e' => "\\x1b".into(),
        'a' => "[A-Za-z]".into(),
        'l' => "[a-z]".into(),
        'u' => "[A-Z]".into(),
        'x' => "[0-9A-Fa-f]".into(),
        'h' => "[A-Za-z_]".into(),
        '=' | '?' if magic != Magic::Very => "?".into(),
        '{' if magic != Magic::Very => brace(chars),
        '(' | ')' | '|' | '+' if magic != Magic::Very => c.to_string(),
        '.' | '*' | '[' | ']' | '~' | '^' | '$' if magic == Magic::None => c.to_string(),
        _ => regex::escape(&c.to_string()),
    }
}

/// An unescaped char.
fn plain(c: char, magic: Magic, chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    match magic {
        Magic::None => regex::escape(&c.to_string()),
        Magic::Normal => match c {
            '.' | '*' | '^' | '$' => c.to_string(),
            '[' => bracket(chars),
            _ => regex::escape(&c.to_string()),
        },
        Magic::Very => match c {
            '<' | '>' => "\\b".into(),
            '=' => "?".into(),
            '{' => brace(chars),
            '[' => bracket(chars),
            '.' | '*' | '^' | '$' | '(' | ')' | '|' | '+' | '?' => c.to_string(),
            _ => regex::escape(&c.to_string()),
        },
    }
}

/// `{n,m}` (after `\{` or very-magic `{`); `{-n,m}` is lazy.
fn brace(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut body = String::new();
    for c in chars.by_ref() {
        if c == '}' {
            break;
        }
        if c != '\\' {
            body.push(c);
        }
    }
    let (lazy, body) = match body.strip_prefix('-') {
        Some(rest) => (true, rest.to_string()),
        None => (false, body),
    };
    let quant = if body.is_empty() {
        "*".to_string()
    } else if body.starts_with(',') {
        format!("{{0{body}}}")
    } else {
        format!("{{{body}}}")
    };
    if lazy { format!("{quant}?") } else { quant }
}

/// A `[...]` collection, copied through (with `[` and `&&` made safe).
fn bracket(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut out = String::from("[");
    if chars.peek() == Some(&'^') {
        out.push('^');
        chars.next();
    }
    if chars.peek() == Some(&']') {
        out.push_str("\\]");
        chars.next();
    }
    let mut closed = false;
    while let Some(c) = chars.next() {
        match c {
            ']' => {
                closed = true;
                break;
            }
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push('\\');
                    out.push(next);
                }
            }
            '[' | '&' | '~' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    if closed {
        out.push(']');
        out
    } else {
        // An unclosed `[` is a literal bracket in vim.
        format!("\\[{}", out.get(1..).unwrap_or_default())
    }
}
