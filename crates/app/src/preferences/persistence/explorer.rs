//! Saved Explorer view and search options.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Explorer search behavior: filter the visible tree by name, or search file
/// contents and show a result list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplorerSearchMode {
    #[default]
    Names,
    Contents,
}

/// Explorer tree view options (`IntelliJ` Project view "Tree Appearance").
/// The defaults keep the original order: folders first, natural name sort.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExplorerView {
    /// Sort folders among files instead of above them.
    pub folders_mixed: bool,
    /// Sort by extension, then by name.
    pub sort_by_type: bool,
    /// Show every file at its own level instead of nesting lock and
    /// generated files under their source (`Cargo.lock` under `Cargo.toml`).
    pub nesting_off: bool,
    /// Show only files with Git changes and the folders that hold them.
    pub changed_only: bool,
    /// Folders and files hidden like ignored files (`IntelliJ` "Mark
    /// Directory as Excluded", but for this view only).
    pub excluded: Vec<PathBuf>,
}
