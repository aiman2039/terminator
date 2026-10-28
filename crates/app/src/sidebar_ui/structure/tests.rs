use super::*;
use terminator_native_edit::lsp::{Position, Range};

fn symbol(name: &str, depth: usize, line: u32) -> Symbol {
    let at = Position { line, character: 0 };
    Symbol {
        name: name.into(),
        kind: "Function".into(),
        range: Range { start: at, end: at },
        selection: Range { start: at, end: at },
        depth,
        detail: None,
        deprecated: false,
    }
}

#[test]
fn outline_tree_links_children_to_their_nearest_shallower_symbol() {
    let symbols = [
        symbol("Point", 0, 0),
        symbol("x", 1, 1),
        symbol("y", 1, 2),
        symbol("new", 0, 4),
        symbol("inner", 1, 5),
    ];
    let tree = OutlineTree::build(&symbols);
    assert_eq!(tree.roots, [0, 3]);
    assert_eq!(tree.children.first(), Some(&vec![1, 2]));
    assert_eq!(tree.parent.get(4), Some(&Some(3)));
    assert_eq!(tree.keys.get(2).map(String::as_str), Some("Point\u{1f}y"));
}

#[test]
fn alphabetical_sort_orders_siblings_only() {
    let symbols = [
        symbol("zeta", 0, 0),
        symbol("b", 1, 1),
        symbol("a", 1, 2),
        symbol("alpha", 0, 3),
    ];
    let mut tree = OutlineTree::build(&symbols);
    tree.sort_alphabetically(&symbols);
    assert_eq!(tree.roots, [3, 0]);
    assert_eq!(tree.children.first(), Some(&vec![2, 1]));
}

#[test]
fn speed_search_is_a_case_insensitive_subsequence() {
    assert!(outline_matches("handle_request", "hreq"));
    assert!(outline_matches("HttpServer", "server"));
    assert!(!outline_matches("parse", "pz"));
}

#[test]
fn outline_falls_back_to_the_syntax_tree_without_a_server() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, terminator_core::Paths::at(dir.path().into()));
    let path = dir.path().join("lib.rs");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::loaded(path.clone(), "struct A;\nfn b() {}\n"),
    );
    let outline = app.document_outline(&path);
    assert_eq!(
        outline
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect::<Vec<_>>(),
        ["A", "b"]
    );
}
