//! The Structure panel: outline source, filter, rows, and toolbar.
use super::*;

/// The outline as filtered and collapsed for this paint.
struct OutlineView<'a> {
    symbols: &'a [Symbol],
    tree: &'a OutlineTree,
    collapsed: HashSet<String>,
    filtering: bool,
}

/// What a click on one outline row asked for.
#[derive(Default)]
struct RowPick {
    jump: Option<native_lsp::Position>,
    toggle: Option<String>,
}

/// The row tracking the caret, and whether to scroll it into view.
#[derive(Clone, Copy)]
struct CurrentRow {
    index: Option<usize>,
    scroll: bool,
}

impl App {
    pub(crate) fn structure_panel(&mut self, ui: &mut egui::Ui) {
        let restart = crate::sidebar_ui::problems::lsp_panel_header(
            ui,
            "Structure",
            "Restart the language server and re-request symbols",
        );
        let Some(path) = self.active_editor_path() else {
            ui.weak("Open a file in the native editor to see its outline.");
            return;
        };
        ui.add(
            egui::Label::new(
                RichText::new(path.file_name().unwrap_or_default().to_string_lossy()).strong(),
            )
            .truncate(),
        )
        .on_hover_text(path.display().to_string());
        let Some(symbols) = self.structure_symbols(ui, &path, restart) else {
            return;
        };
        let mut tree = OutlineTree::build(&symbols);
        self.structure_toolbar(ui, &path, &tree);
        if self.structure.alphabetical {
            tree.sort_alphabetically(&symbols);
        }
        let query = self.structure.filter.trim().to_owned();
        let view = OutlineView {
            symbols: &symbols,
            tree: &tree,
            collapsed: HashSet::new(),
            filtering: !query.is_empty(),
        };
        // Keep matches and every ancestor of a match.
        let keep = view.keep(&query);
        let view = OutlineView {
            collapsed: self
                .structure
                .collapsed
                .get(&path)
                .cloned()
                .unwrap_or_default(),
            ..view
        };
        let rows = view.rows(&keep);
        if rows.is_empty() {
            ui.weak("No symbols match the filter.");
            return;
        }
        let current = self.structure_current(&path, &view, &rows);
        let mut jump: Option<native_lsp::Position> = None;
        let mut toggle: Option<String> = None;
        appearance::sidebar_scroll("structure")
            .max_height(ui.available_height())
            .show(ui, |ui| {
                for (index, depth) in &rows {
                    let pick = view.paint_row(ui, (*index, *depth), current);
                    if pick.jump.is_some() {
                        jump = pick.jump;
                    }
                    if pick.toggle.is_some() {
                        toggle = pick.toggle;
                    }
                }
            });
        if let Some(key) = toggle {
            let set = self.structure.collapsed.entry(path.clone()).or_default();
            if !set.remove(&key) {
                set.insert(key);
            }
        }
        if let Some(position) = jump {
            self.open_file_at(path, position);
        }
    }

    /// The outline to show: Markdown headings, the server's symbols, or the
    /// syntax tree's. `None` after painting why there is nothing to show.
    fn structure_symbols(
        &mut self,
        ui: &mut egui::Ui,
        path: &Path,
        restart: bool,
    ) -> Option<Arc<[Symbol]>> {
        if let Some(outline) = self.markdown_outline(path) {
            return Some(outline);
        }
        let language =
            terminator_native_edit::highlight::language_for_path(&path.to_string_lossy());
        let served = native_lsp::server_language(language).is_some();
        if served && restart {
            if self.lsp.tracked(path) {
                self.lsp.restart(path);
            } else {
                self.lsp.restart_all();
            }
        }
        // The server's outline when it has one; else the syntax tree's,
        // with a note on why the server is not answering yet.
        let note = self.structure_server_note(path, served);
        let served_symbols = if served && note.is_none() {
            self.lsp.symbols_for(path)
        } else {
            Arc::from(Vec::new())
        };
        if !served_symbols.is_empty() {
            return Some(served_symbols);
        }
        if let Some(outline) = self.syntax_outline(path) {
            ui.weak(match &note {
                Some(note) => format!("Syntax outline · {note}"),
                None if served => "Syntax outline while the language server indexes.".to_owned(),
                None => "Syntax outline (no language server for this file type).".to_owned(),
            });
            return Some(outline);
        }
        if !served {
            ui.weak("No document outline for this file type.");
            ui.weak(
                "Structure supports Markdown headings, and Rust, Python, JavaScript and \
                 TypeScript through a language server.",
            );
            return None;
        }
        if let Some(note) = note {
            ui.weak(note);
            return None;
        }
        Some(served_symbols)
    }

    /// Why the server has no outline yet; requests symbols when it can.
    fn structure_server_note(&mut self, path: &Path, served: bool) -> Option<String> {
        if !served {
            None
        } else if !self.lsp.tracked(path) {
            Some("Waiting for the native editor to load this file.".to_owned())
        } else if let Some(note) = self.lsp.server_note(path) {
            Some(note)
        } else {
            self.lsp.request_symbols(path);
            None
        }
    }

