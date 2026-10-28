//! Syntax highlighting for diff sides, and word-change overlays.
use super::*;

/// Syntax-highlighted spans per line, for the in-diff editor.
pub(crate) fn highlight_spans(path: &Path, text: &str) -> Vec<Vec<DiffSpan>> {
    highlight_file(path, text)
}

fn syntaxes() -> &'static SyntaxSet {
    terminator_native_edit::highlight::syntax_set()
}

fn theme() -> &'static Theme {
    static FALLBACK: OnceLock<Theme> = OnceLock::new();
    terminator_native_edit::highlight::theme(DARK.load(Ordering::Relaxed))
        .unwrap_or_else(|| FALLBACK.get_or_init(Theme::default))
}

pub(super) fn highlight_file(path: &Path, text: &str) -> Vec<Vec<DiffSpan>> {
    if text.len() > MAX_HIGHLIGHT_BYTES {
        // Plain rows: `line_spans` falls back to the theme foreground.
        return Vec::new();
    }
    let set = syntaxes();
    let syntax = syntax_for(path, set);
    let mut highlighter = HighlightLines::new(syntax, theme());
    let fallback = fallback_rgb();
    let mut lines = Vec::new();
    for line in LinesWithEndings::from(text) {
        let ranges = highlighter
            .highlight_line(line, set)
            .unwrap_or_else(|_| vec![(syntect::highlighting::Style::default(), line)]);
        let mut spans = Vec::new();
        for (style, piece) in ranges {
            let piece = piece.trim_end_matches(['\n', '\r']);
            if piece.is_empty() {
                continue;
            }
            spans.push(DiffSpan {
                text: piece.to_owned(),
                rgb: [style.foreground.r, style.foreground.g, style.foreground.b],
                intra: Intra::None,
            });
        }
        if spans.is_empty() {
            spans.push(DiffSpan {
                text: String::new(),
                rgb: fallback,
                intra: Intra::None,
            });
        }
        lines.push(spans);
    }
    if text.is_empty() {
        lines.clear();
    }
    lines
}

fn syntax_for<'a>(path: &Path, _set: &'a SyntaxSet) -> &'a SyntaxReference {
    terminator_native_edit::highlight::syntax_for_path(&path.to_string_lossy())
}

pub(super) fn fallback_rgb() -> [u8; 3] {
    let color = theme()
        .settings
        .foreground
        .unwrap_or(syntect::highlighting::Color {
            r: 192,
            g: 197,
            b: 206,
            a: 255,
        });
    [color.r, color.g, color.b]
}

pub(super) fn line_spans(
    highlighted: &[Vec<DiffSpan>],
    index: Option<usize>,
    text: &str,
) -> Vec<DiffSpan> {
    let trimmed = text.trim_end_matches(['\n', '\r']);
    index
        .and_then(|i| highlighted.get(i).cloned())
        .filter(|spans| spans.iter().map(|s| s.text.as_str()).collect::<String>() == trimmed)
        .unwrap_or_else(|| {
            vec![DiffSpan {
                text: trimmed.to_owned(),
                rgb: fallback_rgb(),
                intra: Intra::None,
            }]
        })
}

pub(super) fn overlay(spans: Vec<DiffSpan>, fragments: &[(bool, String)]) -> Vec<DiffSpan> {
    let joined: String = fragments.iter().map(|(_, t)| t.as_str()).collect();
    let original: String = spans.iter().map(|s| s.text.as_str()).collect();
    if joined != original {
        return spans;
    }
    let mut colors = Vec::with_capacity(original.len());
    for span in &spans {
        for _ in 0..span.text.len() {
            colors.push(span.rgb);
        }
    }
    let mut out = Vec::new();
    let mut offset = 0;
    for (changed, text) in fragments {
        if text.is_empty() {
            continue;
        }
        let rgb = colors.get(offset).copied().unwrap_or_else(fallback_rgb);
        out.push(DiffSpan {
            text: text.clone(),
            rgb,
            intra: if *changed { Intra::Change } else { Intra::None },
        });
        offset = offset.saturating_add(text.len());
    }
    if out.is_empty() { spans } else { out }
}
