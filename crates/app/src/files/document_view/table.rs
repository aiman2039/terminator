//! CSV/TSV parsing for the read-only table viewer. Runs on the GUI CPU pool.
use std::cmp::Ordering;

/// Rows kept after the header row; the view says when more were dropped.
pub const MAX_ROWS: usize = 100_000;
/// Columns kept per row; extra cells are dropped.
const MAX_COLUMNS: usize = 1024;
/// Records read to choose between `,` and `;`.
const SAMPLE_RECORDS: usize = 20;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub delimiter: u8,
    /// Rows past `MAX_ROWS`, or bytes past the read cap, were left out.
    pub truncated: bool,
    /// Per column: every non-empty body cell is a number (right-aligned).
    pub numeric: Vec<bool>,
}

impl Table {
    /// Whether `column` holds only numbers; the view right-aligns it.
    pub fn is_numeric(&self, column: usize) -> bool {
        self.numeric.get(column).copied().unwrap_or(false)
    }

    /// Columns where each non-empty body cell parses as a number and at
    /// least one cell is not empty.
    fn numeric_columns(&self) -> Vec<bool> {
        (0..self.columns())
            .map(|column| {
                let mut cells = self
                    .rows
                    .iter()
                    .filter_map(|row| row.get(column))
                    .map(|cell| cell.trim())
                    .filter(|cell| !cell.is_empty())
                    .peekable();
                cells.peek().is_some() && cells.all(|cell| cell.parse::<f64>().is_ok())
            })
            .collect()
    }

    pub fn columns(&self) -> usize {
        self.rows
            .iter()
            .map(Vec::len)
            .chain(std::iter::once(self.headers.len()))
            .max()
            .unwrap_or(0)
    }
}

/// The delimiter for `extension`: tab for TSV; for CSV, `;` when it splits
/// the sample into a steadier, wider shape than `,` does.
pub fn delimiter(extension: &str, sample: &[u8]) -> u8 {
    if extension.eq_ignore_ascii_case("tsv") || extension.eq_ignore_ascii_case("tab") {
        return b'\t';
    }
    if steady_width(sample, b';') > steady_width(sample, b',') {
        b';'
    } else {
        b','
    }
}

/// The field count every sampled record shares, or 0 when they differ or
/// have a single field.
fn steady_width(sample: &[u8], delimiter: u8) -> usize {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(sample);
    let mut widths: Vec<usize> = reader
        .byte_records()
        .take(SAMPLE_RECORDS)
        .map_while(Result::ok)
        .map(|record| record.len())
        .collect();
    // The sample may end inside the last record.
    if widths.len() > 1 {
        widths.pop();
    }
    match widths.first() {
        Some(&first) if first > 1 && widths.iter().all(|width| *width == first) => first,
        _ => 0,
    }
}

/// Parse `bytes`: the first record is the header row. Invalid UTF-8 is
/// replaced; a malformed record ends the table and marks it truncated.
pub fn parse(bytes: &[u8], delimiter: u8, truncated: bool) -> Table {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(bytes);
    let mut table = Table {
        delimiter,
        truncated,
        ..Table::default()
    };
    let mut records = reader.byte_records();
    let cells = |record: &csv::ByteRecord| -> Vec<String> {
        record
            .iter()
            .take(MAX_COLUMNS)
            .map(|cell| String::from_utf8_lossy(cell).into_owned())
            .collect()
    };
    match records.next() {
        Some(Ok(header)) => table.headers = cells(&header),
        Some(Err(_)) => table.truncated = true,
        None => {}
    }
    for record in records {
        if table.rows.len() >= MAX_ROWS {
            table.truncated = true;
            break;
        }
        let Ok(record) = record else {
            table.truncated = true;
            break;
        };
        table.rows.push(cells(&record));
    }
    table.numeric = table.numeric_columns();
    table
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub column: usize,
    pub ascending: bool,
}

/// Row indices that contain `filter` (ASCII case-insensitive) in any cell,
/// in `sort` order. Numbers compare as numbers; the sort is stable.
pub fn order(table: &Table, sort: Option<Sort>, filter: &str) -> Vec<usize> {
    let needle = filter.trim().as_bytes();
    let rows: Vec<usize> = (0..table.rows.len())
        .filter(|&index| {
            needle.is_empty()
                || table.rows.get(index).is_some_and(|row| {
                    row.iter()
                        .any(|cell| contains_ignore_case(cell.as_bytes(), needle))
                })
        })
        .collect();
    let Some(Sort { column, ascending }) = sort else {
        return rows;
    };
    // Parse each cell once, not once per comparison.
    let mut keyed: Vec<(usize, Key<'_>)> = rows
        .into_iter()
        .map(|index| {
            let cell = table
                .rows
                .get(index)
                .and_then(|row| row.get(column))
                .map_or("", String::as_str);
            (index, Key::of(cell))
        })
        .collect();
    keyed.sort_by(|(_, left), (_, right)| {
        let ordering = left.cmp(right);
        if ascending {
            ordering
        } else {
            ordering.reverse()
        }
    });
    keyed.into_iter().map(|(index, _)| index).collect()
}

/// Sort key of one cell: numbers first (by value), then text (case-folded),
/// then empty cells.
enum Key<'a> {
    Number(f64),
    Text(&'a str),
    Empty,
}

impl<'a> Key<'a> {
    fn of(cell: &'a str) -> Self {
        let trimmed = cell.trim();
        if trimmed.is_empty() {
            Self::Empty
        } else if let Ok(number) = trimmed.parse::<f64>() {
            Self::Number(number)
        } else {
            Self::Text(cell)
        }
    }

    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(left), Self::Number(right)) => left.total_cmp(right),
            (Self::Text(left), Self::Text(right)) => left
                .chars()
                .flat_map(char::to_lowercase)
                .cmp(right.chars().flat_map(char::to_lowercase))
                .then_with(|| left.cmp(right)),
            (Self::Empty, Self::Empty) => Ordering::Equal,
            (Self::Number(_), _) | (Self::Text(_), Self::Empty) => Ordering::Less,
            (_, Self::Number(_)) | (Self::Empty, Self::Text(_)) => Ordering::Greater,
        }
    }
}

fn contains_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}
