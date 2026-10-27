//! Call and type hierarchy (`IntelliJ` Ctrl+Alt+H / Type Hierarchy): a
//! tree rooted at the symbol under the caret, expanded lazily through
//! the language server, toggled between callers and callees (or super-
//! and subtypes). Clicking an item jumps to it.

use super::*;
use crate::lsp_manager::HierarchyAnswer;
use native_lsp::{HierarchyDirection, HierarchyItem, HierarchyKind};

/// One tree node; `children` is `None` until asked for.
struct Node {
    item: HierarchyItem,
    depth: usize,
    children: Option<Vec<usize>>,
    open: bool,
    loading: bool,
}

/// The hierarchy window.
pub(crate) struct HierarchyView {
    /// The file the hierarchy started from: its server answers.
    path: PathBuf,
    kind: HierarchyKind,
    direction: HierarchyDirection,
    /// Request generation; answers for older ones are dropped.
    generation: u64,
    /// `nodes[0]` is the root once the server answered.
    nodes: Vec<Node>,
    /// Shown instead of the tree ("Loading…", "Nothing here").
    status: Option<String>,
}

impl HierarchyView {
    fn title(&self) -> &'static str {
        match self.kind {
            HierarchyKind::Call => "Call Hierarchy",
            HierarchyKind::Type => "Type Hierarchy",
        }
    }

    /// Labels of the two directions, `(up, down)`.
    fn direction_labels(&self) -> (&'static str, &'static str) {
        match self.kind {
            HierarchyKind::Call => ("Callers", "Callees"),
            HierarchyKind::Type => ("Supertypes", "Subtypes"),
        }
    }

    /// Visible nodes in tree order (open nodes show their children).
    fn visible(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![0usize];
        while let Some(index) = stack.pop() {
            let Some(node) = self.nodes.get(index) else {
                continue;
            };
            out.push(index);
            if node.open
                && let Some(children) = &node.children
            {
                stack.extend(children.iter().rev());
            }
        }
        out
    }

    /// Apply one answer; returns the node whose children should be asked
    /// for next (a fresh root opens itself).
    fn apply(&mut self, answer: HierarchyAnswer) -> Option<usize> {
        match answer {
            HierarchyAnswer::Roots { generation, items } if generation == self.generation => {
                let Some(root) = items.into_iter().next() else {
                    self.status = Some("No hierarchy for the symbol at the caret".into());
                    return None;
                };
                self.status = None;
                self.nodes = vec![Node {
                    item: root,
                    depth: 0,
                    children: None,
                    open: true,
                    loading: false,
                }];
                Some(0)
            }
            HierarchyAnswer::Children {
                generation,
                node,
                items,
            } if generation == self.generation => {
                let depth = self.nodes.get(node)?.depth.saturating_add(1);
                let first = self.nodes.len();
                self.nodes.extend(items.into_iter().map(|item| Node {
                    item,
                    depth,
                    children: None,
                    open: false,
                    loading: false,
                }));
                let children = (first..self.nodes.len()).collect();
                let parent = self.nodes.get_mut(node)?;
                parent.children = Some(children);
                parent.loading = false;
                None
            }
            _ => None,
        }
    }
}

impl App {
    /// Start a hierarchy at the caret's UTF-16 position in `path`.
    pub(crate) fn start_hierarchy(
        &mut self,
        path: &Path,
        position: native_lsp::Position,
        kind: HierarchyKind,
    ) {
        match self.lsp.request_hierarchy(path, position, kind) {
            Ok(generation) => {
                self.hierarchy = Some(HierarchyView {
                    path: path.to_owned(),
                    kind,
                    direction: HierarchyDirection::Up,
                    generation,
                    nodes: Vec::new(),
                    status: Some("Loading…".into()),
                });
            }
            Err(error) => self.info = Some(error),
        }
    }

