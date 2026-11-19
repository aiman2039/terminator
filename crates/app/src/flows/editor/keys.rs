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

    /// The other delete key (Backspace or Delete) also removes the
    /// selected cards, while the canvas has the keyboard.
    pub(super) fn flow_delete_alias(
        &self,
        ctx: &egui::Context,
        (path, editor): (&Path, &mut super::FlowEditor),
    ) {
        let bound = shortcuts::binding(&self.state.settings.keybindings, Action::FlowRemove.id());
        let other = match shortcuts::parse(&bound).map(|chord| (chord.key, chord.modifiers)) {
            Some((egui::Key::Backspace, egui::Modifiers::NONE)) => egui::Key::Delete,
            Some((egui::Key::Delete, egui::Modifiers::NONE)) => egui::Key::Backspace,
            _ => return,
        };
        if self.focused_flow_pane() == Some(path)
            && self.canvas_keyboard(ctx)
            && ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, other))
        {
            editor.remove_selected();
        }
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
