//! What the status bar shows for the focused native editor: its path as
//! breadcrumbs, the caret's symbol, and whether it is saving or renaming.
//! Unsaved changes show on the file's tab instead.
use super::*;

/// One path segment and the folder or file it reveals in Explorer.
pub(crate) struct Crumb {
    pub(crate) label: String,
    /// `None` when Explorer cannot show it: a remote file, a file outside
    /// the project, or a focused shell instead of the editor.
    pub(crate) reveal: Option<PathBuf>,
}

/// The focused native editor's place and state.
pub(crate) struct EditorCrumbs {
    pub(crate) segments: Vec<Crumb>,
    /// Innermost outline symbol around the caret.
    pub(crate) symbol: Option<String>,
    /// "Saving…" or "Renaming…" while the file is busy.
    pub(crate) busy: Option<&'static str>,
}

impl App {
    /// Breadcrumbs for the native editor showing `path` in `project`.
    pub(crate) fn editor_crumbs(&mut self, project: &Project, path: &Path) -> EditorCrumbs {
        let outline = self.document_outline(path);
        let doc = self.native_docs.get(path);
        let symbol = doc
            .and_then(NativeDoc::caret_line)
            .and_then(|line| u32::try_from(line).ok())
            .and_then(|line| native_lsp::innermost_symbol(&outline, line))
            .and_then(|index| outline.get(index))
            .map(|symbol| symbol.name.clone());
        let busy = if doc.is_some_and(|doc| doc.saving) {
            Some("Saving…")
        } else if self
            .native_rename
            .as_ref()
            .is_some_and(|dialog| dialog.applying_to(path))
        {
            Some("Renaming…")
        } else {
            None
        };
        EditorCrumbs {
            segments: self.crumb_segments(project, path),
            symbol,
            busy,
        }
    }

    /// One segment per folder under the project root and the file. A
    /// remote file or a file outside the project is one plain segment.
    fn crumb_segments(&self, project: &Project, path: &Path) -> Vec<Crumb> {
        if let Some(label) = self.remote_file_label(path) {
            return vec![Crumb {
                label,
                reveal: None,
            }];
        }
        let Ok(relative) = path.strip_prefix(&project.path) else {
            return vec![Crumb {
                label: path.display().to_string(),
                reveal: None,
            }];
        };
        // Explorer reveals the focused file's folders when Locate can.
        let revealable = self
            .explorer_reveal_candidate()
            .is_some_and(|(file, _)| file == path);
        let mut at = project.path.clone();
        relative
            .components()
            .map(|part| {
                at.push(part);
                Crumb {
                    label: part.as_os_str().to_string_lossy().into_owned(),
                    reveal: revealable.then(|| at.clone()),
                }
            })
            .collect()
    }
}
