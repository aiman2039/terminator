//! Structure tool window: the active editor's outline as a collapsible
//! tree (`IntelliJ` Structure). Language servers supply code outlines;
//! Markdown headings come from the buffer itself. The caret's enclosing
//! symbol is selected and scrolled into view as it moves.

use crate::{App, Tab, appearance};
use eframe::egui::{self, RichText};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use terminator_native_edit::doc::Buffer;
use terminator_native_edit::lsp::{self as native_lsp, Symbol};

const ROW_HEIGHT: f32 = 22.0;
const INDENT: f32 = 14.0;
const CHEVRON: f32 = 16.0;
const BADGE: f32 = 16.0;

/// Panel options and per-file tree state. Collapsed nodes are keyed by
/// their name path, so they survive edits that shift symbol indices.
#[derive(Default)]
pub(crate) struct StructureState {
    collapsed: HashMap<PathBuf, HashSet<String>>,
    filter: String,
    alphabetical: bool,
    /// Autoscroll from source is on by default; this turns it off.
    autoscroll_off: bool,
    markdown: Option<(PathBuf, u64, Arc<[Symbol]>)>,
    /// Syntax-tree outline (no language server answer yet), per revision.
    syntax: Option<(PathBuf, u64, Arc<[Symbol]>)>,
    /// Last caret-selected node, so the tree scrolls only when it changes.
    followed: Option<(PathBuf, String)>,
}

/// Parent links, child lists (document order), roots, and name-path keys
/// for a depth-tagged flat outline.
pub(crate) struct OutlineTree {
    pub(crate) parent: Vec<Option<usize>>,
    pub(crate) children: Vec<Vec<usize>>,
    pub(crate) roots: Vec<usize>,
    pub(crate) keys: Vec<String>,
}

impl OutlineTree {
    pub(crate) fn build(symbols: &[Symbol]) -> Self {
        let len = symbols.len();
        let mut tree = Self {
            parent: vec![None; len],
            children: vec![Vec::new(); len],
            roots: Vec::new(),
            keys: Vec::with_capacity(len),
        };
        let mut stack: Vec<usize> = Vec::new();
        for (index, symbol) in symbols.iter().enumerate() {
            while stack
                .last()
                .and_then(|top| symbols.get(*top))
                .is_some_and(|top| top.depth >= symbol.depth)
            {
                stack.pop();
            }
            let parent = stack.last().copied();
            if let Some(slot) = tree.parent.get_mut(index) {
                *slot = parent;
            }
            let key = match parent.and_then(|parent| tree.keys.get(parent)) {
                Some(prefix) => format!("{prefix}\u{1f}{}", symbol.name),
                None => symbol.name.clone(),
            };
            tree.keys.push(key);
            match parent.and_then(|parent| tree.children.get_mut(parent)) {
                Some(children) => children.push(index),
                None => tree.roots.push(index),
            }
            stack.push(index);
        }
        tree
    }

    fn sort_alphabetically(&mut self, symbols: &[Symbol]) {
        let name = |index: &usize| {
            symbols
                .get(*index)
                .map(|symbol| symbol.name.to_lowercase())
                .unwrap_or_default()
        };
        self.roots.sort_by_key(name);
        for children in &mut self.children {
            children.sort_by_key(name);
        }
    }

    fn has_children(&self, index: usize) -> bool {
        self.children
            .get(index)
            .is_some_and(|children| !children.is_empty())
    }
}

/// Case-insensitive subsequence match, the tree's speed search.
pub(crate) fn outline_matches(name: &str, query: &str) -> bool {
    let mut name = name.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| !c.is_whitespace())
        .all(|wanted| name.any(|c| c == wanted))
}

