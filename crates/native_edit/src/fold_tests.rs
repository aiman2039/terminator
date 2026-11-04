//! Fold region and hiding tests.
use super::*;
use crate::doc::Doc;

#[test]
fn bracket_regions_span_lines_and_fold_hides_their_body() {
    let doc = Doc::new("fn a() {\n    b(\n        1,\n    );\n}\nfn c() {}\n");
    let mut folds = Folds::default();
    folds.ensure(&doc, Language::Rust);
    assert_eq!(folds.region_at(0), Some((0, 4)));
    assert_eq!(folds.region_at(1), Some((1, 3)));
    assert_eq!(folds.region_at(5), None);
    folds.fold_at(2);
    assert!(folds.is_folded(1));
    assert!(folds.hidden(2) && folds.hidden(3) && !folds.hidden(1));
    folds.fold_at(2);
    assert!(folds.is_folded(0));
    folds.unfold_at(2);
    assert!(!folds.is_folded(0) && !folds.is_folded(1));
    folds.fold_all();
    assert!(folds.hidden(4));
    folds.unfold_all();
    assert!(!folds.hidden(4));
}

#[test]
fn fold_follows_lines_inserted_or_removed_above() {
    let mut doc = Doc::new("fn a() {\n    x();\n}\nfn b() {\n    y();\n}\n");
    let mut folds = Folds::default();
    folds.ensure(&doc, Language::Rust);
    folds.toggle(3);
    assert!(folds.is_folded(3) && folds.hidden(4));
    // Enter inside `fn a`, above the folded `fn b`.
    let at = doc.char_at_line_col(1, 8);
    doc.insert(at, "\n");
    folds.ensure(&doc, Language::Rust);
    assert!(folds.is_folded(4) && !folds.is_folded(3) && !folds.is_folded(0));
    assert!(folds.hidden(5) && !folds.hidden(1));
    // Removing the line again moves the fold back.
    let start = doc.char_at_line_col(1, 8);
    doc.delete(start..start + 1);
    folds.ensure(&doc, Language::Rust);
    assert!(folds.is_folded(3) && !folds.is_folded(0));
}

#[test]
fn enclosing_lists_open_blocks_outermost_first() {
    let doc = Doc::new("impl S {\n    fn f() {\n        x();\n    }\n}\n");
    let mut folds = Folds::default();
    folds.ensure(&doc, Language::Rust);
    assert_eq!(folds.enclosing(2), [(0, 4), (1, 3)]);
    // A block's own first line is not "enclosed" by it.
    assert_eq!(folds.enclosing(1), [(0, 4)]);
    assert!(folds.enclosing(0).is_empty());
}

#[test]
fn brackets_in_strings_and_comments_do_not_make_regions() {
    let doc = Doc::new("fn a() {\n    let s = \"{\";\n    // }\n    /* {\n    } */\n}\n");
    assert_eq!(fold_ranges(&doc, Language::Rust), [(0, 5)]);
}

#[test]
fn indentation_regions_for_python() {
    let doc = Doc::new("def f():\n    x = 1\n\n    return x\nprint(f())\n");
    assert_eq!(fold_ranges(&doc, Language::Python), [(0, 3)]);
}

#[test]
fn folds_survive_edits_that_keep_their_region() {
    let mut doc = Doc::new("if a {\n    b\n}\n");
    let mut folds = Folds::default();
    folds.ensure(&doc, Language::Rust);
    folds.toggle(0);
    doc.insert(doc.len_chars(), "x\n");
    folds.ensure(&doc, Language::Rust);
    assert!(folds.is_folded(0));
    // Deleting the block's opener drops the fold with its region.
    doc.delete(0..6);
    folds.ensure(&doc, Language::Rust);
    assert!(!folds.is_folded(0));
}

#[test]
fn fold_all_on_a_large_file_hides_bodies_in_one_sweep() {
    let blocks = 20_000;
    let text = "fn f() {\n    x();\n}\n".repeat(blocks);
    let doc = Doc::new(text);
    let mut folds = Folds::default();
    folds.ensure(&doc, Language::Rust);
    folds.fold_all();
    let started = std::time::Instant::now();
    let rows = crate::rows::visual_rows_folded(&doc, None, Some(&folds));
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    // Each block keeps its first line; the trailing empty line stays.
    assert_eq!(rows.len(), blocks + 1);
    assert_eq!(folds.hiding(1), Some(0));
    assert_eq!(folds.hiding(3), None);
    assert!(folds.hidden(2) && !folds.hidden(3));
}

#[test]
fn nested_folds_hide_up_to_the_outer_block_end() {
    let doc = Doc::new("a {\n  b {\n    c\n  }\n  d\n}\ne\n");
    let mut folds = Folds::default();
    folds.ensure(&doc, Language::Rust);
    folds.toggle(1);
    assert!(folds.hidden(2) && folds.hidden(3) && !folds.hidden(4));
    folds.toggle(0);
    assert_eq!(folds.hiding(2), Some(0));
    assert!(folds.hidden(5) && !folds.hidden(6));
}

#[test]
fn indentation_regions_close_in_one_pass_on_deep_files() {
    let mut text = String::new();
    for depth in 0..2_000 {
        text.push_str(&" ".repeat(depth));
        text.push_str("x:\n");
    }
    text.push_str("end\n");
    let doc = Doc::new(text);
    let started = std::time::Instant::now();
    let ranges = fold_ranges(&doc, Language::Yaml);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(ranges.first(), Some(&(0, 1_999)));
    assert_eq!(ranges.len(), 1_999);
    let nested = Doc::new("a:\n  b:\n    c\n\n  d\ne\n");
    assert_eq!(fold_ranges(&nested, Language::Yaml), [(0, 4), (1, 2)]);
}
