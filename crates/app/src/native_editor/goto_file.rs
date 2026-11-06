//! `gf`: resolve the file name under the cursor to an existing file;
//! Ctrl-^: track the alternate (previously active) file.
use super::NavLocation;
use crate::App;
use crate::daemon_link::services::resolve_location;
use std::path::{Path, PathBuf};

/// Vim file and window navigation state of the native editors. The
/// requests wait here while the workspace is checked out for rendering.
#[derive(Default)]
pub(crate) struct NavPending {
    /// The active editor file, and the one active before it
    /// (Ctrl-^ / Ctrl-6 alternate file).
    pub(crate) current: Option<PathBuf>,
    pub(crate) alternate: Option<PathBuf>,
    /// `:e newfile` targets: their first load is an empty new buffer.
    pub(crate) new_files: std::collections::HashSet<PathBuf>,
    /// Ctrl-O / Ctrl-I past a buffer's jumps: Back (false) or Forward.
    pub(crate) history: Option<bool>,
    /// Ctrl-W h/j/k/l/w.
    pub(crate) pane_move: Option<terminator_native_edit::vim::PaneMove>,
    /// `:bn` (true) / `:bp` (false): cycle the focused pane's tabs.
    pub(crate) cycle_tab: Option<bool>,
    /// `:sp file` / `:vs file` / `:tabe file` lookups that landed: the
    /// issuing file, the target, its 1-based line and where it opens.
    pub(crate) split_opens: Vec<SplitOpen>,
}

pub(crate) type SplitOpen = (
    PathBuf,
    PathBuf,
    u32,
    terminator_native_edit::vim::SplitPlace,
);

/// Resolve a `gf` file name with the terminal link resolver, next to the
/// current file and then in the project root. A trailing `:line[:col]`
/// (1-based) becomes a zero-based LSP position. `None` when no candidate
/// is an existing file.
pub(crate) fn resolve_goto_file(
    name: &str,
    current: &Path,
    root: Option<&Path>,
) -> Option<(PathBuf, u32, u32)> {
    let zero_based = |n: Option<u32>| n.unwrap_or(0).saturating_sub(1);
    current
        .parent()
        .into_iter()
        .chain(root)
        .filter_map(|dir| resolve_location(name, dir).ok())
        .find(|(path, ..)| path.is_file())
        .map(|(path, line, column)| (path, zero_based(line), zero_based(column)))
}

/// `:e name`: [`resolve_goto_file`], else a new file at `name` (taken
/// from the project root, like vim's working directory) when its folder
/// exists. The flag is true for the new file.
pub(crate) fn resolve_edit_file(
    name: &str,
    current: &Path,
    root: Option<&Path>,
) -> Option<(PathBuf, u32, u32, bool)> {
    if let Some((path, line, column)) = resolve_goto_file(name, current, root) {
        return Some((path, line, column, false));
    }
    let base = root.or_else(|| current.parent())?;
    let path = base.join(name);
    let folder_exists = path.parent().is_some_and(Path::is_dir);
    (folder_exists && !path.exists()).then_some((path, 0, 0, true))
}

impl App {
    /// Follow the active editor: when another file becomes active, the
    /// one left behind (with its last caret) is the alternate file.
    /// Runs every frame, so it only borrows until the active file changes;
    /// the caret is read from the left-behind buffer at Ctrl-^ time.
    pub(crate) fn track_alternate_file(&mut self) {
        let Some(path) = self.active_editor_path_ref() else {
            return;
        };
        if self.native_nav.current.as_deref() == Some(path) {
            return;
        }
        let path = path.to_owned();
        if let Some(left) = self.native_nav.current.replace(path) {
            self.native_nav.alternate = Some(left);
        }
    }

    /// Show a vim message in the editor of `path` until its next edit.
    pub(crate) fn native_notice(&mut self, path: &Path, message: impl Into<String>) {
        if let Some(doc) = self.native_docs.get_mut(path) {
            doc.set_notice(message);
        }
    }

    /// The alternate file and its last caret (start of file when that
    /// buffer is gone).
    pub(crate) fn alternate_location(&self) -> Option<NavLocation> {
        let path = self.native_nav.alternate.clone()?;
        Some(self.location_of(&path).unwrap_or(NavLocation {
            path,
            line: 0,
            character: 0,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goto_file_resolves_next_to_the_file_then_the_project_root() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let src = base.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("near.rs"), "").unwrap();
        std::fs::write(base.join("README.md"), "").unwrap();
        let current = src.join("main.rs");
        let root = Some(base.as_path());
        assert_eq!(
            resolve_goto_file("near.rs", &current, root),
            Some((src.join("near.rs"), 0, 0))
        );
        assert_eq!(
            resolve_goto_file("README.md:12:3", &current, root),
            Some((base.join("README.md"), 11, 2))
        );
        assert_eq!(
            resolve_goto_file("src/near.rs:7", &current, root),
            Some((src.join("near.rs"), 6, 0))
        );
        assert_eq!(resolve_goto_file("missing.rs", &current, root), None);
    }

    #[test]
    fn edit_opens_an_existing_file_or_a_new_one_in_an_existing_folder() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::write(base.join("old.rs"), "").unwrap();
        let current = base.join("main.rs");
        let root = Some(base.as_path());
        assert_eq!(
            resolve_edit_file("old.rs", &current, root),
            Some((base.join("old.rs"), 0, 0, false))
        );
        assert_eq!(
            resolve_edit_file("new.rs", &current, root),
            Some((base.join("new.rs"), 0, 0, true))
        );
        assert_eq!(resolve_edit_file("no/such/dir.rs", &current, root), None);
    }
}
