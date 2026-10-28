//! Outline from the syntax tree, for when no language server answers.
use super::grammar;
use crate::highlight::{Language, MAX_HIGHLIGHT_BYTES};

/// Declarations worth listing in an outline, per language: node kind to
/// symbol kind label (labels match the LSP outline's, so badges agree).
fn outline_kinds(language: Language) -> &'static [(&'static str, &'static str)] {
    match language {
        Language::Rust => &[
            ("function_item", "Function"),
            ("function_signature_item", "Function"),
            ("struct_item", "Struct"),
            ("enum_item", "Enum"),
            ("union_item", "Struct"),
            ("trait_item", "Interface"),
            ("impl_item", "Object"),
            ("mod_item", "Module"),
            ("const_item", "Constant"),
            ("static_item", "Constant"),
            ("type_item", "Type parameter"),
            ("macro_definition", "Function"),
        ],
        Language::Python => &[
            ("function_definition", "Function"),
            ("class_definition", "Class"),
        ],
        Language::JavaScript => &[
            ("function_declaration", "Function"),
            ("generator_function_declaration", "Function"),
            ("class_declaration", "Class"),
            ("method_definition", "Method"),
        ],
        _ => &[],
    }
}

/// Outline from the syntax tree, for when no language server answers:
/// declarations with their names and nesting. `None` for languages
/// without a bundled grammar.
#[must_use]
pub fn outline(language: Language, text: &str) -> Option<Vec<crate::lsp::Symbol>> {
    let kinds = outline_kinds(language);
    if kinds.is_empty() || text.len() > MAX_HIGHLIGHT_BYTES {
        return None;
    }
    let (_, grammar, _, _) = grammar(language)?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&grammar).ok()?;
    let tree = parser.parse(text, None)?;
    let walk = OutlineWalk {
        text,
        lines: text.lines().collect(),
        kinds,
    };
    Some(walk.symbols(tree.root_node()))
}

/// Depth-first walk over the tree, listing declarations in source order.
struct OutlineWalk<'t> {
    text: &'t str,
    lines: Vec<&'t str>,
    kinds: &'static [(&'static str, &'static str)],
}

impl OutlineWalk<'_> {
    fn symbols(&self, root: tree_sitter::Node<'_>) -> Vec<crate::lsp::Symbol> {
        let mut out = Vec::new();
        let mut stack: Vec<(tree_sitter::Node<'_>, usize)> = vec![(root, 0)];
        while let Some((node, depth)) = stack.pop() {
            let mut child_depth = depth;
            if let Some(symbol) = self.symbol(node, depth) {
                out.push(symbol);
                child_depth = depth.saturating_add(1);
            }
            // Children in reverse so the stack pops them in source order.
            let mut cursor = node.walk();
            let children: Vec<tree_sitter::Node<'_>> = node.children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                stack.push((child, child_depth));
            }
        }
        out
    }

    /// The symbol `node` declares, when it is a listed kind with a name.
    fn symbol(&self, node: tree_sitter::Node<'_>, depth: usize) -> Option<crate::lsp::Symbol> {
        let (_, label) = self.kinds.iter().find(|(kind, _)| *kind == node.kind())?;
        let name_node = node
            .child_by_field_name("name")
            .or_else(|| node.child_by_field_name("type"));
        let name = name_node
            .and_then(|name| self.text.get(name.byte_range()))
            .map(|name| name.split_whitespace().collect::<Vec<_>>().join(" "))?;
        let at = name_node.unwrap_or(node);
        Some(crate::lsp::Symbol {
            name: self.display_name(node, name),
            kind: (*label).to_owned(),
            range: crate::lsp::Range {
                start: self.position(node.start_position()),
                end: self.position(node.end_position()),
            },
            selection: crate::lsp::Range {
                start: self.position(at.start_position()),
                end: self.position(at.end_position()),
            },
            depth,
            detail: None,
            deprecated: false,
        })
    }

    /// `impl Trait for Type` / `impl Type` for impl blocks; the plain
    /// name otherwise.
    fn display_name(&self, node: tree_sitter::Node<'_>, name: String) -> String {
        if node.kind() != "impl_item" {
            return name;
        }
        match node
            .child_by_field_name("trait")
            .and_then(|item| self.text.get(item.byte_range()))
        {
            Some(trait_name) => format!("impl {trait_name} for {name}"),
            None => format!("impl {name}"),
        }
    }

    fn position(&self, point: tree_sitter::Point) -> crate::lsp::Position {
        let line = self.lines.get(point.row).copied().unwrap_or_default();
        let col = line
            .get(..point.column.min(line.len()))
            .map_or(0, |head| head.chars().count());
        crate::lsp::Position {
            line: u32::try_from(point.row).unwrap_or(u32::MAX),
            character: crate::lsp::char_col_to_utf16(line, col),
        }
    }
}
