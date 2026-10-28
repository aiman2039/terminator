//! Structure tool window: the active editor's outline as a collapsible
//! tree (`IntelliJ` Structure). Language servers supply code outlines;
//! Markdown headings come from the buffer itself. The caret's enclosing
//! symbol is selected and scrolled into view as it moves.

mod panel;

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
    paint_kind_badge(
        &painter,
        &symbol.kind,
        egui::pos2(x + BADGE / 2.0, rect.center().y),
    );
    x += BADGE + 6.0;
    paint_symbol_text(PaintSymbolTextInput {
        ui,
        painter: &painter,
        symbol,
        start: egui::pos2(x, rect.center().y),
    });
    response.on_hover_text(outline_tip(symbol))
}

/// The kind letter on a colored disc centered at `center`.
fn paint_kind_badge(painter: &egui::Painter, kind: &str, center: egui::Pos2) {
    let (letter, color) = kind_badge(kind);
    painter.circle_filled(center, BADGE / 2.0 - 1.0, color);
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        letter,
        egui::FontId::monospace(10.0),
        egui::Color32::WHITE,
    );
}

struct PaintSymbolTextInput<'a> {
    ui: &'a egui::Ui,
    painter: &'a egui::Painter,
    symbol: &'a Symbol,
    start: egui::Pos2,
}

/// Name (struck through when deprecated) and the first detail line,
/// starting at `start` (left edge, vertical center).
fn paint_symbol_text(input: PaintSymbolTextInput<'_>) {
    let PaintSymbolTextInput {
        ui,
        painter,
        symbol,
        start,
    } = input;
    let visuals = ui.visuals();
    let x = start.x;
    let center_y = start.y;
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
    let name_pos = egui::pos2(x, center_y - name.size().y / 2.0);
    let name_width = name.size().x;
    painter.galley(name_pos, name, name_color);
    if symbol.deprecated {
        painter.hline(
            x..=x + name_width,
            center_y,
            egui::Stroke::new(1.0, name_color),
        );
    }
    if let Some(detail) = &symbol.detail {
        painter.text(
            egui::pos2(x + name_width + 8.0, center_y),
            egui::Align2::LEFT_CENTER,
            detail.lines().next().unwrap_or_default(),
            egui::FontId::proportional(11.0),
            visuals.weak_text_color(),
        );
    }
}

fn outline_tip(symbol: &Symbol) -> String {
    let mut tip = format!(
        "{} — line {}",
        symbol.kind,
        symbol.selection.start.line.saturating_add(1)
    );
    if let Some(detail) = &symbol.detail {
        tip.push('\n');
        tip.push_str(detail);
    }
    tip
}

#[cfg(test)]
mod tests;
