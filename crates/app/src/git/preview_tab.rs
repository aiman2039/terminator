//! The Git panel's preview diff tab. A click on a changed file shows its
//! diff in one reused tab (italic title), the way an IDE preview tab does.
//! A double click keeps the tab; a pinned tab or a diff with unsaved edits
//! is never reused. The flag lives in GUI memory only: the layout JSON
//! does not change.
use crate::*;

/// The project and diff tab that the next Git panel click may replace.
#[derive(Default)]
pub(crate) struct Preview {
    tab: Option<(String, Tab)>,
    /// Set while a Git panel click opens its diff.
    opening: bool,
}

impl App {
    /// Opens the diff of a Git panel row click in the preview tab. A repeat
    /// within the double-click interval keeps the preview tab instead.
    pub(crate) fn open_git_preview(
        &mut self,
        ui: &egui::Ui,
        path: &std::path::Path,
        action: FileAction,
    ) {
        self.git_extras.preview.opening = true;
        let before = self.file_activation.as_ref().map(|last| last.at);
        self.activate_file_action(ui, path, action);
        self.git_extras.preview.opening = false;
        let repeated = before.is_some() && self.file_activation.as_ref().map(|l| l.at) == before;
        if repeated {
            self.keep_preview_diff_of(path);
        }
    }

    /// The preview tab becomes an ordinary tab: the next click opens a new one.
    pub(crate) fn keep_preview_diff(&mut self) {
        self.git_extras.preview.tab = None;
    }

    /// Keeps the preview tab when it shows `path`.
    pub(crate) fn keep_preview_diff_of(&mut self, path: &std::path::Path) {
        let shows =
            self.git_extras.preview.tab.as_ref().is_some_and(
                |(_, tab)| matches!(tab, Tab::Diff { path: shown, .. } if shown == path),
            );
        if shows {
            self.keep_preview_diff();
        }
    }

    /// The diff toolbar's Previous/Next file replaces a tab in place: the
    /// preview flag moves with it. Runs before `apply_diff_replacement`.
    pub(crate) fn follow_preview_replacement(&mut self) {
        let Some((current, next)) = &self.diff_replacement else {
            return;
        };
        if let Some((_, preview)) = self.git_extras.preview.tab.as_mut()
            && preview == current
        {
            *preview = next.clone();
        }
    }

    /// True when `tab` is the preview diff tab of the selected project.
    pub(crate) fn is_preview_diff(&self, tab: Option<&Tab>) -> bool {
        let (Some(tab), Some(project)) = (tab, self.selected.as_deref()) else {
            return false;
        };
        self.git_extras
            .preview
            .tab
            .as_ref()
            .is_some_and(|(owner, preview)| owner == project && preview == tab)
    }

    /// Called by `spawn_diff` for a native diff `tab` that is not open yet.
    /// During a Git panel click, replaces the preview tab and returns true;
    /// otherwise returns false, and [`Self::note_new_diff_tab`] follows.
    pub(crate) fn reuse_preview_diff(&mut self, project: &str, tab: &Tab) -> bool {
        if !self.git_extras.preview.opening {
            return false;
        }
        let Some(old) = self.reusable_preview(project, tab) else {
            return false;
        };
        self.focus_file_view(project, &old);
        self.diffs.remove(&tab.key());
        self.diff_preview.remove(&tab.key());
        self.diff_replacement = Some((old, tab.clone()));
        self.git_extras.preview.tab = Some((project.to_owned(), tab.clone()));
        true
    }

    /// A new diff tab opened in `project`: a Git panel click makes it the
    /// preview tab.
    pub(crate) fn note_new_diff_tab(&mut self, project: &str, tab: &Tab) {
        if self.git_extras.preview.opening {
            self.git_extras.preview.tab = Some((project.to_owned(), tab.clone()));
        }
    }

