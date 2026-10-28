//! Explorer tree rows: a depth-first walk over loaded directory listings.
use super::{ExplorerCache, ExplorerRow};
use crate::App;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use terminator_git::Entry;

/// Deepest folder level the explorer walks.
const MAX_DEPTH: usize = 20;
/// Longest compact folder chain (`a/b/c`).
const MAX_CHAIN: usize = 20;

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
    fn walk(&mut self, path: &Path, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        self.dirs.push(path.to_owned());
        let app = self.filter.app;
        if app.directory_errors.contains_key(path) || !app.dirs.contains_key(path) {
            self.rows.push(ExplorerRow {
                entry: None,
                path: path.to_owned(),
                label: String::new(),
                depth,
                chain: Vec::new(),
            });
        }
        let Some(entries) = app.dirs.get(path) else {
            return;
        };
        let filter = self.filter;
        for entry in entries.iter().filter(|entry| filter.shown(entry)) {
            self.visit(entry, depth);
        }
    }

    fn visit(&mut self, entry: &'a Entry, depth: usize) {
        let label = entry_name(entry);
        if !crate::sidebar_ui::explorer_query_keeps_file(&label, entry.directory, self.query) {
            return;
        }
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
