//! Deferred goto jumps and `:split` / `:vsplit` / `:tabnew` layout changes.
use super::*;

impl App {
    /// Open one finished goto jump: same file places the cursor, other
    /// files open at the target line and column through the pending
    /// machinery (`open_native` carries the line, `native_pending_col`
    /// the UTF-16 column). Runs after the workspace is checked back in, when
    /// the originating project resolves again.
    pub(super) fn apply_goto_jump(&mut self, jump: lsp_manager::GotoJump) {
        // Several declarations: let the user pick (`IntelliJ`).
        if jump.targets.len() > 1 {
            self.show_usages(navigation::DECLARATION_TITLE, jump.targets);
            return;
        }
        let Some(target) = jump.targets.first() else {
            return;
        };
        self.record_jump_origin();
        let Some(target_path) = native_lsp::uri_to_path(&target.uri) else {
            return;
        };
        let line = usize::try_from(target.range.start.line).unwrap_or(usize::MAX);
        if target_path == jump.from
            && let Some(doc) = self.native_docs.get_mut(&target_path)
            && let Some(buffer) = doc.doc.as_mut()
        {
            // The live buffer converts unsaved edits exactly.
            let col = native_lsp::utf16_to_char_col(
                &buffer.line_text(line),
                target.range.start.character,
            );
            doc.engine.place_cursor(buffer, line, col);
            return;
        }
        if target_path == jump.from {
            return;
        }
        let Some(project) = self.project_with_native_tab(&jump.from) else {
            return;
        };
        self.open_native(OpenNativeInput {
            project: &project,
            path: target_path.clone(),
            line: target.range.start.line.checked_add(1),
            split: None,
        });
        // The column converts once the target buffer has loaded.
        let absolute = std::path::absolute(&target_path).unwrap_or(target_path);
        self.native_pending_col
            .insert(absolute, target.range.start.character);
    }

    /// Project owning a live native view of `path`, for deferred splits:
    /// the originating project is preserved, never the selected one.
    /// Floating windows count too: a jump issued from a floated editor
    /// opens in its home project.
    pub(super) fn project_with_native_tab(&self, path: &Path) -> Option<String> {
        self.layouts
            .iter()
            .find_map(|(project, workspace)| {
                workspace
                    .tabs
                    .iter()
                    .flat_map(|tab| tab.layout.iter_all_tabs())
                    .any(|(_, tab)| {
                        matches!(tab, Tab::NativeEditor { path: current } if current == path)
                    })
                    .then(|| project.clone())
            })
            .or_else(|| {
                self.floating.iter().find_map(|window| {
                    window
                        .dock
                        .as_ref()
                        .is_some_and(|dock| {
                            dock.iter_all_tabs().any(|(_, tab)| {
                                matches!(tab, Tab::NativeEditor { path: current } if current == path)
                            })
                        })
                        .then(|| window.home.0.clone())
                })
            })
    }

    /// Carry out a deferred `:split` / `:vsplit` / `:tabnew`: the same
    /// file opens beside, below, or in a new top-level tab of its own
    /// project — or beside/below inside the issuing floating window.
    /// Both views share the one buffer and engine for now (one cursor
    /// across panes); per-view cursors are future work.
    pub(super) fn split_native_view(
        &mut self,
        viewport: egui::ViewportId,
        path: &Path,
        split: NativeSplit,
    ) {
        let tab = Tab::NativeEditor {
            path: path.to_owned(),
        };
        if split == NativeSplit::NewTab {
            let Some(project) = self.project_with_native_tab(path) else {
                return;
            };
            self.place_gui_tab(
                project,
                tab,
                After::Workspace(terminator_core::id(), Vec::new()),
            );
            return;
        }
        // A split issued from a floating window stays in that window.
        if viewport != egui::ViewportId::ROOT
            && let Some(window) = self
                .floating
                .iter_mut()
                .find(|window| window.viewport == viewport)
            && let Some(dock) = window.dock.as_mut()
        {
            Self::split_dock(dock, tab, split);
            return;
        }
        let Some(project) = self.project_with_native_tab(path) else {
            return;
        };
        let Some(dock) = self.layouts.get_mut(&project) else {
            return;
        };
        Self::split_dock(dock, tab, split);
    }

    /// Add one more view of `tab` beside or below the focused leaf. The
    /// issuing copy is focused first when present, so repeated splits fan
    /// out from the pane the user is typing in; a dock that no longer
    /// holds it (closed or reloaded between frames) still splits at its
    /// focused leaf instead of silently dropping the request.
    pub(super) fn split_dock(dock: &mut egui_dock::DockState<Tab>, tab: Tab, split: NativeSplit) {
        if let Some(found) = dock.find_tab(&tab) {
            dock.set_focused_node_and_surface(found.node_path());
        }
        let tree = dock.main_surface_mut();
        if tree.is_empty() {
            tree.push_to_focused_leaf(tab);
            return;
        }
        let node = tree.focused_leaf().unwrap_or(NodeIndex::root());
        let placed = match split {
            NativeSplit::Below => tree.split_below(node, 0.5, vec![tab]),
            _ => tree.split_right(node, 0.5, vec![tab]),
        };
        tree.set_focused_node(placed[1]);
    }
}