    /// The preview tab, when `next` may replace it: same project and
    /// repository, still open, not pinned, and no unsaved or running edit.
    fn reusable_preview(&self, project: &str, next: &Tab) -> Option<Tab> {
        let (owner, old) = self.git_extras.preview.tab.as_ref()?;
        let (
            Tab::Diff {
                remote: None,
                cwd: old_cwd,
                ..
            },
            Tab::Diff {
                remote: None,
                cwd: next_cwd,
                ..
            },
        ) = (old, next)
        else {
            return None;
        };
        if owner != project || old_cwd != next_cwd || old == next {
            return None;
        }
        let workspace = self.layouts.get(project)?;
        let group = workspace
            .tabs
            .iter()
            .find(|group| group.layout.find_tab(old).is_some())?;
        let idle = self
            .diff_reviews
            .get(&old.key())
            .is_none_or(|review| !review.busy && !review.unsaved_edits());
        (!group.pinned && idle).then(|| old.clone())
    }
}

#[cfg(test)]
mod tests {
    use crate::*;
    use std::path::PathBuf;

    fn diff(path: &str) -> Tab {
        Tab::Diff {
            remote: None,
            cwd: PathBuf::from("/repo"),
            path: PathBuf::from(path),
            staged: false,
        }
    }

    fn open(app: &mut App, path: &str) {
        app.git_extras.preview.opening = true;
        app.spawn_diff(SpawnDiff {
            cwd: PathBuf::from("/repo"),
            path: PathBuf::from(path),
            staged: false,
            native: true,
        });
        app.git_extras.preview.opening = false;
        app.apply_diff_replacement();
    }

    fn diff_tabs(app: &App) -> Vec<Tab> {
        app.layouts["p"]
            .tabs
            .iter()
            .flat_map(|group| group.layout.iter_all_tabs().map(|(_, tab)| tab.clone()))
            .filter(|tab| matches!(tab, Tab::Diff { .. }))
            .collect()
    }

    fn app(dir: &tempfile::TempDir) -> App {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let (jobs, _requests) = std::sync::mpsc::channel();
        app.jobs = jobs.into();
        app.selected = Some("p".into());
        app
    }

    #[test]
    fn git_panel_clicks_reuse_one_preview_diff_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(&dir);
        open(&mut app, "/repo/a.rs");
        open(&mut app, "/repo/b.rs");
        open(&mut app, "/repo/c.rs");
        assert_eq!(diff_tabs(&app), vec![diff("/repo/c.rs")]);
        assert!(app.is_preview_diff(Some(&diff("/repo/c.rs"))));
    }

    #[test]
    fn a_kept_preview_tab_makes_the_next_click_open_a_new_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(&dir);
        open(&mut app, "/repo/a.rs");
        app.keep_preview_diff();
        open(&mut app, "/repo/b.rs");
        assert_eq!(
            diff_tabs(&app),
            vec![diff("/repo/a.rs"), diff("/repo/b.rs")]
        );
        assert!(!app.is_preview_diff(Some(&diff("/repo/a.rs"))));
        assert!(app.is_preview_diff(Some(&diff("/repo/b.rs"))));
    }

    #[test]
    fn a_pinned_preview_tab_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(&dir);
        open(&mut app, "/repo/a.rs");
        if let Some(group) = app.layouts.get_mut("p").and_then(|w| w.tabs.last_mut()) {
            group.pinned = true;
        }
        open(&mut app, "/repo/b.rs");
        assert_eq!(diff_tabs(&app).len(), 2);
    }

    #[test]
    fn other_diff_opens_never_become_the_preview_tab() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(&dir);
        app.spawn_diff(SpawnDiff {
            cwd: PathBuf::from("/repo"),
            path: PathBuf::from("/repo/a.rs"),
            staged: false,
            native: true,
        });
        open(&mut app, "/repo/b.rs");
        assert_eq!(diff_tabs(&app).len(), 2);
        assert!(!app.is_preview_diff(Some(&diff("/repo/a.rs"))));
    }
}