    /// Ask for `node`'s children in the current direction.
    fn expand_hierarchy_node(&mut self, node: usize) {
        let Some(view) = self.hierarchy.as_mut() else {
            return;
        };
        let Some(entry) = view.nodes.get_mut(node) else {
            return;
        };
        entry.open = true;
        // An empty answer may be a server still indexing: ask again.
        if entry.loading
            || entry
                .children
                .as_ref()
                .is_some_and(|children| !children.is_empty())
        {
            return;
        }
        entry.children = None;
        let raw = entry.item.raw.clone();
        entry.loading = self.lsp.request_hierarchy_children(
            &view.path,
            &raw,
            view.kind,
            view.direction,
            view.generation,
            node,
        );
        if !entry.loading {
            entry.children = Some(Vec::new());
        }
    }

    /// Switch callers ↔ callees (or super ↔ sub types), or refresh with
    /// the same direction: keep the root, drop everything below it, and
    /// ask again (an indexing server first answers with nothing).
    fn set_hierarchy_direction(&mut self, direction: HierarchyDirection) {
        let Some(view) = self.hierarchy.as_mut() else {
            return;
        };
        view.direction = direction;
        view.generation = self.lsp.next_hierarchy_generation();
        view.nodes.truncate(1);
        if let Some(root) = view.nodes.first_mut() {
            root.children = None;
            root.loading = false;
        }
        self.expand_hierarchy_node(0);
    }

