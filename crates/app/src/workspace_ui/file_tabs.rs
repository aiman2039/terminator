//! File tabs like the `JetBrains` editor tabs: the file-type icon, a dot
//! for unsaved changes that turns into the close button on hover, and the
//! parent folder after a file name that another open tab also shows.
use super::super::*;
use crate::workspace::WorkspaceTab;
use std::ffi::OsStr;
use std::path::Component;

/// Tab title font size: the theme's button text, which dock tabs use.
const TITLE_SIZE: f32 = crate::appearance::FONT_BODY;
/// The folder hint is smaller than the name, so it stays secondary even
/// beside a muted (inactive) name.
const HINT_SIZE: f32 = crate::appearance::FONT_SMALL;

impl App {
    /// The file a tab is named after: a native editor or image preview, or
    /// a Neovim editor whose label is still its file name.
    pub(crate) fn tab_file<'a>(&'a self, tab: &'a Tab) -> Option<&'a Path> {
        match tab {
            Tab::NativeEditor { path } | Tab::Image { path } | Tab::Document { path, .. } => {
                Some(path)
            }
            Tab::Terminal(sid) => self.named_editor_file(sid),
            _ => None,
        }
    }

    /// A Neovim editor session's file, while its label is the file name
    /// (a renamed tab no longer shows the file name).
    pub(crate) fn named_editor_file(&self, sid: &str) -> Option<&Path> {
        let session = self.state.sessions.iter().find(|s| s.id == sid)?;
        let file = session.file.as_deref()?;
        (session.kind == SessionKind::Editor
            && !session.review
            && file.file_name() == Some(OsStr::new(&session.label)))
        .then_some(file)
    }

    /// Kind glyph of a session's tab: the file type of an editor's file,
    /// else the generic editor or terminal glyph.
    pub(crate) fn session_kind_icon(session: &Session) -> &'static str {
        match session.file.as_deref() {
            Some(file) if session.kind == SessionKind::Editor && !session.review => {
                icons::file_icon(file)
            }
            _ if session.kind == SessionKind::Editor => "FileCode",
            _ => "Terminal",
        }
    }

    /// A native editor anywhere in this top-level tab has unsaved changes.
    pub(crate) fn group_unsaved(&self, group: &WorkspaceTab) -> bool {
        group
            .layout
            .iter_all_tabs()
            .any(|(_, tab)| matches!(tab, Tab::NativeEditor { path } if self.native_dirty(path)))
    }

    /// [`folder_hint`] for a tab's file among the `open` tab files. The
    /// local copy of a remote (SSH) file shows only its remote folders.
    pub(crate) fn tab_folder_hint<'a>(
        &self,
        path: &Path,
        open: impl IntoIterator<Item = &'a Path>,
    ) -> Option<String> {
        let open = open.into_iter().map(|other| self.remote_relative(other));
        folder_hint(self.remote_relative(path), open)
    }

    /// The remote path of the local copy of a remote file, which lives at
    /// `<data>/remote-files/<project>/<remote path>`; else `path` itself.
    fn remote_relative<'p>(&self, path: &'p Path) -> &'p Path {
        if !crate::remote_ui::is_snapshot(&self.paths.data, path) {
            return path;
        }
        let Ok(rest) = path.strip_prefix(&self.paths.data) else {
            return path;
        };
        let mut parts = rest.components();
        // Skip the cache folder and the project folder.
        if parts.nth(1).is_none() {
            return path;
        }
        parts.as_path()
    }
}

/// The nearest parent folders that tell `path` apart from every other open
/// path with the same file name, joined by `/`: usually just the parent
/// folder. `None` when no other open tab shows that name. A second view of
/// the same file does not count, and a file at the root has no folder.
fn folder_hint<'a>(path: &Path, open: impl IntoIterator<Item = &'a Path>) -> Option<String> {
    let name = path.file_name()?;
    let mut mine: Option<Vec<&OsStr>> = None;
    let mut depth = 0_usize;
    for other in open {
        if other == path || other.file_name() != Some(name) {
            continue;
        }
        let mine = mine.get_or_insert_with(|| parent_names(path).collect());
        let shared = mine
            .iter()
            .zip(parent_names(other))
            .take_while(|(left, right)| **left == *right)
            .count();
        depth = depth.max(shared.saturating_add(1));
    }
    let folders: Vec<_> = mine?
        .into_iter()
        .take(depth)
        .rev()
        .map(OsStr::to_string_lossy)
        .collect();
    (!folders.is_empty()).then(|| folders.join("/"))
}

