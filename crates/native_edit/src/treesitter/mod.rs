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
mod outline;
#[cfg(test)]
mod tests;
pub use outline::outline;
use std::collections::HashMap;
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

/// Compiled highlight configuration per language. Building one compiles
/// the grammar's queries, far costlier than highlighting a typical file,
/// so it happens once per process instead of on every edit. A query the
/// grammar rejects is cached as `None` (syntect handles that language).
fn config_for(language: Language) -> Option<std::sync::Arc<HighlightConfiguration>> {
    type Configs = HashMap<&'static str, Option<std::sync::Arc<HighlightConfiguration>>>;
    static CONFIGS: std::sync::OnceLock<std::sync::Mutex<Configs>> = std::sync::OnceLock::new();
    let (name, grammar, nodes, keywords) = grammar(language)?;
    let configs = CONFIGS.get_or_init(Default::default);
    let mut configs = configs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    configs
        .entry(name)
        .or_insert_with(|| {
            let query_source = build_query(&grammar, nodes, keywords);
            let mut config =
                HighlightConfiguration::new(grammar, name, &query_source, "", "").ok()?;
            config.configure(CAPTURES);
            Some(std::sync::Arc::new(config))
        })
        .clone()
}

/// Highlight text into per-line spans. `None` means fall back to syntect:
/// unsupported language, oversized buffer, bad query, grammar mismatch,
/// or highlight-iteration failure.
#[must_use]
pub fn highlight_text(language: Language, text: &str, theme: &Theme) -> Option<Vec<Vec<Span>>> {
    if text.len() > MAX_HIGHLIGHT_BYTES {
        return None;
    }
    let config = config_for(language)?;
    let palette = CapturePalette::new(theme);
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(&config, text.as_bytes(), None, None, |_| None)
        .ok()?;
    let mut rows = SpanRows::default();
    let mut stack: Vec<usize> = Vec::new();
    for event in events {
        // Mid-stream failure (cancellation, invalid language): fall back
        // to syntect rather than half-painted text.
        match event.ok()? {
            HighlightEvent::HighlightStart(node) => stack.push(node.0),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let bytes = text.as_bytes().get(start..end).unwrap_or_default();
                let piece = std::str::from_utf8(bytes).unwrap_or("");
                rows.push_source(piece, palette.active(&stack));
            }
        }
    }
    Some(rows.finish(text))
}

/// Colors resolve once per capture up front; unstyled gaps paint in the
/// theme default so rows partition exactly like syntect rows.
struct CapturePalette {
    plain: ([u8; 3], SpanStyle),
    colors: Vec<([u8; 3], SpanStyle)>,
}

impl CapturePalette {
    fn new(theme: &Theme) -> Self {
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
        let colors = CAPTURES
            .iter()
            .map(|capture| {
                scope_for_capture(capture).map_or(plain, |scope| resolve_scope(theme, scope))
            })
            .collect();
        Self { plain, colors }
    }

    /// The active style is the innermost capture on the stack.
    fn active(&self, stack: &[usize]) -> ([u8; 3], SpanStyle) {
        stack
            .iter()
            .rev()
            .find_map(|index| self.colors.get(*index).copied())
            .unwrap_or(self.plain)
    }
}

/// Per-line spans under construction: finished rows plus the open one.
#[derive(Default)]
struct SpanRows {
    out: Vec<Vec<Span>>,
    row: Vec<Span>,
    /// Char column of the open row.
    col: usize,
    /// Whether the open row received any source.
    row_touched: bool,
}

impl SpanRows {
    /// A source range can span rows: split on newlines so every span
    /// stays line-local.
    fn push_source(&mut self, piece: &str, (color, style): ([u8; 3], SpanStyle)) {
        for part in split_rows(piece) {
            if !part.text.is_empty() {
                let end = self.col.saturating_add(part.text.chars().count());
                self.row.push(Span {
                    start: self.col,
                    end,
                    color,
                    style,
                });
                self.col = end;
                self.row_touched = true;
            }
            if part.newline {
                self.out.push(std::mem::take(&mut self.row));
                self.col = 0;
                self.row_touched = false;
            }
        }
    }

    /// Row count matches `text.split('\n')`: a trailing newline leaves a
    /// pending empty row, exactly like the syntect backend produces.
    fn finish(mut self, text: &str) -> Vec<Vec<Span>> {
        if text.ends_with('\n') || !self.row.is_empty() || self.row_touched || self.out.is_empty() {
            self.out.push(self.row);
        }
        self.out
    }
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