/// `IntelliJ`-style kind badge: a letter on a colored disc.
fn kind_badge(kind: &str) -> (char, egui::Color32) {
    let rgb = |r, g, b| egui::Color32::from_rgb(r, g, b);
    match kind {
        "Function" => ('f', rgb(0xD0, 0x8B, 0x3A)),
        "Method" => ('m', rgb(0xD0, 0x8B, 0x3A)),
        "Constructor" => ('c', rgb(0xD0, 0x8B, 0x3A)),
        "Struct" => ('S', rgb(0x4A, 0x8C, 0xD0)),
        "Class" | "Object" => ('C', rgb(0x4A, 0x8C, 0xD0)),
        "Enum" => ('E', rgb(0x9A, 0x6C, 0xD6)),
        "Enum member" => ('e', rgb(0x9A, 0x6C, 0xD6)),
        "Interface" => ('T', rgb(0x5A, 0xA6, 0x5C)),
        "Field" | "Property" | "Key" => ('p', rgb(0x3E, 0xA6, 0xA6)),
        "Variable" => ('v', rgb(0x3E, 0xA6, 0xA6)),
        "Constant" => ('K', rgb(0x3E, 0xA6, 0xA6)),
        "Module" | "Namespace" | "Package" | "File" => ('M', rgb(0x9C, 0x7B, 0x55)),
        "Type parameter" => ('T', rgb(0x80, 0x80, 0x80)),
        heading if heading.starts_with('H') => ('#', rgb(0x80, 0x80, 0x80)),
        _ => ('•', rgb(0x80, 0x80, 0x80)),
    }
}

impl App {
    /// Path of the native editor in the selected project's active pane.
    pub(crate) fn active_editor_path(&self) -> Option<PathBuf> {
        let project_id = self.selected.clone()?;
        match self
            .layouts
            .get(&project_id)
            .and_then(|workspace| workspace.active_pane().cloned())
        {
            Some(Tab::NativeEditor { path }) => Some(path),
            _ => None,
        }
    }

    /// Heading outline for a loaded Markdown buffer, cached per revision.
    /// `None` for other files (their outline comes from a server).
    pub(crate) fn markdown_outline(&mut self, path: &Path) -> Option<Arc<[Symbol]>> {
        if !crate::markdown::supported(path) {
            return None;
        }
        let doc = self.native_docs.get(path)?.buffer()?;
        let revision = doc.revision();
        if let Some((cached, at, outline)) = &self.structure.markdown
            && cached == path
            && *at == revision
        {
            return Some(Arc::clone(outline));
        }
        let outline: Arc<[Symbol]> = native_lsp::markdown_outline(doc).into();
        self.structure.markdown = Some((path.to_owned(), revision, Arc::clone(&outline)));
        Some(outline)
    }

    /// Outline from the syntax tree for a loaded buffer, cached per
    /// revision; `None` for languages without a bundled grammar.
    pub(crate) fn syntax_outline(&mut self, path: &Path) -> Option<Arc<[Symbol]>> {
        let doc = self.native_docs.get(path)?.buffer()?;
        let revision = doc.revision();
        if let Some((cached, at, outline)) = &self.structure.syntax
            && cached == path
            && *at == revision
        {
            return Some(Arc::clone(outline));
        }
        let language =
            terminator_native_edit::highlight::language_for_path(&path.to_string_lossy());
        let outline: Arc<[Symbol]> =
            terminator_native_edit::treesitter::outline(language, &doc.text())?.into();
        self.structure.syntax = Some((path.to_owned(), revision, Arc::clone(&outline)));
        Some(outline)
    }

    /// Outline for any file: Markdown headings, else the server's symbols
    /// (requested on demand; revision-gated, so cheap every frame), else
    /// the syntax tree's while the server has nothing.
    pub(crate) fn document_outline(&mut self, path: &Path) -> Arc<[Symbol]> {
        if let Some(outline) = self.markdown_outline(path) {
            return outline;
        }
        self.lsp.request_symbols(path);
        let symbols = self.lsp.symbols_for(path);
        if symbols.is_empty()
            && let Some(outline) = self.syntax_outline(path)
        {
            return outline;
        }
        symbols
    }

