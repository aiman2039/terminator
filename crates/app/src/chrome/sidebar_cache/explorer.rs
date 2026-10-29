//! Explorer tree rows: a depth-first walk over loaded directory listings.
use super::{ExplorerCache, ExplorerRow};
use crate::App;
use crate::preferences::ExplorerView;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use terminator_git::Entry;

/// Deepest folder level the explorer walks.
const MAX_DEPTH: usize = 20;
/// Longest compact folder chain (`a/b/c`).
const MAX_CHAIN: usize = 20;
/// Most folders a name filter lists and watches, collapsed ones included.
const MAX_FILTER_DIRS: usize = 500;

impl App {
    pub(crate) fn explorer_rows(&mut self, root: &Path) -> Arc<Vec<ExplorerRow>> {
        let reveal_file = self
            .explorer_reveal
            .as_ref()
            .filter(|reveal| {
                reveal.root == root && self.selected.as_deref() == Some(reveal.project.as_str())
            })
            .map(|reveal| reveal.file.clone());
        if let Some(rows) = self.cached_explorer_rows(root, reveal_file.as_deref()) {
            return rows;
        }
        let query = (!self.explorer_query.is_empty()).then(|| self.explorer_query.to_lowercase());
        let mut walk = ExplorerWalk {
            filter: ShownFilter {
                app: self,
                reveal_file: reveal_file.as_deref(),
            },
            query: query.as_deref(),
            rows: Vec::new(),
            dirs: Vec::new(),
            filter_budget: MAX_FILTER_DIRS,
        };
        walk.walk(root, 0);
        let ExplorerWalk { rows, dirs, .. } = walk;
        self.visible_dirs.extend(dirs.iter().cloned());
        let rows = Arc::new(rows);
        let cache = self.sidebar_cache.get_mut();
        cache.explorer = Some(ExplorerCache {
            revision: cache.files_revision,
            root: root.to_owned(),
            expanded: self.expanded_dirs.clone(),
            ignored: self.preferences.show_ignored,
            flat: self.preferences.explorer_flat_folders,
            view: self.preferences.explorer_view.clone(),
            query: self.explorer_query.clone(),
            reveal_file,
            rows: Arc::clone(&rows),
            dirs,
        });
        rows
    }

    /// Reuses the cached rows when nothing they depend on changed.
    fn cached_explorer_rows(
        &mut self,
        root: &Path,
        reveal_file: Option<&Path>,
    ) -> Option<Arc<Vec<ExplorerRow>>> {
        let cache = self.sidebar_cache.get_mut();
        let model = cache.explorer.as_ref()?;
        let fresh = model.revision == cache.files_revision
            && model.root == root
            && model.expanded == self.expanded_dirs
            && model.ignored == self.preferences.show_ignored
            && model.flat == self.preferences.explorer_flat_folders
            && model.view == self.preferences.explorer_view
            && model.query == self.explorer_query
            && model.reveal_file.as_deref() == reveal_file;
        if !fresh {
            return None;
        }
        self.visible_dirs.extend(model.dirs.iter().cloned());
        Some(Arc::clone(&model.rows))
    }
}

/// Decides which entries the explorer shows.
#[derive(Clone, Copy)]
struct ShownFilter<'a> {
    app: &'a App,
    reveal_file: Option<&'a Path>,
}

impl ShownFilter<'_> {
    fn shown(self, entry: &Entry) -> bool {
        !entry.ignored
            || self.app.preferences.show_ignored
            || self
                .reveal_file
                .is_some_and(|file| file.starts_with(&entry.path))
    }
}

struct ExplorerWalk<'a> {
    filter: ShownFilter<'a>,
    query: Option<&'a str>,
    rows: Vec<ExplorerRow>,
    dirs: Vec<PathBuf>,
    /// Folders a filtered walk may still enter.
    filter_budget: usize,
}

