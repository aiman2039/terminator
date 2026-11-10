use super::table::{self, MAX_ROWS, Sort};
use super::{DocumentKind, TableState};
use std::path::Path;

#[test]
fn tsv_uses_tabs_and_csv_picks_semicolons_only_when_they_fit_better() {
    assert_eq!(table::delimiter("tsv", b"a,b\tc\n"), b'\t');
    assert_eq!(table::delimiter("TSV", b""), b'\t');
    assert_eq!(table::delimiter("csv", b"a,b,c\n1,2,3\n4,5,6\n"), b',');
    assert_eq!(table::delimiter("csv", b"a;b;c\n1,5;2;3\n4;5,5;6\n"), b';');
    // Commas inside quotes do not make a semicolon file comma-separated.
    assert_eq!(
        table::delimiter("csv", b"name;note\nx;\"a, b, c\"\ny;\"d, e\"\n"),
        b';'
    );
    // A single column stays on the default.
    assert_eq!(table::delimiter("csv", b"one\ntwo\n"), b',');
}

#[test]
fn first_record_is_the_header_and_quoted_cells_keep_delimiters() {
    let parsed = table::parse(
        b"name,city\n\"Doe, J\",Paris\nAmy,\"New\nYork\"\n",
        b',',
        false,
    );
    assert_eq!(parsed.headers, ["name", "city"]);
    assert_eq!(parsed.rows.len(), 2);
    assert_eq!(parsed.rows[0], ["Doe, J", "Paris"]);
    assert_eq!(parsed.rows[1][1], "New\nYork");
    assert!(!parsed.truncated);
    assert_eq!(parsed.columns(), 2);
}

#[test]
fn columns_with_only_numbers_are_right_aligned() {
    let parsed = table::parse(
        b"name,qty,price,note,blank\nA,1,2.5,x,\nB,,-3e2,7,\nC,10,4,y,\n",
        b',',
        false,
    );
    let numeric: Vec<bool> = (0..6).map(|column| parsed.is_numeric(column)).collect();
    // Empty cells do not count; one text cell or no cell at all does.
    assert_eq!(numeric, [false, true, true, false, false, false]);
}

#[test]
fn rows_past_the_cap_are_left_out_and_marked() {
    let mut text = String::from("n\n");
    for index in 0..=MAX_ROWS {
        text.push_str(&format!("{index}\n"));
    }
    let parsed = table::parse(text.as_bytes(), b',', false);
    assert_eq!(parsed.rows.len(), MAX_ROWS);
    assert!(parsed.truncated);
    let small = table::parse(b"n\n1\n2\n", b',', true);
    assert!(small.truncated, "a byte-capped read stays marked");
}

#[test]
fn sort_orders_numbers_by_value_then_text_then_empty_cells() {
    let parsed = table::parse(b"v\n10\nbeta\n\"\"\n9\nAlpha\n2.5\n", b',', false);
    let cell = |rows: Vec<usize>| -> Vec<String> {
        rows.into_iter()
            .map(|index| parsed.rows[index].first().cloned().unwrap_or_default())
            .collect()
    };
    let ascending = Some(Sort {
        column: 0,
        ascending: true,
    });
    assert_eq!(
        cell(table::order(&parsed, ascending, "")),
        ["2.5", "9", "10", "Alpha", "beta", ""]
    );
    let descending = Some(Sort {
        column: 0,
        ascending: false,
    });
    assert_eq!(
        cell(table::order(&parsed, descending, "")),
        ["", "beta", "Alpha", "10", "9", "2.5"]
    );
    assert_eq!(
        cell(table::order(&parsed, None, "")),
        ["10", "beta", "", "9", "Alpha", "2.5"]
    );
}

#[test]
fn filter_keeps_rows_with_a_case_insensitive_match_in_any_cell() {
    let parsed = table::parse(b"a,b\nRed,1\nblue,2\nGreen,red\n", b',', false);
    assert_eq!(table::order(&parsed, None, "RED"), [0, 2]);
    assert_eq!(table::order(&parsed, None, "  "), [0, 1, 2]);
    assert!(table::order(&parsed, None, "violet").is_empty());
}

#[test]
fn header_clicks_cycle_ascending_descending_and_unsorted() {
    let mut state = TableState::default();
    state.toggle_sort(1);
    assert_eq!(
        state.sort,
        Some(Sort {
            column: 1,
            ascending: true
        })
    );
    state.toggle_sort(1);
    assert_eq!(
        state.sort,
        Some(Sort {
            column: 1,
            ascending: false
        })
    );
    state.toggle_sort(1);
    assert_eq!(state.sort, None);
    state.toggle_sort(0);
    state.toggle_sort(2);
    assert_eq!(state.sort.map(|sort| sort.column), Some(2));
}

#[test]
fn only_csv_and_tsv_files_open_in_the_table_viewer() {
    for name in ["a.csv", "a.CSV", "a.tsv", "a.tab"] {
        assert_eq!(
            DocumentKind::for_path(Path::new(name)),
            Some(DocumentKind::Table)
        );
    }
    for name in ["a.txt", "csv", "a.md", "a.mmd"] {
        assert_eq!(DocumentKind::for_path(Path::new(name)), None);
    }
}

#[test]
fn read_drops_a_partial_last_line_only_when_capped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.csv");
    std::fs::write(&path, "a,b\n1,2").unwrap();
    let cancel = terminator_core::async_service::CancellationToken::new();
    let (bytes, truncated) = super::read(&path, &cancel).unwrap();
    assert_eq!(bytes, b"a,b\n1,2");
    assert!(!truncated);
    assert!(super::read(dir.path(), &cancel).is_err());
}
