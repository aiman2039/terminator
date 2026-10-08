mod persistence;
mod sorting;
mod strip;

pub use persistence::{AgentsTab, ExplorerSearchMode, RadioStation, SidebarTool, UiPreferences};
// `Playlist` is only named by `lib.rs` tests; keep its old path for them
// without tripping `unused_imports` on normal builds.
#[cfg(test)]
pub use persistence::Playlist;
pub use sorting::{
    HistoryGroup, HistoryInput, HistorySort, ProjectSort, VisibleProjects, sort_history,
    sort_visible_projects,
};