/// Listing order for non-default view options. Listings arrive folders
/// first with natural name order; this re-sorts them.
fn view_order(view: &ExplorerView, a: &Entry, b: &Entry) -> std::cmp::Ordering {
    let folders = if view.folders_mixed {
        std::cmp::Ordering::Equal
    } else {
        b.directory.cmp(&a.directory)
    };
    let extension = |entry: &Entry| {
        (!entry.directory && view.sort_by_type)
            .then(|| entry.path.extension())
            .flatten()
            .map(|ext| ext.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };
    folders
        .then_with(|| terminator_git::natural_cmp(&extension(a), &extension(b)))
        .then_with(|| terminator_git::natural_cmp(&entry_name(a), &entry_name(b)))
        .then_with(|| a.path.cmp(&b.path))
}

fn entry_name(entry: &Entry) -> String {
    entry
        .path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

impl<'a> ExplorerWalk<'a> {
    /// Rows for the listing of `path`. Returns whether any row matched
    /// the filter (always true without a filter).
    fn walk(&mut self, path: &Path, depth: usize) -> bool {
        if depth > MAX_DEPTH {
            return false;
        }
        if self.query.is_some() {
            if self.filter_budget == 0 {
                return false;
            }
            self.filter_budget = self.filter_budget.saturating_sub(1);
        }
        self.dirs.push(path.to_owned());
        let app = self.filter.app;
        // A filtered tree lists folders in the background: hide their
        // "Loading…" rows and show matches as they arrive.
        if self.query.is_none()
            && (app.directory_errors.contains_key(path) || !app.dirs.contains_key(path))
        {
            self.rows.push(ExplorerRow {
                entry: None,
                path: path.to_owned(),
                label: String::new(),
                depth,
                chain: Vec::new(),
                filtered_open: false,
            });
        }
        let Some(entries) = app.dirs.get(path) else {
            return self.query.is_none();
        };
        let filter = self.filter;
        let mut shown: Vec<&'a Entry> =
            entries.iter().filter(|entry| filter.shown(entry)).collect();
        let view = &app.preferences.explorer_view;
        if *view != ExplorerView::default() {
            shown.sort_by(|a, b| view_order(view, a, b));
        }
        let mut matched = self.query.is_none();
        for entry in shown {
            matched |= self.visit(entry, depth);
        }
        matched
    }

    /// With a filter, a file stays when its name matches and a folder
    /// stays when its name or something inside it matches. A matching
    /// folder shows its contents unfiltered, as usual.
    fn visit(&mut self, entry: &'a Entry, depth: usize) -> bool {
        let label = entry_name(entry);
        let Some(query) = self.query else {
            self.visit_unfiltered(entry, label, depth);
            return true;
        };
        let name_matches = crate::sidebar_ui::explorer_query_keeps_file(&label, false, Some(query));
        if !entry.directory {
            if name_matches {
                self.visit_unfiltered(entry, label, depth);
            }
            return name_matches;
        }
        if name_matches {
            self.query = None;
            self.visit_unfiltered(entry, label, depth);
            self.query = Some(query);
            return true;
        }
        let start = self.rows.len();
        self.rows.push(ExplorerRow {
            entry: Some(entry.clone()),
            path: entry.path.clone(),
            label,
            depth,
            chain: Vec::new(),
            filtered_open: true,
        });
        if self.walk(&entry.path, depth.saturating_add(1)) {
            return true;
        }
        self.rows.truncate(start);
        false
    }

    fn visit_unfiltered(&mut self, entry: &'a Entry, label: String, depth: usize) {
        let chain = self.compact_chain(entry);
        let Some(&deepest) = chain.last() else {
            return;
        };
        let expanded = deepest.directory
            && chain
                .iter()
                .any(|link| self.filter.app.expanded_dirs.contains(&link.path));
        let compact = chain.len() > 1;
        self.rows.push(ExplorerRow {
            entry: Some(deepest.clone()),
            path: deepest.path.clone(),
            label: if compact {
                chain
                    .iter()
                    .map(|link| entry_name(link))
                    .collect::<Vec<_>>()
                    .join("/")
            } else {
                label
            },
            depth,
            chain: if compact {
                chain.iter().map(|link| link.path.clone()).collect()
            } else {
                Vec::new()
            },
            filtered_open: false,
        });
        if expanded {
            // Keep the chain's own listings fresh: they decide
            // whether it stays compact.
            let upper = chain.len().saturating_sub(1);
            self.dirs
                .extend(chain.iter().take(upper).map(|link| link.path.clone()));
            self.walk(&deepest.path, depth.saturating_add(1));
        }
    }

    /// Compact folders: follow folders whose only shown child is a folder,
    /// as far as listings are loaded.
    fn compact_chain(&self, entry: &'a Entry) -> Vec<&'a Entry> {
        let mut chain = vec![entry];
        let app = self.filter.app;
        if !entry.directory || self.query.is_some() || app.preferences.explorer_flat_folders {
            return chain;
        }
        let filter = self.filter;
        while chain.len() < MAX_CHAIN
            && let Some(last) = chain.last()
            && let Some(children) = app.dirs.get(&last.path)
            && let [only] = children
                .iter()
                .filter(|e| filter.shown(e))
                .collect::<Vec<_>>()[..]
            && only.directory
        {
            chain.push(only);
        }
        chain
    }
}
