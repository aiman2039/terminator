use super::*;

pub fn file_uri(path: &Path) -> Option<Url> {
    Url::from_file_path(path).ok()
}

/// Local path for a `file://` URI string. `None` for non-file URIs or
/// unparsable input (the caller keeps the URI for display).
#[must_use]
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    Url::parse(uri).ok()?.to_file_path().ok()
}

/// Buffer char column to LSP UTF-16 units for one line. Astral chars
/// count 2, everything else 1; clamps at the line end.
#[must_use]
pub fn char_col_to_utf16(line: &str, char_col: usize) -> u32 {
    let mut units = 0usize;
    let mut seen = 0usize;
    for c in line.chars() {
        if seen >= char_col {
            break;
        }
        units = units.saturating_add(c.len_utf16());
        seen = seen.saturating_add(1);
    }
    u32::try_from(units).unwrap_or(u32::MAX)
}

/// LSP UTF-16 units back to a buffer char column for one line. Splits
/// inside a surrogate pair snap to the pair start; clamps at line end.
#[must_use]
pub fn utf16_to_char_col(line: &str, units: u32) -> usize {
    let mut used = 0u32;
    let mut col = 0usize;
    for c in line.chars() {
        let width = u32::try_from(c.len_utf16()).unwrap_or(u32::MAX);
        if used.saturating_add(width) > units {
            break;
        }
        used = used.saturating_add(width);
        col = col.saturating_add(1);
    }
    col
}
