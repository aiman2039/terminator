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
        "sh" | "bash" | "zsh" => Language::Shell,
        "json" | "jsonc" | "jsonl" | "ndjson" => Language::Json,
        "toml" => Language::Toml,
        "yaml" | "yml" => Language::Yaml,
        "md" | "markdown" => Language::Markdown,
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

/// `IntelliJ` Darcula's syntax palette, bundled so code colors match the
/// IDE chrome instead of a third-party scheme.
fn darcula() -> Option<&'static syntect::highlighting::Theme> {
    static DARCULA: OnceLock<Option<syntect::highlighting::Theme>> = OnceLock::new();
    DARCULA
        .get_or_init(|| {
            let mut reader = std::io::Cursor::new(include_bytes!("../assets/Darcula.tmTheme"));
            ThemeSet::load_from_reader(&mut reader).ok()
        })
        .as_ref()
}

/// Active syntect theme, so sibling backends (tree-sitter) resolve the
/// same colors the regex backend paints. `None` when the bundle holds
/// no themes at all (then the caller falls back to syntect-or-plain).
pub fn theme(dark: bool) -> Option<&'static syntect::highlighting::Theme> {
    if dark {
        return darcula();
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
    // tree-sitter first for supported languages (returns None when
    // the language is unsupported, so this doubles as the check);
    // syntect fallback.
    if let Some(native_theme) = theme(dark)
        && let Some(spans) = crate::treesitter::highlight_text(language, text, native_theme)
    {
        return Some(spans);
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
                    style: span_style(style.font_style),
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
        assert_ne!(language_for_path("Makefile"), Language::Plain);
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
    fn dark_theme_uses_darcula_colors() {
        let lines = highlight_text(
            Language::Rust,
            "fn main() {\n    // note\n    let x = 1;\n}",
            true,
        )
        .expect("rust highlights");
        let has = |line: usize, color: [u8; 3]| lines[line].iter().any(|span| span.color == color);
        assert!(has(0, [0xCC, 0x78, 0x32]), "keyword orange: {:?}", lines[0]);
        assert!(
            has(0, [0xFF, 0xC6, 0x6D]),
            "function yellow: {:?}",
            lines[0]
        );
        assert!(has(1, [0x80, 0x80, 0x80]), "comment gray: {:?}", lines[1]);
        assert!(has(2, [0x68, 0x97, 0xBB]), "number blue: {:?}", lines[2]);
        assert!(!has(3, [0x80, 0x80, 0x80]));
        assert!(
            lines[2].iter().any(|span| span.color == [0xA9, 0xB7, 0xC6]),
            "plain text stays Darcula foreground: {:?}",
            lines[2]
        );
    }

    #[test]
    fn javascript_config_has_keyword_and_string_colors() {
        let lines = highlight_text(
            Language::JavaScript,
            "import js from \"@eslint/js\";\nconst value = 42;",
            true,
        )
        .expect("JavaScript highlights");
        let mut colors: Vec<[u8; 3]> = lines
            .iter()
            .flat_map(|line| line.iter().map(|span| span.color))
            .collect();
        colors.sort_unstable();
        colors.dedup();
        assert!(
            colors.len() > 1,
            "JavaScript painted in one color: {lines:?}"
        );
        let color_at = |line: usize, column: usize| {
            lines[line]
                .iter()
                .find(|span| span.start <= column && column < span.end)
                .map(|span| span.color)
                .expect("colored position")
        };
        assert_ne!(color_at(0, 0), color_at(0, 7), "import keyword is plain");
        assert_ne!(color_at(0, 16), color_at(0, 7), "module string is plain");
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
    fn jsonc_highlights_comments_and_keeps_urls_as_strings() {
        assert_eq!(language_for_path("wrangler.jsonc"), Language::Json);
        let text = "{\n  // Comment\n  \"url\": \"https://example.test\",\n  /* Block */ \"enabled\": true,\n}\n";
        let lines = highlight_text(language_for_path("wrangler.jsonc"), text, true)
            .expect("JSONC highlight");
        assert!(lines[1].iter().any(|span| span.color == [0x80, 0x80, 0x80]));
        assert!(lines[3].iter().any(|span| span.color == [0x80, 0x80, 0x80]));
        assert!(lines[2].iter().any(|span| span.color == [0x6A, 0x87, 0x59]));
    }

    #[test]
    fn common_format_inventory_uses_the_bundled_grammars() {
        for path in [
            "index.html",
            "style.css",
            "style.scss",
            "query.sql",
            "Dockerfile",
            "Makefile",
            "query.graphql",
            "main.lua",
            "main.rb",
            "main.php",
            "main.swift",
            "main.kt",
            "main.cs",
            "main.tf",
            ".env",
            "config.xml",
            "app.vue",
        ] {
            let language = language_for_path(path);
            assert_ne!(language, Language::Plain, "missing grammar: {path}");
            assert!(
                highlight_text(language, "value = 1", true).is_some(),
                "missing highlight: {path}"
            );
        }
        for path in [
            "wrangler.jsonc",
            "view.astro",
            "view.svelte",
            "notes.mdx",
            "data.json5",
            "build.zig",
            "main.sol",
        ] {
            println!("FORMAT {path}: {:?}", language_for_path(path));
        }
        println!("BUNDLED GRAMMARS: {}", syntax_set().syntaxes().len());
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
