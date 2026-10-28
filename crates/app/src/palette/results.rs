use super::*;

impl App {
    /// Empty field: a few recent files, then live sessions and agents,
    /// then more files from this project.
    fn idle_palette(&self, items: Vec<PaletteItem>) -> Vec<PaletteItem> {
        let mut out = Vec::new();
        let mut seen_files = HashSet::new();
        if self.palette_wants(FILTER_FILES) {
            for path in &self.preferences.recent_native_files {
                if !self.path_in_palette_project(path) {
                    continue;
                }
                if seen_files.insert(path.clone()) {
                    out.push(PaletteItem::File(path.clone()));
                }
                if seen_files.len() >= RECENT_RESULTS {
                    break;
                }
            }
        }
        out.extend(
            items
                .iter()
                .filter(|item| {
                    self.palette_accept(item)
                        && matches!(item, PaletteItem::Session(..) | PaletteItem::Agent(..))
                })
                .cloned(),
        );
        if self.palette.filters != 0 {
            out.extend(
                items
                    .iter()
                    .filter(|item| {
                        self.palette_accept(item)
                            && !matches!(
                                item,
                                PaletteItem::File(_)
                                    | PaletteItem::Session(..)
                                    | PaletteItem::Agent(..)
                            )
                    })
                    .cloned(),
            );
        }
        if self.palette_wants(FILTER_FILES) {
            let room = FILE_RESULTS.saturating_sub(seen_files.len());
            if self.file_index.is_empty() {
                out.extend(
                    items
                        .into_iter()
                        .filter(|item| {
                            let PaletteItem::File(path) = item else {
                                return false;
                            };
                            self.palette_accept(item) && seen_files.insert(path.clone())
                        })
                        .take(room),
                );
            } else if let Some(root) = self.palette_project_root() {
                let exclude = self.palette_exclude_roots();
                out.extend(
                    self.file_index
                        .query_scored_in(terminator_native_edit::finder::QueryScoredInInput {
                            root: Some(root),
                            exclude: &exclude,
                            pattern: "",
                            limit: room,
                        })
                        .into_iter()
                        .map(|(_, file)| file.path)
                        .filter(|path| seen_files.insert(path.clone()))
                        .map(PaletteItem::File),
                );
            }
        }
        out.truncate(MAX_RESULTS);
        out
    }

    /// File Structure: the scoped file's outline in document order,
    /// indented by depth, fuzzy-filtered on symbol names.
    fn file_structure_items(&self, path: &Path) -> Vec<PaletteItem> {
        let markdown = crate::markdown::supported(path)
            .then(|| {
                self.native_docs
                    .get(path)
                    .and_then(|doc| doc.buffer())
                    .map(native_lsp::markdown_outline)
            })
            .flatten();
        let symbols: Vec<native_lsp::Symbol> = markdown.unwrap_or_else(|| {
            let served = self.lsp.symbols_for(path).to_vec();
            if !served.is_empty() {
                return served;
            }
            // No server answer: the syntax tree's outline instead.
            self.native_docs
                .get(path)
                .and_then(|doc| doc.buffer())
                .and_then(|buffer| {
                    terminator_native_edit::treesitter::outline(
                        terminator_native_edit::highlight::language_for_path(
                            &path.to_string_lossy(),
                        ),
                        &buffer.text(),
                    )
                })
                .unwrap_or_default()
        });
        let query = self.palette.query.trim();
        let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
        let mut needle_buf = Vec::new();
        let needle = nucleo::Utf32Str::new(query, &mut needle_buf);
        let mut haystack_buf = Vec::new();
        symbols
            .into_iter()
            .filter(|symbol| {
                query.is_empty()
                    || matcher
                        .fuzzy_match(
                            nucleo::Utf32Str::new(&symbol.name, &mut haystack_buf),
                            needle,
                        )
                        .is_some()
            })
            .take(MAX_FILE_STRUCTURE)
            .map(|symbol| PaletteItem::Symbol {
                path: path.to_owned(),
                // Filtered results flatten; the full outline shows nesting.
                name: if query.is_empty() {
                    format!("{}{}", "   ".repeat(symbol.depth), symbol.name)
                } else {
                    symbol.name
                },
                kind: symbol.kind,
                line: symbol.selection.start.line,
                column: symbol.selection.start.character,
                container: None,
            })
            .collect()
    }

    pub(crate) fn filtered_palette(&self) -> Vec<PaletteItem> {
        if let Some(scope) = self.palette.file_scope.as_deref() {
            return self.file_structure_items(scope);
        }
        let query = self.palette.query.trim();
        let items = self.palette_items();
        if query.is_empty() {
            return self.idle_palette(items);
        }
        let mut ranked = self.rank_palette_items(query, items);
        self.rank_index_files(query, &mut ranked);
        ranked.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.cmp(&b.1))
                .then_with(|| a.2.cmp(&b.2))
        });
        ranked
            .into_iter()
            .take(MAX_RESULTS)
            .map(|(_, _, _, item)| item)
            .collect()
    }

    fn rank_palette_items(&self, query: &str, items: Vec<PaletteItem>) -> Vec<RankedItem> {
        let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
        let mut needle_buf = Vec::new();
        let needle = nucleo::Utf32Str::new(query, &mut needle_buf);
        let mut haystack_buf = Vec::new();
        let mut ranked = Vec::new();
        for item in items {
            if !self.palette_accept(&item) {
                continue;
            }
            // A ready index ranks files on its own, under the current project.
            if !self.file_index.is_empty() && matches!(item, PaletteItem::File(_)) {
                continue;
            }
            let text = self.palette_match_text(&item);
            let haystack = nucleo::Utf32Str::new(&text, &mut haystack_buf);
            let Some(score) = matcher.fuzzy_match(haystack, needle) else {
                continue;
            };
            let title = item.title();
            let boost = if title.eq_ignore_ascii_case(query) {
                80
            } else if title.to_lowercase().starts_with(&query.to_lowercase()) {
                25
            } else {
                0
            };
            ranked.push((score.saturating_add(boost), item.kind_rank(), title, item));
        }
        ranked
    }

    /// Symbols rank on their name (and container), never on the file path
    /// or kind: `main` must not match all of `main.rs`.
    fn palette_match_text(&self, item: &PaletteItem) -> String {
        match item {
            PaletteItem::Symbol {
                name, container, ..
            } => match container {
                Some(container) => format!("{name} {container}"),
                None => name.clone(),
            },
            _ => item.search_text(self),
        }
    }

    fn rank_index_files(&self, query: &str, ranked: &mut Vec<RankedItem>) {
        if !self.palette_wants(FILTER_FILES) || self.file_index.is_empty() {
            return;
        }
        let Some(root) = self.palette_project_root() else {
            return;
        };
        let exclude = self.palette_exclude_roots();
        for (score, file) in
            self.file_index
                .query_scored_in(terminator_native_edit::finder::QueryScoredInInput {
                    root: Some(root),
                    exclude: &exclude,
                    pattern: query,
                    limit: FILE_RESULTS,
                })
        {
            let item = PaletteItem::File(file.path);
            let title = item.title();
            let boost = if title.eq_ignore_ascii_case(query) {
                80
            } else {
                0
            };
            ranked.push((score.saturating_add(boost), item.kind_rank(), title, item));
        }
    }
}

/// Score, kind rank, and title: the sort keys for a matched item.
type RankedItem = (u16, u8, String, PaletteItem);
