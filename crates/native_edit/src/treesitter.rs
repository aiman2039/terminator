//! Tree-sitter highlighting for the most-edited languages.
//!
//! Where syntect paints by regex state, tree-sitter parses: nested
//! strings, macros, and interpolations highlight precisely. Styled
//! captures produce themed spans, unstyled gaps produce spans in the
//! theme's default foreground, so every row partitions exactly like the
//! syntect backend; colors come from the active syntect theme by scope,
//! so both backends agree.
//! Parse failures, grammar mismatches, and oversized buffers fall back
//! to syntect (the 256KB cap bounds parse work; no timeout API exists
//! on this tree-sitter version).
use syntect::highlighting::Theme;
use syntect::parsing::{MatchPower, Scope, ScopeStack};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use crate::highlight::{Language, MAX_HIGHLIGHT_BYTES, Span};

const RUST_QUERY: &str = r"
(line_comment) @comment
(block_comment) @comment
(string_literal) @string
(raw_string_literal) @string
(char_literal) @string
(boolean_literal) @keyword
(integer_literal) @number
(float_literal) @number
(function_item name: (identifier) @function)
(call_expression function: (identifier) @function)
(call_expression function: (field_expression field: (field_identifier) @function))
(primitive_type) @type
(type_identifier) @type
(mutable_specifier) @keyword
";

/// Anonymous keyword tokens per language. Grammars rename tokens across
/// versions (`mut`, `None`, `this` all became named nodes), so this list
/// is filtered at build time against the linked grammar: unknown tokens
/// are dropped instead of failing the whole query.
const RUST_KEYWORDS: &[&str] = &[
    "fn", "let", "return", "if", "else", "match", "struct", "enum", "impl", "pub", "use", "mod",
    "const", "static", "trait", "where", "for", "in", "while", "loop", "break", "continue", "move",
    "ref", "self", "Self", "crate", "super", "as", "dyn", "unsafe", "extern", "type", "union",
    "async", "await", "try",
];

const PYTHON_QUERY: &str = r"
(comment) @comment
(string) @string
(integer) @number
(float) @number
(function_definition name: (identifier) @function)
(call function: (identifier) @function)
(class_definition name: (identifier) @type)
(none) @keyword
(true) @keyword
(false) @keyword
";

const PYTHON_KEYWORDS: &[&str] = &[
    "def", "class", "return", "if", "elif", "else", "for", "while", "import", "from", "as", "with",
    "try", "except", "finally", "raise", "pass", "break", "continue", "lambda", "and", "or", "not",
    "in", "is", "global", "nonlocal", "assert", "del", "yield", "async", "await",
];

const JAVASCRIPT_QUERY: &str = r"
(comment) @comment
(string) @string
(template_string) @string
(number) @number
(function_declaration name: (identifier) @function)
(call_expression function: (identifier) @function)
(this) @keyword
";

const JAVASCRIPT_KEYWORDS: &[&str] = &[
    "function",
    "return",
    "if",
    "else",
    "for",
    "while",
    "do",
    "switch",
    "case",
    "break",
    "continue",
    "var",
    "let",
    "const",
    "class",
    "extends",
    "new",
    "typeof",
    "in",
    "of",
    "import",
    "from",
    "export",
    "default",
    "try",
    "catch",
    "finally",
    "throw",
    "async",
    "await",
    "yield",
    "null",
    "undefined",
    "true",
    "false",
];

/// Capture names this backend styles; order is the query `configure` list.
const CAPTURES: &[&str] = &["comment", "string", "keyword", "function", "type", "number"];

/// Capture name to syntect scope, for theme-consistent colors.
fn scope_for_capture(capture: &str) -> Option<&'static str> {
    match capture {
        "comment" => Some("comment"),
        "string" => Some("string"),
        "keyword" => Some("keyword"),
        "function" => Some("entity.name.function"),
        "type" => Some("entity.name.type"),
        "number" => Some("constant.numeric"),
        _ => None,
    }
}

