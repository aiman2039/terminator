//! The flow editor's keys: Save (the Save action's key, Cmd+S), Remove
//! selected cards, Duplicate and Fit to view. They are keymap actions, so
//! menus show their keys. They apply only while a flow editor is the
//! focused pane and no text field, sidebar or terminal has the keyboard;
//! the shortcut dispatch consumes them, so they never reach a PTY. Esc in
//! an inspector field gives the keyboard back to the canvas (egui).
use super::{Slot, canvas};
use crate::document_view::DocumentKind;
use crate::*;
use std::path::{Path, PathBuf};

/// The keymap actions of the flow editor.
pub(crate) const FLOW_ACTIONS: &[Action] =
    &[Action::FlowRemove, Action::FlowDuplicate, Action::FlowFit];

impl App {
    /// The flow file of the focused pane, when it is a flow editor tab.
    fn focused_flow_pane(&self) -> Option<&Path> {
        let pane = self.layouts.get(self.selected.as_ref()?)?.active_pane()?;
        match pane {
            Tab::Document {
                path,
                kind: DocumentKind::Flow,
            } => Some(path),
            _ => None,
        }
    }

    /// True when `path` is open in any floating window.
    fn is_floated_flow(&self, path: &Path) -> bool {
        self.floating_tabs().any(|(_, tab)| {
            matches!(
                tab,
                Tab::Document {
                    path: at,
                    kind: DocumentKind::Flow,
                } if at == path
            )
        })
    }

    /// The flow file of the focused pane, when it is an open flow editor.
    pub(crate) fn active_flow_editor(&self) -> Option<PathBuf> {
        let path = self.focused_flow_pane()?;
        matches!(self.flows.editors.get(path), Some(Slot::Ready(_))).then(|| path.to_path_buf())
    }

    /// No text field, sidebar row or terminal has the keyboard.
    fn canvas_keyboard(&self, ctx: &egui::Context) -> bool {
        !self.text_input_focused(ctx)
            && shortcuts::focused_sidebar(ctx).is_none()
            && !shortcuts::terminal_has_focus(ctx)
    }

    /// A flow editor is the focused pane and its canvas has the keyboard.
    pub(crate) fn flow_editor_has_keyboard(&self, ctx: &egui::Context) -> bool {
        self.active_flow_editor().is_some() && self.canvas_keyboard(ctx)
    }

    /// Runs a flow editor key on the focused flow editor. False when no
    /// flow editor has the focus (Save then saves the focused editor).
    pub(crate) fn run_flow_editor_key(&mut self, action: Action) -> bool {
        let Some(path) = self.active_flow_editor() else {
            return false;
        };
        let Some(Slot::Ready(editor)) = self.flows.editors.get_mut(&path) else {
            return false;
        };
        match action {
            Action::EditorSave if editor.dirty() => editor.save(),
            Action::FlowRemove => editor.remove_selected(),
            Action::FlowDuplicate => {
                if let Some(node) = editor.selected {
                    editor.selected = canvas::duplicate(&mut editor.snarl, node).or(Some(node));
                }
            }
            Action::FlowFit => editor.fit = true,
            _ => {}
        }
        true
    }

    /// Runs undo/redo, delete alias, and (in floating windows) all flow
    /// editor shortcuts directly on `ctx`.
    pub(super) fn flow_pane_keys(
        &self,
        ctx: &egui::Context,
        (path, editor): (&Path, &mut super::FlowEditor),
    ) {
        let in_main = self.focused_flow_pane() == Some(path);
        let floated = !in_main && self.is_floated_flow(path);
        if !in_main && !floated {
            return;
        }
        if floated {
            let save =
                shortcuts::binding(&self.state.settings.keybindings, Action::EditorSave.id());
            if !save.is_empty() && shortcuts::consume(ctx, &save) && editor.dirty() {
                editor.save();
            }
        }
        if !self.canvas_keyboard(ctx) {
            return;
        }
        if floated {
            for &action in FLOW_ACTIONS {
                let key = shortcuts::binding(&self.state.settings.keybindings, action.id());
                if !key.is_empty() && shortcuts::consume(ctx, &key) {
                    apply_editor_key(editor, action);
                }
            }
        }
        self.flow_undo_redo_keys(ctx, editor);
        self.flow_delete_alias(ctx, editor);
    }

    fn flow_undo_redo_keys(&self, ctx: &egui::Context, editor: &mut super::FlowEditor) {
        let redo_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        let redo = ctx.input_mut(|input| {
            input.consume_key(redo_shift, egui::Key::Z)
                || (!cfg!(target_os = "macos")
                    && input.consume_key(egui::Modifiers::COMMAND, egui::Key::Y))
        });
        if redo {
            editor.redo();
            return;
        }
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
            editor.undo();
        }
    }

    /// The other delete key (Backspace or Delete) also removes the
    /// selected cards, while the canvas has the keyboard.
    fn flow_delete_alias(&self, ctx: &egui::Context, editor: &mut super::FlowEditor) {
        let bound = shortcuts::binding(&self.state.settings.keybindings, Action::FlowRemove.id());
        let other = match shortcuts::parse(&bound).map(|chord| (chord.key, chord.modifiers)) {
            Some((egui::Key::Backspace, egui::Modifiers::NONE)) => egui::Key::Delete,
            Some((egui::Key::Delete, egui::Modifiers::NONE)) => egui::Key::Backspace,
            _ => return,
        };
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, other)) {
            editor.remove_selected();
        }
    }
}

fn apply_editor_key(editor: &mut super::FlowEditor, action: Action) {
    match action {
        Action::EditorSave if editor.dirty() => editor.save(),
        Action::FlowRemove => editor.remove_selected(),
        Action::FlowDuplicate => {
            if let Some(node) = editor.selected {
                editor.selected = canvas::duplicate(&mut editor.snarl, node).or(Some(node));
            }
        }
        Action::FlowFit => editor.fit = true,
        _ => {}
    }
}

impl super::FlowEditor {
    /// Removes the selected card and the cards that egui-snarl selected.
    pub(super) fn remove_selected(&mut self) {
        let nodes: Vec<_> = self.marked.drain(..).chain(self.selected.take()).collect();
        for node in nodes {
            if self.snarl.get_node(node).is_some() {
                self.snarl.remove_node(node);
            }
        }
    }
}
