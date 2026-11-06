//! syntect-backed syntax highlighting over two-face assets.
//!
//! The assets (syntax dump, theme dump) load once per process; per-buffer
//! work is a full parse cached by [`HighlightCache`] on the buffer
//! revision, so frames after an edit cost nothing. Buffers over
//! [`SYNC_HIGHLIGHT_BYTES`] parse on a worker thread; buffers over
//! [`MAX_HIGHLIGHT_BYTES`] paint plain (large-file degradation).
use std::sync::OnceLock;
use syntect::highlighting::{HighlightIterator, HighlightState, Highlighter, ThemeSet};
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};

/// Languages with bundled syntax definitions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
    C,
    Cpp,
    Java,
    Shell,
    Json,
    Toml,
    Yaml,
    Markdown,
    Html,
    Css,
    Scss,
    Less,
    /// Any other grammar bundled by two-face (its name has process lifetime).
    Syntax(&'static str),
    Plain,
}

/// File extension to highlight language. Headers (`.h`) read as C;
/// ambiguity there is inherent and cheap to revisit per file later.
#[must_use]
pub fn language_for_path(path: &str) -> Language {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "rs" => Language::Rust,
        "py" => Language::Python,
        "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
        "ts" | "tsx" | "mts" | "cts" => Language::TypeScript,
        "go" => Language::Go,
        "c" | "h" => Language::C,
        "cc" | "cpp" | "cxx" | "hh" | "hpp" => Language::Cpp,
        "java" => Language::Java,
        "sh" | "bash" | "zsh" | "ksh" => Language::Shell,
        "json" | "jsonc" | "jsonl" | "ndjson" => Language::Json,
        "toml" => Language::Toml,
        "yaml" | "yml" => Language::Yaml,
        "md" | "markdown" => Language::Markdown,
        "html" | "htm" | "xhtml" => Language::Html,
        "css" => Language::Css,
        "scss" => Language::Scss,
        "less" => Language::Less,
        _ => {
            let syntax = syntax_for_path(path);
            if syntax.name == "Plain Text" {
                Language::Plain
            } else {
                Language::Syntax(syntax.name.as_str())
            }
        }
    }
}

impl Language {
    fn extension(self) -> Option<&'static str> {
        match self {
            Language::Rust => Some("rs"),
            Language::Python => Some("py"),
            Language::JavaScript => Some("js"),
            Language::TypeScript => Some("ts"),
            Language::Go => Some("go"),
            Language::C => Some("c"),
            Language::Cpp => Some("cpp"),
            Language::Java => Some("java"),
            Language::Shell => Some("sh"),
            Language::Json => Some("json"),
            Language::Toml => Some("toml"),
            Language::Yaml => Some("yaml"),
            Language::Markdown => Some("md"),
            Language::Html => Some("html"),
            Language::Css => Some("css"),
            Language::Scss => Some("scss"),
            Language::Less => Some("less"),
            Language::Plain | Language::Syntax(_) => None,
        }
    }
}

/// One colored span: char-based, line-local, end-exclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub color: [u8; 3],
    pub style: SpanStyle,
}

/// Font emphasis for a span. Themes carry it per scope; the view picks an
/// italic/bold font family for spans that ask for one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SpanStyle {
    pub italic: bool,
    pub bold: bool,
}

/// Convert a syntect [`FontStyle`](syntect::highlighting::FontStyle) bit set.
pub(crate) fn span_style(font_style: syntect::highlighting::FontStyle) -> SpanStyle {
    use syntect::highlighting::FontStyle;
    SpanStyle {
        italic: font_style.contains(FontStyle::ITALIC),
        bold: font_style.contains(FontStyle::BOLD),
    }
}

/// Buffers over this size paint plain instead of highlighted.
pub const MAX_HIGHLIGHT_BYTES: usize = 256 * 1024;

/// Buffers up to this size highlight on the calling thread; larger ones
/// highlight on a worker so a keystroke never waits for a full parse.
pub const SYNC_HIGHLIGHT_BYTES: usize = 32 * 1024;

/// Process-wide syntax/theme assets, for sibling backends (tree-sitter
/// resolves its colors from the same theme).
pub(crate) fn assets() -> &'static (SyntaxSet, ThemeSet) {
    static ASSETS: OnceLock<(SyntaxSet, ThemeSet)> = OnceLock::new();
    ASSETS.get_or_init(|| {
        (
            two_face::syntax::extra_newlines(),
            two_face::theme::extra().into(),
        )
    })
}

/// Shared bundled grammars for the native editor and Git snapshots.
#[must_use]
pub fn syntax_set() -> &'static SyntaxSet {
    &assets().0
}

