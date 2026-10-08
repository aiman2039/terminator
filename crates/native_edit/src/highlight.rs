//! syntect-backed syntax highlighting over two-face assets.
//!
//! The assets (syntax dump, theme dump) load once per process; per-buffer
//! work is a full parse cached by [`HighlightCache`] on the buffer
//! revision, so frames after an edit cost nothing. Buffers over
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
        "js" | "jsx" | "mjs" => Language::JavaScript,
        "ts" | "tsx" => Language::TypeScript,
        "go" => Language::Go,
        "c" | "h" => Language::C,
        "cc" | "cpp" | "cxx" | "hh" | "hpp" => Language::Cpp,
        "java" => Language::Java,
        "sh" | "bash" | "zsh" => Language::Shell,
        "json" => Language::Json,
        "toml" => Language::Toml,
        "yaml" | "yml" => Language::Yaml,
        "md" | "markdown" => Language::Markdown,
        _ => Language::Plain,
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
            Language::Plain => None,
        }
    }
}

/// One colored span: char-based, line-local, end-exclusive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub color: [u8; 3],
}

/// Buffers over this size paint plain instead of highlighted.
pub const MAX_HIGHLIGHT_BYTES: usize = 256 * 1024;

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

fn theme_name(dark: bool) -> &'static str {
    if dark { "Nord" } else { "OneHalfLight" }
}

/// Active syntect theme, so sibling backends (tree-sitter) resolve the
/// same colors the regex backend paints. `None` when the bundle holds
/// no themes at all (then the caller falls back to syntect-or-plain).
pub(crate) fn theme(dark: bool) -> Option<&'static syntect::highlighting::Theme> {
    let (_, themes) = assets();
    themes
        .themes
        .get(theme_name(dark))
        .or_else(|| themes.themes.values().next())
}

/// Highlight whole text into per-line spans. `None` means paint plain:
/// over the size cap, plain language, or missing syntax/theme.
#[must_use]
pub fn highlight_text(language: Language, text: &str, dark: bool) -> Option<Vec<Vec<Span>>> {
    if text.len() > MAX_HIGHLIGHT_BYTES {
        return None;
    }
    // tree-sitter first for supported languages (returns None when
    // the language is unsupported, so this doubles as the check);
    // syntect fallback.
    if let Some(native_theme) = theme(dark)
        && let Some(spans) = crate::treesitter::highlight_text(language, text, native_theme)
    {
        return Some(spans);
    }
    let ext = language.extension()?;
    let (syntaxes, themes) = assets();
    let syntax = syntaxes.find_syntax_by_extension(ext)?;
    let theme = themes
        .themes
        .get(theme_name(dark))
        .or_else(|| themes.themes.values().next())?;
    let highlighter = Highlighter::new(theme);
    let mut highlight_state = HighlightState::new(&highlighter, ScopeStack::new());
    let mut parser = ParseState::new(syntax);
    let mut out = Vec::new();
    // syntect lines exclude the terminator; parser state carries across
    // lines, so multi-line strings and comments still highlight.
    for line in text.split('\n') {
        let ops = parser.parse_line(line, syntaxes).unwrap_or_default();
        let iter = HighlightIterator::new(&mut highlight_state, &ops, line, &highlighter);
        let mut spans = Vec::new();
        let mut col = 0usize;
        for (style, piece) in iter {
            let len = piece.chars().count();
            let end = col.saturating_add(len);
            if end > col {
                let foreground = style.foreground;
                spans.push(Span {
                    start: col,
                    end,
                    color: [foreground.r, foreground.g, foreground.b],
                });
            }
            col = end;
        }
        out.push(spans);
    }
    Some(out)
}

/// Revision-keyed highlight cache: one per open buffer, owned by the app
/// next to the buffer it derives from.
#[derive(Default)]
pub struct HighlightCache {
    language: Option<Language>,
    dark: bool,
    revision: u64,
    ready: bool,
    lines: Vec<Vec<Span>>,
}

impl HighlightCache {
    /// True when the cache already holds this exact revision: the view
    /// can skip rebuilding the buffer text entirely.
    #[must_use]
    pub fn is_current(&self, language: Language, dark: bool, revision: u64) -> bool {
        self.ready
            && self.language == Some(language)
            && self.dark == dark
            && self.revision == revision
    }

    /// Recompute when the key changed; over-cap and plain buffers cache
    /// as empty (every row paints plain).
    pub fn ensure(&mut self, language: Language, dark: bool, revision: u64, text: &str) {
        if self.is_current(language, dark, revision) {
            return;
        }
        self.language = Some(language);
        self.dark = dark;
        self.revision = revision;
        self.ready = true;
        self.lines = highlight_text(language, text, dark).unwrap_or_default();
    }

    /// Highlight spans for one row; empty when plain or out of range.
    #[must_use]
    pub fn line(&self, line: usize) -> &[Span] {
        match self.lines.get(line) {
            Some(spans) => spans,
            None => &[],
        }
    }
}

#[cfg(test)]
pub(crate) fn test_assets() -> &'static (SyntaxSet, ThemeSet) {
    assets()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_map_to_languages() {
        assert_eq!(language_for_path("main.rs"), Language::Rust);
        assert_eq!(language_for_path("app.py"), Language::Python);
        assert_eq!(language_for_path("ui.tsx"), Language::TypeScript);
        assert_eq!(language_for_path("data.yaml"), Language::Yaml);
        assert_eq!(language_for_path("notes.md"), Language::Markdown);
        assert_eq!(language_for_path("nope.zq"), Language::Plain);
        assert_eq!(language_for_path("Makefile"), Language::Plain);
    }

    #[test]
    fn rust_keyword_gets_own_span() {
        let lines = highlight_text(Language::Rust, "fn main() {}", true).expect("rust highlights");
        assert_eq!(lines.len(), 1);
        let first = lines[0].first().expect("first span");
        assert_eq!((first.start, first.end), (0, 2));
        // A real theme paints more than one color on this line.
        let mut colors: Vec<[u8; 3]> = lines[0].iter().map(|span| span.color).collect();
        colors.sort_unstable();
        colors.dedup();
        assert!(colors.len() > 1, "spans: {:?}", lines[0]);
    }

    #[test]
    fn spans_partition_the_whole_line() {
        let lines = highlight_text(Language::Rust, "let x = 1;", true).expect("rust highlights");
        let mut col = 0;
        for span in &lines[0] {
            assert_eq!(span.start, col);
            col = span.end;
        }
        assert_eq!(col, "let x = 1;".chars().count());
    }

    #[test]
    fn oversized_buffers_paint_plain() {
        let big = "x\n".repeat(MAX_HIGHLIGHT_BYTES / 2 + 1);
        assert!(highlight_text(Language::Rust, &big, true).is_none());
    }

    #[test]
    fn plain_language_paints_plain() {
        assert!(highlight_text(Language::Plain, "fn main() {}", true).is_none());
    }

    #[test]
    fn cache_only_recomputes_on_key_change() {
        let mut cache = HighlightCache::default();
        cache.ensure(Language::Rust, true, 1, "fn a() {}");
        let first = cache.line(0).to_vec();
        assert!(!first.is_empty());
        cache.ensure(Language::Rust, true, 1, "fn replaced() {}");
        assert_eq!(cache.line(0), first.as_slice());
        cache.ensure(Language::Rust, true, 2, "fn b() {}");
        assert_eq!(cache.line(0).len(), first.len());
        assert!(cache.is_current(Language::Rust, true, 2));
        assert!(!cache.is_current(Language::Rust, false, 2));
    }
}