/// Theme foreground for a syntect scope: strongest matching rule wins,
/// falling back to the theme default.
fn color_for_scope(theme: &Theme, scope: &str) -> [u8; 3] {
    let default = theme
        .settings
        .foreground
        .unwrap_or(syntect::highlighting::Color {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        });
    let Ok(wanted) = Scope::new(scope) else {
        return [default.r, default.g, default.b];
    };
    let mut stack = ScopeStack::new();
    stack.push(wanted);
    let mut best: Option<(MatchPower, [u8; 3])> = None;
    for item in &theme.scopes {
        if let Some(power) = item.scope.does_match(stack.as_slice())
            && let Some(foreground) = item.style.foreground
        {
            let color = [foreground.r, foreground.g, foreground.b];
            if best.is_none_or(|(best_power, _)| power > best_power) {
                best = Some((power, color));
            }
        }
    }
    best.map_or([default.r, default.g, default.b], |(_, color)| color)
}

fn grammar(
    language: Language,
) -> Option<(
    &'static str,
    tree_sitter::Language,
    &'static str,
    &'static [&'static str],
)> {
    match language {
        Language::Rust => Some((
            "rust",
            tree_sitter_rust::LANGUAGE.into(),
            RUST_QUERY,
            RUST_KEYWORDS,
        )),
        Language::Python => Some((
            "python",
            tree_sitter_python::LANGUAGE.into(),
            PYTHON_QUERY,
            PYTHON_KEYWORDS,
        )),
        Language::JavaScript => Some((
            "javascript",
            tree_sitter_javascript::LANGUAGE.into(),
            JAVASCRIPT_QUERY,
            JAVASCRIPT_KEYWORDS,
        )),
        _ => None,
    }
}

/// Full highlight query: node patterns plus the keyword tokens the
/// linked grammar actually defines (`id_for_node_kind` returns 0 for
/// unknown names, so renames across grammar versions degrade to fewer
/// keywords instead of a failed query).
fn build_query(grammar: &tree_sitter::Language, nodes: &str, keywords: &[&str]) -> String {
    let mut query = nodes.to_owned();
    query.push_str("\n[\n ");
    for word in keywords
        .iter()
        .filter(|word| grammar.id_for_node_kind(word, false) != 0)
    {
        query.push(' ');
        query.push('"');
        query.push_str(word);
        query.push('"');
    }
    query.push_str("\n] @keyword\n");
    query
}

/// Highlight text into per-line spans. `None` means fall back to syntect:
/// unsupported language, oversized buffer, bad query, grammar mismatch,
/// or highlight-iteration failure.
#[must_use]
pub fn highlight_text(language: Language, text: &str, theme: &Theme) -> Option<Vec<Vec<Span>>> {
    if text.len() > MAX_HIGHLIGHT_BYTES {
        return None;
    }
    let (name, grammar, nodes, keywords) = grammar(language)?;
    let query_source = build_query(&grammar, nodes, keywords);
    let mut config = HighlightConfiguration::new(grammar, name, &query_source, "", "").ok()?;
    config.configure(CAPTURES);

    // Colors resolve once per capture up front; unstyled gaps paint in
    // the theme default so rows partition exactly like syntect rows.
    let foreground = theme
        .settings
        .foreground
        .unwrap_or(syntect::highlighting::Color {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        });
    let plain = [foreground.r, foreground.g, foreground.b];
    let colors: Vec<[u8; 3]> = CAPTURES
        .iter()
        .map(|capture| {
            scope_for_capture(capture).map_or(plain, |scope| color_for_scope(theme, scope))
        })
        .collect();

    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(&config, text.as_bytes(), None, None, |_| None)
        .ok()?;

    let mut out: Vec<Vec<Span>> = Vec::new();
    let mut row: Vec<Span> = Vec::new();
    // Char column and styled state of the open row.
    let mut col = 0usize;
    let mut row_touched = false;
    let mut stack: Vec<usize> = Vec::new();

    for event in events {
        let event = match event {
            Ok(event) => event,
            // Mid-stream failure (cancellation, invalid language):
            // fall back to syntect rather than half-painted text.
            Err(_) => return None,
        };
        match event {
            HighlightEvent::HighlightStart(node) => {
                stack.push(node.0);
            }
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                // The active style is the innermost capture on the stack.
                let mut color = None;
                for index in stack.iter().rev() {
                    if let Some(styled) = colors.get(*index) {
                        color = Some(*styled);
                        break;
                    }
                }
                let bytes = text.as_bytes().get(start..end).unwrap_or_default();
                let piece = std::str::from_utf8(bytes).unwrap_or("");
                // A source range can span rows: split on newlines so every
                // span stays line-local.
                for part in split_rows(piece) {
                    if !part.text.is_empty() {
                        let end = col.saturating_add(part.text.chars().count());
                        row.push(Span {
                            start: col,
                            end,
                            color: color.unwrap_or(plain),
                        });
                        col = end;
                        row_touched = true;
                    }
                    if part.newline {
                        out.push(std::mem::take(&mut row));
                        col = 0;
                        row_touched = false;
                    }
                }
            }
        }
    }
    // Row count matches `text.split('\n')`: a trailing newline leaves a
    // pending empty row, exactly like the syntect backend produces.
    if text.ends_with('\n') {
        out.push(std::mem::take(&mut row));
    } else if !row.is_empty() || row_touched || out.is_empty() {
        out.push(row);
    }
    Some(out)
}