/// Detect extensions and common extensionless configuration filenames.
#[must_use]
pub fn syntax_for_path(path: &str) -> &'static syntect::parsing::SyntaxReference {
    let set = syntax_set();
    let path = std::path::Path::new(path);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let lower = name.to_ascii_lowercase();
    let named =
        if lower == "dockerfile" || lower.starts_with("dockerfile.") || lower == "containerfile" {
            Some("Dockerfile")
        } else if matches!(lower.as_str(), "makefile" | "gnumakefile") {
            Some("Makefile")
        } else if lower == ".env" || lower.starts_with(".env.") {
            Some("DotENV")
        } else {
            None
        };
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    named
        .and_then(|name| set.find_syntax_by_name(name))
        .or_else(|| {
            (ext == "ksh")
                .then(|| set.find_syntax_by_extension("sh"))
                .flatten()
        })
        .or_else(|| set.find_syntax_by_extension(&ext))
        .or_else(|| {
            matches!(ext.as_str(), "jsonc" | "jsonl" | "ndjson")
                .then(|| set.find_syntax_by_extension("json"))
                .flatten()
        })
        .or_else(|| set.find_syntax_by_extension(name))
        .or_else(|| set.find_syntax_by_extension(name.trim_start_matches('.')))
        .unwrap_or_else(|| set.find_syntax_plain_text())
}

/// `IntelliJ` "Islands Dark" syntax palette, bundled so code colors match
/// the IDE chrome instead of a third-party scheme.
fn islands_dark() -> Option<&'static syntect::highlighting::Theme> {
    static ISLANDS_DARK: OnceLock<Option<syntect::highlighting::Theme>> = OnceLock::new();
    ISLANDS_DARK
        .get_or_init(|| {
            let mut reader =
                std::io::Cursor::new(include_bytes!("../../assets/IslandsDark.tmTheme"));
            ThemeSet::load_from_reader(&mut reader).ok()
        })
        .as_ref()
}

/// Active syntect theme, so sibling backends (tree-sitter) resolve the
/// same colors the regex backend paints. `None` when the bundle holds
/// no themes at all (then the caller falls back to syntect-or-plain).
pub fn theme(dark: bool) -> Option<&'static syntect::highlighting::Theme> {
    if dark {
        return islands_dark();
    }
    let (_, themes) = assets();
    themes
        .themes
        .get("OneHalfLight")
        .or_else(|| themes.themes.values().next())
}

/// Highlight whole text into per-line spans. `None` means paint plain:
/// over the size cap, plain language, or missing syntax/theme.
#[must_use]
pub fn highlight_text(language: Language, text: &str, dark: bool) -> Option<Vec<Vec<Span>>> {
    if text.len() > MAX_HIGHLIGHT_BYTES {
        return None;
    }
    highlight_lf(language, &crate::doc::normalize_breaks(text), dark)
}

/// [`highlight_text`] for text already split with `\n` only (see
/// [`crate::doc::normalize_breaks`]), with no size cap.
pub(crate) fn highlight_lf(language: Language, text: &str, dark: bool) -> Option<Vec<Vec<Span>>> {
    highlight_lf_kept(language, text, dark, None)
}

/// [`highlight_lf`]; with `syntax`, tree-sitter re-parses only the edit
/// since the last text kept there (see `treesitter::highlight_incremental`).
pub(crate) fn highlight_lf_kept(
    language: Language,
    text: &str,
    dark: bool,
    syntax: Option<&mut Option<crate::treesitter::SyntaxState>>,
) -> Option<Vec<Vec<Span>>> {
    // tree-sitter first for supported languages (returns None when
    // the language is unsupported, so this doubles as the check);
    // syntect fallback.
    if let Some(native_theme) = theme(dark) {
        let spans = match syntax {
            Some(state) => {
                crate::treesitter::highlight_incremental(language, text, native_theme, state)
            }
            None => crate::treesitter::highlight_text(language, text, native_theme),
        };
        if spans.is_some() {
            return spans;
        }
    }
    let (syntaxes, _) = assets();
    let syntax = match language {
        Language::Syntax(name) => syntaxes.find_syntax_by_name(name)?,
        _ => syntaxes.find_syntax_by_extension(language.extension()?)?,
    };
    let theme = theme(dark)?;
    let highlighter = Highlighter::new(theme);
    let mut highlight_state = HighlightState::new(&highlighter, ScopeStack::new());
    let mut parser = ParseState::new(syntax);
    let mut out = Vec::new();
    // These grammars require line endings to close line comments and other
    // line-scoped contexts. Keep them for parsing, but omit LF from view spans.
    for line in syntect::util::LinesWithEndings::from(text) {
        let visible_chars = line.strip_suffix('\n').unwrap_or(line).chars().count();
        let ops = parser.parse_line(line, syntaxes).unwrap_or_default();
        let iter = HighlightIterator::new(&mut highlight_state, &ops, line, &highlighter);
        let mut spans = Vec::new();
        let mut col = 0usize;
        for (style, piece) in iter {
            let len = piece.chars().count();
            let end = col.saturating_add(len).min(visible_chars);
            if end > col {
                let foreground = style.foreground;
                spans.push(Span {
                    start: col,
                    end,
                    color: [foreground.r, foreground.g, foreground.b],
                    style: span_style(style.font_style),
                });
            }
            col = end;
        }
        out.push(spans);
    }
    if text.is_empty() || text.ends_with('\n') {
        out.push(Vec::new());
    }
    Some(out)
}

#[cfg(test)]
pub(crate) fn test_assets() -> &'static (SyntaxSet, ThemeSet) {
    assets()
}

mod cache;
mod rainbow;

pub use cache::{EnsureInput, HighlightCache};

#[cfg(test)]
mod tests;