    pub(crate) fn structure_panel(&mut self, ui: &mut egui::Ui) {
        let restart = super::problems::lsp_panel_header(
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
        let symbols = if let Some(outline) = self.markdown_outline(&path) {
            outline
        } else {
            let language =
                terminator_native_edit::highlight::language_for_path(&path.to_string_lossy());
            let served = native_lsp::server_language(language).is_some();
            if served && restart {
                if self.lsp.tracked(&path) {
                    self.lsp.restart(&path);
                } else {
                    self.lsp.restart_all();
                }
            }
            // The server's outline when it has one; else the syntax tree's,
            // with a note on why the server is not answering yet.
            let note = if !served {
                None
            } else if !self.lsp.tracked(&path) {
                Some("Waiting for the native editor to load this file.".to_owned())
            } else if let Some(note) = self.lsp.server_note(&path) {
                Some(note)
            } else {
                self.lsp.request_symbols(&path);
                None
            };
            let served_symbols = if served && note.is_none() {
                self.lsp.symbols_for(&path)
            } else {
                Arc::from(Vec::new())
            };
            if !served_symbols.is_empty() {
                served_symbols
            } else if let Some(outline) = self.syntax_outline(&path) {
                ui.weak(match &note {
                    Some(note) => format!("Syntax outline · {note}"),
                    None if served => {
                        "Syntax outline while the language server indexes.".to_owned()
                    }
                    None => "Syntax outline (no language server for this file type).".to_owned(),
                });
                outline
            } else if !served {
                ui.weak("No document outline for this file type.");
                ui.weak(
                    "Structure supports Markdown headings, and Rust, Python, JavaScript and \
                     TypeScript through a language server.",
                );
                return;
            } else {
                if let Some(note) = note {
                    ui.weak(note);
                    return;
                }
                served_symbols
            }
        };
        let mut tree = OutlineTree::build(&symbols);
        self.structure_toolbar(ui, &path, &tree);
        if self.structure.alphabetical {
            tree.sort_alphabetically(&symbols);
        }
        let query = self.structure.filter.trim().to_owned();
        let filtering = !query.is_empty();
        // Keep matches and every ancestor of a match.
        let mut keep = vec![!filtering; symbols.len()];
        if filtering {
            for (index, symbol) in symbols.iter().enumerate().rev() {
                let hit = outline_matches(&symbol.name, &query)
                    || tree.children.get(index).is_some_and(|children| {
                        children.iter().any(|c| keep.get(*c) == Some(&true))
                    });
                if let Some(slot) = keep.get_mut(index) {
                    *slot = hit;
                }
            }
        }
        let collapsed = self
            .structure
            .collapsed
            .get(&path)
            .cloned()
            .unwrap_or_default();
        let expanded = |index: usize| {
            filtering
                || tree
                    .keys
                    .get(index)
                    .is_none_or(|key| !collapsed.contains(key))
        };
        // Depth-first visible rows.
        let mut rows: Vec<(usize, usize)> = Vec::new();
        let mut stack: Vec<(usize, usize)> = tree.roots.iter().rev().map(|r| (*r, 0)).collect();
        while let Some((index, depth)) = stack.pop() {
            if keep.get(index) != Some(&true) {
                continue;
            }
            rows.push((index, depth));
            if expanded(index)
                && let Some(children) = tree.children.get(index)
            {
                stack.extend(children.iter().rev().map(|c| (*c, depth.saturating_add(1))));
            }
        }
        if rows.is_empty() {
            ui.weak("No symbols match the filter.");
            return;
        }
        // Autoscroll from source: the caret's innermost symbol, or its
        // nearest visible ancestor when that one is collapsed away.
        let caret = (!self.structure.autoscroll_off)
            .then(|| self.native_docs.get(&path).and_then(|doc| doc.caret_line()))
            .flatten()
            .and_then(|line| u32::try_from(line).ok());
        let visible: HashSet<usize> = rows.iter().map(|(index, _)| *index).collect();
        let mut current = caret.and_then(|line| native_lsp::innermost_symbol(&symbols, line));
        while let Some(index) = current
            && !visible.contains(&index)
        {
            current = tree.parent.get(index).copied().flatten();
        }
        let current_key = current.and_then(|index| tree.keys.get(index).cloned());
        let scroll_to_current = current_key.is_some()
            && self.structure.followed.as_ref().map(|(p, k)| (p, Some(k)))
                != Some((&path, current_key.as_ref()));
        if scroll_to_current {
            self.structure.followed = current_key.clone().map(|key| (path.clone(), key));
        }
        let mut jump: Option<native_lsp::Position> = None;
        let mut toggle: Option<String> = None;
        appearance::sidebar_scroll("structure")
            .max_height(ui.available_height())
            .show(ui, |ui| {
                for (index, depth) in &rows {
                    let Some(symbol) = symbols.get(*index) else {
                        continue;
                    };
                    let parent = tree.has_children(*index);
                    let selected = current == Some(*index);
                    let response = outline_row(OutlineRowInput {
                        ui,
                        symbol,
                        depth: *depth,
                        parent,
                        expanded: expanded(*index),
                        selected,
                    });
                    if selected && scroll_to_current {
                        response.scroll_to_me(None);
                    }
                    if response.clicked() {
                        let chevron_end = response.rect.left() + *depth as f32 * INDENT + CHEVRON;
                        let on_chevron = response
                            .interact_pointer_pos()
                            .is_some_and(|pos| pos.x < chevron_end);
                        if parent && on_chevron && !filtering {
                            toggle = tree.keys.get(*index).cloned();
                        } else {
                            jump = Some(symbol.selection.start);
                        }
                    }
                    if response.double_clicked() && parent && !filtering {
                        toggle = tree.keys.get(*index).cloned();
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

    /// Filter field plus sort, autoscroll, and collapse-all toggles.
    fn structure_toolbar(&mut self, ui: &mut egui::Ui, path: &Path, tree: &OutlineTree) {
        ui.horizontal(|ui| {
            let buttons = 3.0 * (ui.spacing().interact_size.y + ui.spacing().item_spacing.x);
            ui.add(
                appearance::singleline(&mut self.structure.filter)
                    .hint_text("Filter symbols…")
                    .desired_width((ui.available_width() - buttons).max(60.0)),
            );
            if appearance::selectable_icon(
                ui,
                "ArrowDownWideNarrow",
                "Sort alphabetically",
                self.structure.alphabetical,
            )
            .clicked()
            {
                self.structure.alphabetical = !self.structure.alphabetical;
            }
            if appearance::selectable_icon(
                ui,
                "LocateFixed",
                "Autoscroll from source",
                !self.structure.autoscroll_off,
            )
            .clicked()
            {
                self.structure.autoscroll_off = !self.structure.autoscroll_off;
                self.structure.followed = None;
            }
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
        });
    }
}

/// One outline row: chevron, kind badge, name (struck through when
/// deprecated), and the dimmed server detail.
pub(crate) struct OutlineRowInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) symbol: &'a Symbol,
    pub(crate) depth: usize,
    pub(crate) parent: bool,
    pub(crate) expanded: bool,
    pub(crate) selected: bool,
}

fn outline_row(input: OutlineRowInput<'_>) -> egui::Response {
    let OutlineRowInput {
        ui,
        symbol,
        depth,
        parent,
        expanded,
        selected,
    } = input;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), ROW_HEIGHT),
        egui::Sense::click(),
    );
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals();
    if selected {
        painter.rect_filled(rect, 3.0, visuals.selection.bg_fill);
    } else if response.hovered() {
        painter.rect_filled(rect, 3.0, visuals.widgets.hovered.weak_bg_fill);
    }
    let mut x = rect.left() + depth as f32 * INDENT;
    if parent {
        let chevron =
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(CHEVRON, ROW_HEIGHT));
        appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
            ui,
            rect: chevron,
            icon: if expanded {
                "ChevronDown"
            } else {
                "ChevronRight"
            },
            size: 12.0,
            tint: appearance::icon_color(),
        });
    }
    x += CHEVRON;
    let (letter, color) = kind_badge(&symbol.kind);
    let center = egui::pos2(x + BADGE / 2.0, rect.center().y);
    painter.circle_filled(center, BADGE / 2.0 - 1.0, color);
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        letter,
        egui::FontId::monospace(10.0),
        egui::Color32::WHITE,
    );
    x += BADGE + 6.0;
    let name_color = if symbol.deprecated {
        visuals.weak_text_color()
    } else {
        visuals.text_color()
    };
    let name = painter.layout_no_wrap(
        symbol.name.clone(),
        egui::FontId::proportional(13.0),
        name_color,
    );
    let name_pos = egui::pos2(x, rect.center().y - name.size().y / 2.0);
    let name_width = name.size().x;
    painter.galley(name_pos, name, name_color);
    if symbol.deprecated {
        painter.hline(
            x..=x + name_width,
            rect.center().y,
            egui::Stroke::new(1.0, name_color),
        );
    }
    if let Some(detail) = &symbol.detail {
        painter.text(
            egui::pos2(x + name_width + 8.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            detail.lines().next().unwrap_or_default(),
            egui::FontId::proportional(11.0),
            visuals.weak_text_color(),
        );
    }
    let mut tip = format!(
        "{} — line {}",
        symbol.kind,
        symbol.selection.start.line.saturating_add(1)
    );
    if let Some(detail) = &symbol.detail {
        tip.push('\n');
        tip.push_str(detail);
    }
    response.on_hover_text(tip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminator_native_edit::lsp::{Position, Range};

    fn symbol(name: &str, depth: usize, line: u32) -> Symbol {
        let at = Position { line, character: 0 };
        Symbol {
            name: name.into(),
            kind: "Function".into(),
            range: Range { start: at, end: at },
            selection: Range { start: at, end: at },
            depth,
            detail: None,
            deprecated: false,
        }
    }

    #[test]
    fn outline_tree_links_children_to_their_nearest_shallower_symbol() {
        let symbols = [
            symbol("Point", 0, 0),
            symbol("x", 1, 1),
            symbol("y", 1, 2),
            symbol("new", 0, 4),
            symbol("inner", 1, 5),
        ];
        let tree = OutlineTree::build(&symbols);
        assert_eq!(tree.roots, [0, 3]);
        assert_eq!(tree.children.first(), Some(&vec![1, 2]));
        assert_eq!(tree.parent.get(4), Some(&Some(3)));
        assert_eq!(tree.keys.get(2).map(String::as_str), Some("Point\u{1f}y"));
    }

    #[test]
    fn alphabetical_sort_orders_siblings_only() {
        let symbols = [
            symbol("zeta", 0, 0),
            symbol("b", 1, 1),
            symbol("a", 1, 2),
            symbol("alpha", 0, 3),
        ];
        let mut tree = OutlineTree::build(&symbols);
        tree.sort_alphabetically(&symbols);
        assert_eq!(tree.roots, [3, 0]);
        assert_eq!(tree.children.first(), Some(&vec![2, 1]));
    }

    #[test]
    fn speed_search_is_a_case_insensitive_subsequence() {
        assert!(outline_matches("handle_request", "hreq"));
        assert!(outline_matches("HttpServer", "server"));
        assert!(!outline_matches("parse", "pz"));
    }

    #[test]
    fn outline_falls_back_to_the_syntax_tree_without_a_server() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, terminator_core::Paths::at(dir.path().into()));
        let path = dir.path().join("lib.rs");
        app.native_docs.insert(
            path.clone(),
            crate::native_editor::NativeDoc::loaded(path.clone(), "struct A;\nfn b() {}\n"),
        );
        let outline = app.document_outline(&path);
        assert_eq!(
            outline
                .iter()
                .map(|symbol| symbol.name.as_str())
                .collect::<Vec<_>>(),
            ["A", "b"]
        );
    }
}
