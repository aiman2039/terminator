use std::path::PathBuf;

use crate::{sidebar_ui, workspace_ops};

/// Sidebar Git panel state: the commit draft and its options, branch/log/
/// shelf lists for the current Git root, the branch comparison, and the
/// destructive action awaiting confirmation.
#[derive(Default)]
pub(crate) struct GitPanelState {
    pub(crate) commit: String,
    /// Message of the commit in flight; restored into the draft if the
    /// commit itself fails so a rejected hook does not lose it.
    pub(crate) commit_inflight: Option<String>,
    pub(crate) amend: bool,
    pub(crate) signoff: bool,
    pub(crate) history: bool,
    pub(crate) branches: Vec<String>,
    pub(crate) log: Vec<(String, String)>,
    pub(crate) shelves: Vec<(String, String)>,
    pub(crate) list_root: Option<PathBuf>,
    pub(crate) collapse: u64,
    /// Branch comparison for the current Git root (upstream, ahead/behind,
    /// and files committed since the base ref).
    pub(crate) compare: Option<workspace_ops::CompareData>,
    pub(crate) compare_root: Option<PathBuf>,
    /// User-picked base ref; falls back to the upstream branch when `None`.
    pub(crate) base_ref: Option<String>,
    /// Destructive Git action awaiting confirmation on the inline bar.
    pub(crate) confirm: Option<sidebar_ui::GitConfirm>,
}
