//! `gf`: resolve the file name under the cursor to an existing file;
//! Ctrl-^: track the alternate (previously active) file.
use super::NavLocation;
use crate::App;
use crate::daemon_link::services::resolve_location;
use std::path::{Path, PathBuf};

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

impl App {
    /// Follow the active editor: when another file becomes active, the
    /// one left behind (with its last caret) is the alternate file.
    /// Runs every frame, so it only borrows until the active file changes;
    /// the caret is read from the left-behind buffer at Ctrl-^ time.
    pub(crate) fn track_alternate_file(&mut self) {
        let Some(path) = self.active_editor_path_ref() else {
            return;
        };
        if self.editor_current.as_deref() == Some(path) {
            return;
        }
        let path = path.to_owned();
        if let Some(left) = self.editor_current.replace(path) {
            self.editor_alternate = Some(left);
        }
    }

    /// The alternate file and its last caret (start of file when that
    /// buffer is gone).
    pub(crate) fn alternate_location(&self) -> Option<NavLocation> {
        let path = self.editor_alternate.clone()?;
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
}
