//! Explorer tree rows: a depth-first walk over loaded directory listings.
use super::{ExplorerCache, ExplorerRow};
use crate::App;
use crate::preferences::ExplorerView;
use std::{
    collections::HashMap,
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
            changed_only: self.preferences.explorer_view.changed_only,
            rows: Vec::new(),
            dirs: Vec::new(),
            filter_budget: MAX_FILTER_DIRS,
            nested: HashMap::new(),
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
    fn excluded(self, entry: &Entry) -> bool {
        self.app
            .preferences
            .explorer_view
            .excluded
            .contains(&entry.path)
    }

    fn shown(self, entry: &Entry) -> bool {
        (!entry.ignored && !self.excluded(entry))
            || self.app.preferences.show_ignored
            || self
                .reveal_file
                .is_some_and(|file| file.starts_with(&entry.path))
    }

    /// The entry a row keeps: excluded entries paint like ignored ones.
    fn row_entry(self, entry: &Entry) -> Entry {
        let mut entry = entry.clone();
        entry.ignored |= self.excluded(&entry);
        entry
    }

    /// Whether Git reports a change at `path` (a folder: under it).
    fn changed(self, path: &Path) -> bool {
        self.app
            .context
            .as_ref()
            .and_then(|context| context.decorations.get(path))
            .is_some_and(|status| *status != ' ')
    }
}

struct ExplorerWalk<'a> {
    filter: ShownFilter<'a>,
    query: Option<&'a str>,
    /// "Changed files only" scope.
    changed_only: bool,
    rows: Vec<ExplorerRow>,
    dirs: Vec<PathBuf>,
    /// Folders a filtered walk may still enter.
    filter_budget: usize,
    /// Files nested under each parent file, for listings walked so far.
    nested: HashMap<PathBuf, Vec<&'a Entry>>,
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
    /// A name filter or a scope hides rows that do not match.
    fn filtering(&self) -> bool {
        self.query.is_some() || self.changed_only
    }

    /// Rows for the listing of `path`. Returns whether any row matched
    /// the filter (always true without a filter).
    fn walk(&mut self, path: &Path, depth: usize) -> bool {
        if depth > MAX_DEPTH {
            return false;
        }
        if self.filtering() {
            if self.filter_budget == 0 {
                return false;
            }
            self.filter_budget = self.filter_budget.saturating_sub(1);
        }
        self.dirs.push(path.to_owned());
        let app = self.filter.app;
        // A filtered tree lists folders in the background: hide their
        // "Loading…" rows and show matches as they arrive.
        if !self.filtering()
            && (app.directory_errors.contains_key(path) || !app.dirs.contains_key(path))
        {
            self.rows.push(ExplorerRow {
                entry: None,
                path: path.to_owned(),
                label: String::new(),
                depth,
                chain: Vec::new(),
                filtered_open: false,
                nests: false,
            });
        }
        let Some(entries) = app.dirs.get(path) else {
            return !self.filtering();
        };
        let filter = self.filter;
        let mut shown: Vec<&'a Entry> =
            entries.iter().filter(|entry| filter.shown(entry)).collect();
        let view = &app.preferences.explorer_view;
        if view.folders_mixed || view.sort_by_type {
            shown.sort_by(|a, b| view_order(view, a, b));
        }
        if !view.nesting_off && !self.filtering() {
            let (top, nested) = super::nesting::group(shown);
            shown = top;
            self.nested.extend(nested);
        }
        let mut matched = !self.filtering();
        for entry in shown {
            matched |= self.visit(entry, depth);
        }
        matched
    }

    /// With a filter, a file stays when its name matches (and, in the
    /// Changed scope, it has changes) and a folder stays when something
    /// inside it stays. Without a scope, a folder whose name matches shows
    /// its contents unfiltered, as usual.
    fn visit(&mut self, entry: &'a Entry, depth: usize) -> bool {
        let label = entry_name(entry);
        if !self.filtering() {
            self.visit_unfiltered(entry, label, depth);
            return true;
        }
        if self.changed_only && !self.filter.changed(&entry.path) {
            return false;
        }
        let name_matches = crate::sidebar_ui::explorer_query_keeps_file(&label, false, self.query);
        if !entry.directory {
            if name_matches {
                self.visit_unfiltered(entry, label, depth);
            }
            return name_matches;
        }
        if name_matches && self.query.is_some() && !self.changed_only {
            let query = self.query.take();
            self.visit_unfiltered(entry, label, depth);
            self.query = query;
            return true;
        }
        let start = self.rows.len();
        self.rows.push(ExplorerRow {
            entry: Some(self.filter.row_entry(entry)),
            path: entry.path.clone(),
            label,
            depth,
            chain: Vec::new(),
            filtered_open: true,
            nests: false,
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
        let expanded_dirs = &self.filter.app.expanded_dirs;
        let expanded = chain.iter().any(|link| expanded_dirs.contains(&link.path));
        let nested = self.nested.remove(&deepest.path).unwrap_or_default();
        let compact = chain.len() > 1;
        let label = if compact {
            chain
                .iter()
                .map(|link| entry_name(link))
                .collect::<Vec<_>>()
                .join("/")
        } else if !nested.is_empty() && !expanded {
            // A closed nesting parent says how many files it holds.
            format!("{label} +{}", nested.len())
        } else {
            label
        };
        self.rows.push(ExplorerRow {
            entry: Some(self.filter.row_entry(deepest)),
            path: deepest.path.clone(),
            label,
            depth,
            chain: if compact {
                chain.iter().map(|link| link.path.clone()).collect()
            } else {
                Vec::new()
            },
            filtered_open: false,
            nests: !nested.is_empty(),
        });
        if !deepest.directory {
            if expanded {
                for child in nested {
                    let label = entry_name(child);
                    self.visit_unfiltered(child, label, depth.saturating_add(1));
                }
            }
            return;
        }
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
        if !entry.directory || self.filtering() || app.preferences.explorer_flat_folders {
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
