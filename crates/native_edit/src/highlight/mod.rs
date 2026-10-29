//! syntect-backed syntax highlighting over two-face assets.
//!
//! The assets (syntax dump, theme dump) load once per process; per-buffer
//! work is a full parse cached by [`HighlightCache`] on the buffer
//! revision, so frames after an edit cost nothing. Buffers over
//! [`MAX_HIGHLIGHT_BYTES`] paint plain (large-file degradation).
use crate::doc::Buffer;
use crate::rows::{RowCache, VisualRow};
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
        "sh" | "bash" | "zsh" | "ksh" => Language::Shell,
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

/// `IntelliJ` Darcula's syntax palette, bundled so code colors match the
/// IDE chrome instead of a third-party scheme.
fn darcula() -> Option<&'static syntect::highlighting::Theme> {
    static DARCULA: OnceLock<Option<syntect::highlighting::Theme>> = OnceLock::new();
    DARCULA
        .get_or_init(|| {
            let mut reader = std::io::Cursor::new(include_bytes!("../../assets/Darcula.tmTheme"));
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

/// Revision-keyed highlight cache: one per open buffer, owned by the app
/// next to the buffer it derives from. The row layout lives here too: the
/// app owns exactly one cache per buffer, and both derivations share the
/// same revision key.
#[derive(Default)]
pub struct HighlightCache {
    language: Option<Language>,
    dark: bool,
    revision: u64,
    ready: bool,
    lines: Vec<Vec<Span>>,
    rows: RowCache,
    folds: crate::fold::Folds,
}

pub struct EnsureInput<'a> {
    pub language: Language,
    pub dark: bool,
    pub revision: u64,
    pub text: &'a str,
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
    pub fn ensure(&mut self, input: EnsureInput<'_>) {
        let EnsureInput {
            language,
            dark,
            revision,
            text,
        } = input;
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

    /// Lay out the painted rows for this revision and wrap width, reusing
    /// the previous frame's rows until either changes.
    pub fn ensure_rows<B: Buffer>(&mut self, doc: &B, wrap_columns: Option<usize>) {
        self.ensure_rows_spaced(doc, wrap_columns, &[]);
    }

    /// [`HighlightCache::ensure_rows`] with host filler rows before some
    /// lines (see [`crate::spacers`]).
    pub fn ensure_rows_spaced<B: Buffer>(
        &mut self,
        doc: &B,
        wrap_columns: Option<usize>,
        spacers: &[(usize, usize)],
    ) {
        self.folds
            .ensure(doc, self.language.unwrap_or(Language::Plain));
        self.rows.ensure_spaced(
            doc,
            crate::rows::RowLayout {
                wrap_columns,
                folds: Some(&self.folds),
                spacers,
            },
        );
    }

    /// This view's code folds.
    #[must_use]
    pub fn folds(&self) -> &crate::fold::Folds {
        &self.folds
    }

    pub fn folds_mut(&mut self) -> &mut crate::fold::Folds {
        &mut self.folds
    }

    /// Rows laid out by the last [`HighlightCache::ensure_rows`].
    #[must_use]
    pub fn rows(&self) -> &[VisualRow] {
        self.rows.rows()
    }

    /// Widest laid-out row in display cells, for the horizontal extent.
    #[must_use]
    pub fn max_cells(&self) -> usize {
        self.rows.max_cells()
    }
}

#[cfg(test)]
pub(crate) fn test_assets() -> &'static (SyntaxSet, ThemeSet) {
    assets()
}

#[cfg(test)]
mod tests;
