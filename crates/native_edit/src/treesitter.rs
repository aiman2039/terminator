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

use crate::highlight::{Language, MAX_HIGHLIGHT_BYTES, Span, SpanStyle};

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
(macro_invocation macro: (identifier) @function)
(primitive_type) @builtin_type
(type_identifier) @type
(mutable_specifier) @keyword
(enum_variant name: (identifier) @enum_variant)
(const_item name: (identifier) @constant)
(static_item name: (identifier) @constant)
(attribute_item (attribute (identifier) @attribute))
(inner_attribute_item (attribute (identifier) @attribute))
(field_expression field: (field_identifier) @property)
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
const CAPTURES: &[&str] = &[
    "comment",
    "string",
    "keyword",
    "function",
    "type",
    "number",
    "constant",
    "attribute",
    "property",
    "builtin_type",
    "enum_variant",
];

/// Capture name to syntect scope, for theme-consistent colors.
fn scope_for_capture(capture: &str) -> Option<&'static str> {
    match capture {
        "comment" => Some("comment"),
        // The bundled Darcula/OneHalf themes style concrete TextMate
        // scopes. Generic `string` and `keyword` resolve to their default
        // foreground.
        "string" => Some("string.quoted.double"),
        "keyword" => Some("keyword.control"),
        "function" => Some("entity.name.function"),
        "type" => Some("support.class"),
        "builtin_type" => Some("support.type.primitive"),
        "number" => Some("constant.numeric"),
        "constant" => Some("constant.other"),
        "enum_variant" => Some("variable.other.enummember"),
        "attribute" => Some("entity.name.annotation"),
        "property" => Some("variable.other.member"),
        _ => None,
    }
}

/// Theme foreground and emphasis for a syntect scope: strongest matching
/// rule wins, falling back to the theme default.
fn resolve_scope(theme: &Theme, scope: &str) -> ([u8; 3], SpanStyle) {
    let default = theme
        .settings
        .foreground
        .unwrap_or(syntect::highlighting::Color {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        });
    let plain = [default.r, default.g, default.b];
    let Ok(wanted) = Scope::new(scope) else {
        return (plain, SpanStyle::default());
    };
    let mut stack = ScopeStack::new();
    stack.push(wanted);
    let mut best: Option<(MatchPower, [u8; 3], SpanStyle)> = None;
    for item in &theme.scopes {
        if let Some(power) = item.scope.does_match(stack.as_slice()) {
            let color = item.style.foreground.map_or(plain, |foreground| {
                [foreground.r, foreground.g, foreground.b]
            });
            if best.is_none_or(|(best_power, _, _)| power > best_power) {
                best = Some((
                    power,
                    color,
                    item.style
                        .font_style
                        .map(crate::highlight::span_style)
                        .unwrap_or_default(),
                ));
            }
        }
    }
    best.map_or((plain, SpanStyle::default()), |(_, color, style)| {
        (color, style)
    })
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
    let plain = (
        [foreground.r, foreground.g, foreground.b],
        SpanStyle::default(),
    );
    let colors: Vec<([u8; 3], SpanStyle)> = CAPTURES
        .iter()
        .map(|capture| {
            scope_for_capture(capture).map_or(plain, |scope| resolve_scope(theme, scope))
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
                        let (color, style) = color.unwrap_or(plain);
                        row.push(Span {
                            start: col,
                            end,
                            color,
                            style,
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
    fn rust_query_captures_constants_attributes_and_fields() {
        let theme = crate::highlight::theme(true).expect("darcula");
        let text = "#[derive(Debug)]\npub const SCALE: u64 = 1_000_000;\nstruct S { a: u64 }\nfn f(s: &S) -> u64 { s.a }\npub enum E {\n    Variant,\n}\n";
        let lines = highlight_text(Language::Rust, text, theme).expect("spans");
        let plain = theme.settings.foreground.expect("foreground");
        let plain = [plain.r, plain.g, plain.b];

        let span_at = |row: usize, col: usize| {
            lines[row]
                .iter()
                .find(|span| span.start <= col && col < span.end)
        };
        // `derive` attribute, and `SCALE` constant name.
        assert!(
            span_at(0, 2).is_some_and(|span| span.color != plain),
            "attribute"
        );
        assert!(
            span_at(1, 10).is_some_and(|span| span.color != plain),
            "const"
        );
        // User-defined type tints and italicizes; primitives stay upright.
        let user_type = span_at(2, 7).expect("type span");
        assert!(user_type.style.italic, "type italic");
        assert_ne!(user_type.color, plain, "type tint");
        let primitive = span_at(2, 14).expect("primitive span");
        assert_ne!(primitive.color, plain, "primitive tint");
        assert_ne!(primitive.color, user_type.color, "type vs primitive");
        assert!(!primitive.style.italic, "primitives are upright");
        // Enum variants are italic, like `IntelliJ`.
        let variant = span_at(5, 4).expect("variant span");
        assert!(variant.style.italic, "enum variant italic");
        assert_ne!(variant.color, plain, "enum variant tint");
        // Field access `s.a`, not the struct declaration.
        let field_line = "fn f(s: &S) -> u64 { s.a }";
        assert_eq!(text.split('\n').nth(3), Some(field_line));
        let field_col = field_line.find('a').expect("field char");
        assert!(
            span_at(3, field_col).is_some_and(|span| span.color != plain),
            "field access"
        );
        assert_eq!(lines.len(), text.split('\n').count());
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