    /// Autoscroll from source: the caret's innermost symbol, or its
    /// nearest visible ancestor when that one is collapsed away.
    fn structure_current(
        &mut self,
        path: &PathBuf,
        view: &OutlineView<'_>,
        rows: &[(usize, usize)],
    ) -> CurrentRow {
        let caret = (!self.structure.autoscroll_off)
            .then(|| self.native_docs.get(path).and_then(|doc| doc.caret_line()))
            .flatten()
            .and_then(|line| u32::try_from(line).ok());
        let visible: HashSet<usize> = rows.iter().map(|(index, _)| *index).collect();
        let mut current = caret.and_then(|line| native_lsp::innermost_symbol(view.symbols, line));
        while let Some(index) = current
            && !visible.contains(&index)
        {
            current = view.tree.parent.get(index).copied().flatten();
        }
        let current_key = current.and_then(|index| view.tree.keys.get(index).cloned());
        let scroll = current_key.is_some()
            && self.structure.followed.as_ref().map(|(p, k)| (p, Some(k)))
                != Some((path, current_key.as_ref()));
        if scroll {
            self.structure.followed = current_key.map(|key| (path.clone(), key));
        }
        CurrentRow {
            index: current,
            scroll,
        }
    }

    /// Filter field plus sort, autoscroll, and collapse-all toggles.
    fn structure_toolbar(&mut self, ui: &mut egui::Ui, path: &Path, tree: &OutlineTree) {
        ui.horizontal(|ui| {
            let buttons = 3.0 * (ui.spacing().interact_size.y + ui.spacing().item_spacing.x);
            ui.add(
                appearance::singleline(&mut self.structure.filter)
                    .hint_text("Filter symbols…")
                    .desired_width((ui.available_width() - buttons).max(60.0)),
            );
            if appearance::selectable_icon(appearance::SelectableIconInput {
                ui,
                icon: "ArrowDownWideNarrow",
                tip: "Sort alphabetically",
                selected: self.structure.alphabetical,
            })
            .clicked()
            {
                self.structure.alphabetical = !self.structure.alphabetical;
            }
            if appearance::selectable_icon(appearance::SelectableIconInput {
                ui,
                icon: "LocateFixed",
                tip: "Autoscroll from source",
                selected: !self.structure.autoscroll_off,
            })
            .clicked()
            {
                self.structure.autoscroll_off = !self.structure.autoscroll_off;
                self.structure.followed = None;
            }
            self.structure_collapse_all(ui, path, tree);
        });
    }

    fn structure_collapse_all(&mut self, ui: &mut egui::Ui, path: &Path, tree: &OutlineTree) {
        let parents: Vec<String> = (0..tree.keys.len())
            .filter(|index| tree.has_children(*index))
            .filter_map(|index| tree.keys.get(index).cloned())
            .collect();
        let set = self.structure.collapsed.entry(path.to_owned()).or_default();
        let all_collapsed = parents.iter().all(|key| set.contains(key));
        let (icon, tip) = if all_collapsed {
            ("ChevronDown", "Expand all")
        } else {
            ("ChevronsDownUp", "Collapse all")
        };
        if appearance::sidebar_action(ui, icon, tip).clicked() {
            if all_collapsed {
                set.clear();
            } else {
                set.extend(parents);
            }
        }
    }
}

impl OutlineView<'_> {
    fn expanded(&self, index: usize) -> bool {
        self.filtering
            || self
                .tree
                .keys
                .get(index)
                .is_none_or(|key| !self.collapsed.contains(key))
    }

    /// Symbols kept by the filter: matches and every ancestor of a match.
    fn keep(&self, query: &str) -> Vec<bool> {
        let mut keep = vec![!self.filtering; self.symbols.len()];
        if !self.filtering {
            return keep;
        }
        for (index, symbol) in self.symbols.iter().enumerate().rev() {
            let hit =
                outline_matches(&symbol.name, query)
                    || self.tree.children.get(index).is_some_and(|children| {
                        children.iter().any(|c| keep.get(*c) == Some(&true))
                    });
            if let Some(slot) = keep.get_mut(index) {
                *slot = hit;
            }
        }
        keep
    }

    /// Depth-first visible rows as `(index, depth)`.
    fn rows(&self, keep: &[bool]) -> Vec<(usize, usize)> {
        let mut rows: Vec<(usize, usize)> = Vec::new();
        let mut stack: Vec<(usize, usize)> =
            self.tree.roots.iter().rev().map(|r| (*r, 0)).collect();
        while let Some((index, depth)) = stack.pop() {
            if keep.get(index) != Some(&true) {
                continue;
            }
            rows.push((index, depth));
            if self.expanded(index)
                && let Some(children) = self.tree.children.get(index)
            {
                stack.extend(children.iter().rev().map(|c| (*c, depth.saturating_add(1))));
            }
        }
        rows
    }

    /// One row; a chevron click (or double click) toggles, any other
    /// click jumps to the symbol.
    fn paint_row(
        &self,
        ui: &mut egui::Ui,
        (index, depth): (usize, usize),
        current: CurrentRow,
    ) -> RowPick {
        let mut pick = RowPick::default();
        let Some(symbol) = self.symbols.get(index) else {
            return pick;
        };
        let parent = self.tree.has_children(index);
        let selected = current.index == Some(index);
        let response = outline_row(OutlineRowInput {
            ui,
            symbol,
            depth,
            parent,
            expanded: self.expanded(index),
            selected,
        });
        if selected && current.scroll {
            response.scroll_to_me(None);
        }
        if response.clicked() {
            let chevron_end = response.rect.left() + depth as f32 * INDENT + CHEVRON;
            let on_chevron = response
                .interact_pointer_pos()
                .is_some_and(|pos| pos.x < chevron_end);
            if parent && on_chevron && !self.filtering {
                pick.toggle = self.tree.keys.get(index).cloned();
            } else {
                pick.jump = Some(symbol.selection.start);
            }
        }
        if response.double_clicked() && parent && !self.filtering {
            pick.toggle = self.tree.keys.get(index).cloned();
        }
        pick
    }
}