/// Folder names above `path`, nearest first.
fn parent_names(path: &Path) -> impl Iterator<Item = &OsStr> {
    path.parent()
        .into_iter()
        .flat_map(|parent| parent.components().rev())
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name),
            _ => None,
        })
}

/// A tab title: the name, then the folder hint in smaller `weak` text.
/// `Color32::PLACEHOLDER` as `name_color` lets the dock pick its tab text
/// color.
pub(crate) fn hinted_title(
    name: &str,
    hint: &str,
    name_color: Color32,
    weak: Color32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        name,
        0.0,
        egui::TextFormat::simple(egui::FontId::proportional(TITLE_SIZE), name_color),
    );
    job.append(
        &format!(" {hint}"),
        3.0,
        egui::TextFormat::simple(egui::FontId::proportional(HINT_SIZE), weak),
    );
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hint(path: &str, open: &[&str]) -> Option<String> {
        folder_hint(Path::new(path), open.iter().map(Path::new))
    }

    #[test]
    fn a_unique_file_name_has_no_folder_hint() {
        assert_eq!(hint("/p/a/mod.rs", &["/p/a/mod.rs", "/p/b/lib.rs"]), None);
        assert_eq!(hint("/p/a/mod.rs", &[]), None);
    }

    #[test]
    fn same_name_tabs_show_their_parent_folder() {
        let open = ["/p/a/mod.rs", "/p/b/mod.rs"];
        assert_eq!(hint("/p/a/mod.rs", &open).as_deref(), Some("a"));
        assert_eq!(hint("/p/b/mod.rs", &open).as_deref(), Some("b"));
    }

    #[test]
    fn a_shared_parent_name_adds_folders_until_the_paths_differ() {
        let open = ["/w/crates/app/src/lib.rs", "/w/crates/core/src/lib.rs"];
        assert_eq!(hint(open[0], &open).as_deref(), Some("app/src"));
        assert_eq!(hint(open[1], &open).as_deref(), Some("core/src"));
        // Each path only goes as deep as it must: `c` is unique at once.
        let three = ["/p/a/src/lib.rs", "/p/b/src/lib.rs", "/p/c/lib.rs"];
        assert_eq!(hint(three[0], &three).as_deref(), Some("a/src"));
        assert_eq!(hint(three[2], &three).as_deref(), Some("c"));
        // A path inside the other's folders shows its whole parent path.
        let nested = ["/x/src/lib.rs", "/y/x/src/lib.rs"];
        assert_eq!(hint(nested[0], &nested).as_deref(), Some("x/src"));
        assert_eq!(hint(nested[1], &nested).as_deref(), Some("y/x/src"));
    }

    #[test]
    fn two_views_of_one_file_need_no_hint() {
        let open = ["/p/a/mod.rs", "/p/a/mod.rs"];
        assert_eq!(hint("/p/a/mod.rs", &open), None);
    }

    #[test]
    fn a_root_file_has_no_folder_to_show() {
        let open = ["/lib.rs", "/p/lib.rs"];
        assert_eq!(hint("/lib.rs", &open), None);
        assert_eq!(hint("/p/lib.rs", &open).as_deref(), Some("p"));
    }

    #[test]
    fn hinted_titles_keep_the_name_then_a_smaller_weak_folder() {
        let weak = Color32::from_gray(120);
        let job = hinted_title("mod.rs", "a", Color32::PLACEHOLDER, weak);
        assert_eq!(job.text, "mod.rs a");
        let [name, folder] = job.sections.as_slice() else {
            panic!("two sections: {:?}", job.sections);
        };
        assert_eq!(name.format.color, Color32::PLACEHOLDER);
        assert_eq!(folder.format.color, weak);
        assert!(folder.format.font_id.size < name.format.font_id.size);
    }
}
