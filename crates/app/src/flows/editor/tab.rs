//! The identity of a flow editor tab: the flow's name as its title, the
//! flow glyph, the file path on hover, and the unsaved dot.
use super::Slot;
use crate::*;
use std::path::Path;

/// The glyph of a flow editor tab: the Flows glyph.
pub(crate) const FLOW_TAB_ICON: &str = "Workflow";

impl App {
    /// The title of the flow editor tab of `path`: the flow's name, or
    /// the file name without `.toml` before the flow is read.
    pub(crate) fn flow_tab_title(&self, path: &Path) -> String {
        match self.flows.editors.get(path) {
            Some(Slot::Ready(editor)) if !editor.title.trim().is_empty() => {
                editor.title.trim().to_owned()
            }
            _ => path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        }
    }

    /// The flow of `path` has content that is not saved: its tab shows the
    /// unsaved dot. A moved card alone is not a change here.
    pub(crate) fn flow_tab_dirty(&self, path: &Path) -> bool {
        matches!(self.flows.editors.get(path), Some(Slot::Ready(editor)) if editor.changed())
    }
}
