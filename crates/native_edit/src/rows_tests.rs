//! Row layout and display-cell tests.
use super::*;
use crate::doc::Doc;
use crate::indent::IndentStyle;

/// The default tab width.
const TAB_WIDTH: usize = 4;

#[test]
fn hints_shift_the_chars_after_them_but_not_the_caret_before() {
    let hints = [RowHint {
        col: 1,
        cells: 5,
        label: ": i32",
    }];
    let row = RowCells {
        text: "x = 1",
        hints: &hints,
        tab: TAB_WIDTH,
    };
    assert_eq!(row.caret_x(1), 1);
    assert_eq!(row.char_x(1), 6);
    assert_eq!(row.char_x(0), 0);
    assert_eq!(row.caret_x(2), 7);
    // Clicks on the hint land before the char it annotates.
    assert_eq!(row.col_at(3), 1);
    assert_eq!(row.col_at(6), 1);
    assert_eq!(row.col_at(7), 2);
    assert_eq!(row.col_at(0), 0);
    assert_eq!(row.col_at(99), 5);
    // Tab stops ignore hint cells.
    let tabbed = RowCells {
        text: "a\tb",
        hints: &[RowHint {
            col: 1,
            cells: 2,
            label: "  ",
        }],
        tab: TAB_WIDTH,
    };
    assert_eq!(tabbed.char_x(2), TAB_WIDTH + 2);
}

#[test]
fn tabs_advance_to_the_next_tab_stop() {
    assert_eq!(display_x("\tfoo", 1, TAB_WIDTH), TAB_WIDTH);
    assert_eq!(display_x("ab\tc", 3, TAB_WIDTH), TAB_WIDTH);
    assert_eq!(display_x("abcd\tx", 5, TAB_WIDTH), TAB_WIDTH * 2);
    assert_eq!(col_at_display_x("\tfoo", 2, TAB_WIDTH), 0);
    assert_eq!(col_at_display_x("\tfoo", TAB_WIDTH, TAB_WIDTH), 1);
    assert_eq!(expand_tabs("a\tb", 0, TAB_WIDTH), "a   b");
    assert_eq!(expand_tabs("\tb", 2, TAB_WIDTH), "  b");
}

#[test]
fn tab_stops_follow_the_documents_tab_width() {
    assert_eq!(display_x("\t\tx", 2, 8), 16);
    assert_eq!(display_x("ab\tc", 3, 2), 4);
    assert_eq!(col_at_display_x("\tfoo", 7, 8), 0);
    assert_eq!(col_at_display_x("\tfoo", 8, 8), 1);
    assert_eq!(expand_tabs("a\tb", 0, 8), "a       b");
    // A zero width never stalls the stops.
    assert_eq!(display_x("\t\t", 2, 0), 2);
    let row = RowCells {
        text: "\tx",
        hints: &[],
        tab: 8,
    };
    assert_eq!((row.caret_x(1), row.col_at(9)), (8, 2));
}

#[test]
fn row_cache_reuses_rows_until_the_buffer_or_width_changes() {
    let mut doc = Doc::new("alpha\nbeta\n");
    let mut cache = RowCache::default();
    cache.ensure(&doc, None, None);
    let first = cache.rows().to_vec();
    assert_eq!(first.len(), doc.line_count());
    assert_eq!(cache.max_cells(), "alpha".chars().count());

    // Same revision and width: the rows are reused, not rebuilt.
    cache.ensure(&doc, None, None);
    assert_eq!(cache.rows(), first.as_slice());

    doc.insert(doc.char_at_line_col(1, 4), "!");
    cache.ensure(&doc, None, None);
    assert_ne!(cache.rows(), first.as_slice());
    assert_eq!(cache.max_cells(), "beta!".chars().count());

    // A narrower wrap width re-lays the rows out.
    cache.ensure(&doc, Some(2), None);
    assert!(cache.rows().len() > doc.line_count());
}

#[test]
fn row_cache_relays_rows_when_the_tab_width_changes() {
    let mut doc = Doc::new("\tx\n");
    let mut cache = RowCache::default();
    cache.ensure(&doc, None, None);
    assert_eq!(cache.max_cells(), TAB_WIDTH + 1);
    // A new indent style (an `.editorconfig` read) keeps the revision.
    doc.set_indent_style(IndentStyle::tabs(8));
    cache.ensure(&doc, None, None);
    assert_eq!(cache.max_cells(), 9);
    // Wrapping counts the wider tab too.
    let mut wide = Doc::new("\tab\n");
    let first_row = |doc: &Doc| visual_rows(doc, Some(6)).swap_remove(0).text;
    assert_eq!(first_row(&wide), "\tab");
    wide.set_indent_style(IndentStyle::tabs(8));
    assert_eq!(first_row(&wide), "\t");
}

#[test]
fn wrapped_rows_preserve_source_and_cursor_columns() {
    let doc = Doc::new("# Heading\n- Long words remain in one logical line\n");
    let rows = visual_rows(&doc, Some(10));
    for line in 0..doc.line_count() {
        let joined: String = rows
            .iter()
            .filter(|row| row.line == line)
            .map(|row| row.text.as_str())
            .collect();
        assert_eq!(joined, doc.line_text(line));
    }
    assert!(rows.iter().filter(|row| row.line == 1).count() > 1);
    for col in 0..=doc.line_text(1).chars().count() {
        assert_eq!(
            rows.iter().filter(|row| row.owns_cursor(1, col)).count(),
            1,
            "cursor column {col} must have one visual row"
        );
    }
    assert!(
        rows.iter()
            .all(|row| display_x(&row.text, row.text.chars().count(), TAB_WIDTH) <= 10)
    );
}
