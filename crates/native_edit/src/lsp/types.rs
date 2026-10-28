/// Zero-based line/character position. Characters are UTF-16 code units
/// on the wire (see [`char_col_to_utf16`]); lines are always `\n`-based.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// Half-open range of [`Position`]s.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// Diagnostic severity, strongest first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

/// One published diagnostic.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: Option<Severity>,
    pub message: String,
}

/// Hover text with its presentation format preserved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hover {
    pub text: String,
    pub markdown: bool,
}

/// One go-to-definition target: server URI plus range.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GotoTarget {
    pub uri: String,
    pub range: Range,
}

/// One replacement proposed by a language server during a symbol rename.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RenameTextEdit {
    pub range: Range,
    pub new_text: String,
}

/// All edits for one file. A server version, when present, must still
/// match before the host applies the rename.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RenameFileEdit {
    pub uri: String,
    pub version: Option<i32>,
    pub edits: Vec<RenameTextEdit>,
}

/// One document symbol, flattened from the server's tree: `depth` is the
/// nesting level (0 for top-level items) for outline indentation. `range`
/// spans the whole item (doc comments and attributes included) for
/// containment; `selection` is the name, where a jump should land.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub range: Range,
    pub selection: Range,
    pub depth: usize,
    /// Server detail (a signature or type), shown dimmed beside the name.
    pub detail: Option<String>,
    pub deprecated: bool,
}

/// Index of the deepest symbol whose range contains `line`: the caret's
/// enclosing item, for breadcrumbs and Structure autoscroll.
#[must_use]
pub fn innermost_symbol(symbols: &[Symbol], line: u32) -> Option<usize> {
    symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| symbol.range.start.line <= line && line <= symbol.range.end.line)
        .max_by_key(|(_, symbol)| (symbol.depth, symbol.range.start.line))
        .map(|(index, _)| index)
}

/// Markdown outline: one symbol per heading, nested by level. A heading's
/// range runs to the line before the next heading at its level or above,
/// so the caret's section is its innermost symbol.
#[must_use]
pub fn markdown_outline<B: crate::doc::Buffer>(doc: &B) -> Vec<Symbol> {
    use crate::blocks::{BlockKind, parse_blocks};
    let headings: Vec<(u8, usize)> = parse_blocks(doc)
        .into_iter()
        .filter_map(|block| match block.kind {
            BlockKind::Heading(level) => Some((level, block.start_line)),
            _ => None,
        })
        .collect();
    let last_line = u32::try_from(doc.line_count().saturating_sub(1)).unwrap_or(u32::MAX);
    let mut levels: Vec<u8> = Vec::new();
    let mut out = Vec::with_capacity(headings.len());
    for (index, (level, line)) in headings.iter().enumerate() {
        while levels.last().is_some_and(|open| open >= level) {
            levels.pop();
        }
        let depth = levels.len();
        levels.push(*level);
        let end = headings
            .iter()
            .skip(index.saturating_add(1))
            .find(|(next, _)| next <= level)
            .and_then(|(_, next_line)| u32::try_from(*next_line).ok())
            .map_or(last_line, |next| next.saturating_sub(1));
        let text = doc.line_text(*line);
        let name = text.trim_start_matches('#').trim().to_string();
        let start = u32::try_from(*line).unwrap_or(u32::MAX);
        let range = Range {
            start: Position {
                line: start,
                character: 0,
            },
            end: Position {
                line: end.max(start),
                character: 0,
            },
        };
        out.push(Symbol {
            name,
            kind: format!("H{level}"),
            range,
            selection: Range {
                start: range.start,
                end: range.start,
            },
            depth,
            detail: None,
            deprecated: false,
        });
    }
    out
}

/// One workspace-wide symbol match from an LSP server.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WorkspaceSymbolHit {
    pub name: String,
    pub kind: String,
    pub container: Option<String>,
    pub uri: String,
    pub range: Range,
}