/// One newline-delimited piece of a source range.
struct RowPart<'a> {
    text: &'a str,
    newline: bool,
}

/// Split text so every piece holds no newline; the newline itself ends
/// its piece (`newline: true`), keeping spans line-local.
fn split_rows(text: &str) -> Vec<RowPart<'_>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (byte, c) in text.char_indices() {
        if c == '\n' {
            if let Some(part) = text.get(start..byte) {
                out.push(RowPart {
                    text: part,
                    newline: true,
                });
            }
            start = byte.saturating_add(1);
        }
    }
    if let Some(rest) = text.get(start..) {
        out.push(RowPart {
            text: rest,
            newline: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_theme() -> Theme {
        let (syntaxes, themes) = crate::highlight::test_assets();
        let _ = syntaxes;
        themes
            .themes
            .get("Nord")
            .or_else(|| themes.themes.values().next())
            .cloned()
            .expect("theme")
    }

    #[test]
    fn rust_comment_and_keyword_span() {
        let theme = test_theme();
        let lines = highlight_text(Language::Rust, "// hi\nfn f() {}", &theme).expect("spans");
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0][0].start, lines[0][0].end), (0, 5));
        assert!(lines[1].iter().any(|span| span.start == 0 && span.end == 2));
    }

    #[test]
    fn python_function_spans() {
        let theme = test_theme();
        let lines = highlight_text(Language::Python, "def f():\n    pass", &theme).expect("spans");
        assert!(!lines[0].is_empty());
        assert!(!lines[1].is_empty());
    }

    #[test]
    fn rows_match_split_lines_and_partition() {
        let theme = test_theme();
        for text in ["", "fn f() {}", "fn f() {}\n", "// hi\nfn f() {}\n", "\n\n"] {
            let lines = highlight_text(Language::Rust, text, &theme).expect("spans");
            assert_eq!(lines.len(), text.split('\n').count(), "rows for {text:?}");
            for (row, spans) in lines.iter().enumerate() {
                let mut col = 0;
                for span in spans {
                    assert_eq!(span.start, col, "gap in row {row} of {text:?}");
                    col = span.end;
                }
                let width = text.split('\n').nth(row).unwrap_or("").chars().count();
                assert_eq!(col, width, "row {row} of {text:?}");
            }
        }
    }

    #[test]
    fn unsupported_language_falls_back() {
        let theme = test_theme();
        assert!(highlight_text(Language::Toml, "a = 1", &theme).is_none());
    }

    #[test]
    fn oversized_buffers_fall_back() {
        let theme = test_theme();
        let big = "x\n".repeat(MAX_HIGHLIGHT_BYTES / 2 + 1);
        assert!(highlight_text(Language::Rust, &big, &theme).is_none());
    }
}