    /// Drain answers, then paint the window.
    pub(crate) fn show_hierarchy_window(&mut self, ctx: &egui::Context) {
        let answers = std::mem::take(&mut self.lsp.hierarchy_answers);
        for answer in answers {
            if let Some(next) = self.hierarchy.as_mut().and_then(|view| view.apply(answer)) {
                self.expand_hierarchy_node(next);
            }
        }
        let Some(view) = self.hierarchy.as_ref() else {
            return;
        };
        let root = self.selected_project().map(|project| project.path.clone());
        let mut open = true;
        let mut toggle: Option<usize> = None;
        let mut jump: Option<NavLocation> = None;
        let mut direction: Option<HierarchyDirection> = None;
        let (up, down) = view.direction_labels();
        self.popups
            .window(ctx, view.title())
            .open(&mut open)
            .collapsible(false)
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (label, value) in [
                        (up, HierarchyDirection::Up),
                        (down, HierarchyDirection::Down),
                    ] {
                        if ui
                            .selectable_label(view.direction == value, label)
                            .clicked()
                            && view.direction != value
                        {
                            direction = Some(value);
                        }
                    }
                    if !view.nodes.is_empty()
                        && ui
                            .button("Refresh")
                            .on_hover_text("Ask the language server again")
                            .clicked()
                    {
                        direction = Some(view.direction);
                    }
                });
                ui.separator();
                if let Some(status) = &view.status {
                    ui.weak(status);
                    return;
                }
                egui::ScrollArea::vertical()
                    .max_height(420.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for index in view.visible() {
                            let Some(node) = view.nodes.get(index) else {
                                continue;
                            };
                            ui.horizontal(|ui| {
                                ui.add_space(node.depth as f32 * 16.0);
                                let leaf = node.children.as_ref().is_some_and(Vec::is_empty);
                                let arrow = if leaf {
                                    " "
                                } else if node.open {
                                    "▾"
                                } else {
                                    "▸"
                                };
                                let arrow = ui.add(egui::Button::new(arrow).frame(false));
                                let arrow = if leaf {
                                    arrow.on_hover_text("Nothing found; click to ask again")
                                } else {
                                    arrow
                                };
                                if arrow.clicked() {
                                    toggle = Some(index);
                                }
                                if node.loading {
                                    ui.spinner();
                                }
                                let file = native_lsp::uri_to_path(&node.item.uri);
                                let place = file.as_ref().map(|file| {
                                    let shown = root
                                        .as_deref()
                                        .and_then(|root| file.strip_prefix(root).ok())
                                        .unwrap_or(file);
                                    format!(
                                        "{}:{}",
                                        shown.display(),
                                        node.item.selection.start.line.saturating_add(1)
                                    )
                                });
                                let mut text = egui::text::LayoutJob::default();
                                text.append(
                                    &node.item.name,
                                    0.0,
                                    egui::TextFormat::simple(
                                        egui::FontId::proportional(13.0),
                                        ui.visuals().strong_text_color(),
                                    ),
                                );
                                let weak = egui::TextFormat::simple(
                                    egui::FontId::proportional(12.0),
                                    ui.visuals().weak_text_color(),
                                );
                                if let Some(detail) = &node.item.detail {
                                    text.append(detail, 8.0, weak.clone());
                                }
                                if let Some(place) = &place {
                                    text.append(place, 8.0, weak);
                                }
                                let hit = ui
                                    .add(egui::Button::selectable(false, text).truncate())
                                    .on_hover_text(&node.item.kind);
                                if hit.clicked()
                                    && let Some(path) = file
                                {
                                    jump = Some(NavLocation {
                                        path,
                                        line: node.item.selection.start.line,
                                        character: node.item.selection.start.character,
                                    });
                                }
                            });
                        }
                    });
            });
        if let Some(direction) = direction {
            self.set_hierarchy_direction(direction);
        }
        if let Some(index) = toggle {
            let opened = self
                .hierarchy
                .as_ref()
                .and_then(|view| view.nodes.get(index))
                .is_some_and(|node| node.open);
            if opened {
                if let Some(node) = self
                    .hierarchy
                    .as_mut()
                    .and_then(|view| view.nodes.get_mut(index))
                {
                    node.open = false;
                }
            } else {
                self.expand_hierarchy_node(index);
            }
        }
        if let Some(target) = jump {
            self.record_jump_origin();
            self.open_location(&target);
        }
        if !open {
            self.hierarchy = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(name: &str) -> HierarchyItem {
        let at = native_lsp::Position {
            line: 1,
            character: 0,
        };
        HierarchyItem {
            name: name.into(),
            kind: "Function".into(),
            detail: None,
            uri: format!("file:///tmp/{name}.rs"),
            selection: native_lsp::Range { start: at, end: at },
            raw: serde_json::json!({ "name": name }),
        }
    }

    fn view() -> HierarchyView {
        HierarchyView {
            path: PathBuf::from("/tmp/a.rs"),
            kind: HierarchyKind::Call,
            direction: HierarchyDirection::Up,
            generation: 3,
            nodes: Vec::new(),
            status: Some("Loading…".into()),
        }
    }

    #[test]
    fn answers_build_a_lazy_tree_and_stale_ones_are_dropped() {
        let mut view = view();
        // A stale root is ignored.
        assert_eq!(
            view.apply(HierarchyAnswer::Roots {
                generation: 2,
                items: vec![item("old")],
            }),
            None
        );
        assert!(view.nodes.is_empty());
        // The root opens itself: its children are asked for next.
        assert_eq!(
            view.apply(HierarchyAnswer::Roots {
                generation: 3,
                items: vec![item("main")],
            }),
            Some(0)
        );
        view.apply(HierarchyAnswer::Children {
            generation: 3,
            node: 0,
            items: vec![item("a"), item("b")],
        });
        assert_eq!(view.visible(), [0, 1, 2]);
        assert_eq!(view.nodes[1].depth, 1);
        // Closed nodes hide their children; opened ones show them in order.
        view.apply(HierarchyAnswer::Children {
            generation: 3,
            node: 1,
            items: vec![item("c")],
        });
        assert_eq!(view.visible(), [0, 1, 2]);
        view.nodes[1].open = true;
        assert_eq!(view.visible(), [0, 1, 3, 2]);
        // No root at all reads as a status line.
        let mut empty = super::tests::view();
        empty.apply(HierarchyAnswer::Roots {
            generation: 3,
            items: Vec::new(),
        });
        assert!(
            empty
                .status
                .is_some_and(|status| status.starts_with("No hierarchy"))
        );
    }
}
