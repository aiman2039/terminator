use std::time::Instant;

use crate::palette;

/// Command palette state: visibility, query, kind filters, selection, and
/// the cached result list for the current query.
pub(crate) struct PaletteState {
    pub(crate) open: bool,
    pub(crate) query: String,
    /// Bit set of palette chips. `0` means every kind in the current project.
    pub(crate) filters: u8,
    pub(crate) index: usize,
    pub(crate) cache: Option<palette::PaletteCache>,
    pub(crate) query_changed_at: Instant,
    /// File Structure mode: list only this file's outline, in order.
    pub(crate) file_scope: Option<std::path::PathBuf>,
}

impl PaletteState {
    pub(crate) fn new() -> Self {
        Self {
            open: false,
            query: String::new(),
            filters: 0,
            index: 0,
            cache: None,
            query_changed_at: Instant::now(),
            file_scope: None,
        }
    }
}
