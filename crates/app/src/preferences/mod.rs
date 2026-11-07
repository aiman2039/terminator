mod editor_state;
mod persistence;
mod sorting;
mod strip;
mod type_ahead;

pub use editor_state::{EditorState, EditorStates};
pub use persistence::{
    AgentsTab, ExplorerSearchMode, ExplorerView, LeftContent, ProjectStyle, RadioStation,
    SidebarTool, UiPreferences, VncPasteShortcut, clamp_left_width,
};
pub use type_ahead::TypeAhead;
// `Playlist` is only named by `lib.rs` tests; keep its old path for them
// without tripping `unused_imports` on normal builds.
#[cfg(test)]
pub use persistence::Playlist;
pub(crate) use sorting::{
    HistoryGroup, HistoryInput, HistorySort, ProjectSort, VisibleProjects, sort_history,
    sort_visible_projects,
};
