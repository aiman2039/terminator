//! Quitting with flow editors whose content is not saved. Those editors
//! live only in GUI memory, so the quit asks once: Save and quit, Discard
//! and quit, or Cancel. Save goes through `FlowEditor::save` (checked, on a
//! worker thread); a flow that cannot be saved stops the quit and shows
//! its editor. A moved card alone is display only and does not ask.
use super::Slot;
use crate::*;
use std::path::PathBuf;

/// Where the current quit is with the flow editors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Quit {
    /// Not asked yet.
    #[default]
    Idle,
    /// The question shows.
    Asking,
    /// Save and quit was picked: the saves run.
    Saving,
    /// Saved or discarded: the quit continues.
    Done,
}

/// A flow that the quit could not save: its file path, name and error.
type Failed = (PathBuf, String, Option<String>);

impl App {
    /// The names of the flows whose content is not saved, sorted.
    pub(crate) fn unsaved_flows(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .flows
            .editors
            .values()
            .filter_map(|slot| match slot {
                Slot::Ready(editor) if editor.changed() => Some(editor.file.clone()),
                _ => None,
            })
            .collect();
        names.sort_unstable();
        names
    }

    /// True while the quit waits on flow editors: for the answer, or for
    /// the saves of Save and quit.
    pub(crate) fn flow_quit_waits(&mut self, ctx: &egui::Context) -> bool {
        // A native fixture closes its own window and cannot answer.
        #[cfg(feature = "test-support")]
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
            return false;
        }
        match self.flows.quit {
            Quit::Done => false,
            Quit::Asking => true,
            Quit::Saving => {
                let waits = self.flow_quit_saving();
                if waits {
                    ctx.request_repaint_after(Duration::from_millis(20));
                }
                waits
            }
            Quit::Idle if self.unsaved_flows().is_empty() => false,
            Quit::Idle => {
                self.flows.quit = Quit::Asking;
                ctx.request_repaint();
                true
            }
        }
    }

    /// The saves of Save and quit. When they end, a flow that is still not
    /// saved stops the quit and shows its editor with the error.
    fn flow_quit_saving(&mut self) -> bool {
        let mut busy = false;
        let mut note = None;
        for slot in self.flows.editors.values_mut() {
            if let Slot::Ready(editor) = slot {
                note = editor.poll().or(note);
                busy |= editor.saving.is_some();
            }
        }
        if note.is_some() {
            self.info = note;
        }
        if busy {
            return true;
        }
        let Some((path, file, error)) = self.unsaved_flow_editor() else {
            self.flows.quit = Quit::Done;
            return false;
        };
        self.cancel_app_quit();
        self.show_flow_editor(&path);
        let error = error.map(|error| format!(": {error}")).unwrap_or_default();
        self.info = Some(format!(
            "Could not save the flow {file}{error}. Terminator did not quit."
        ));
        true
    }

    fn unsaved_flow_editor(&self) -> Option<Failed> {
        self.flows
            .editors
            .iter()
            .find_map(|(path, slot)| match slot {
                Slot::Ready(editor) if editor.changed() => {
                    Some((path.clone(), editor.file.clone(), editor.error.clone()))
                }
                _ => None,
            })
    }

    /// Opens (or focuses) the editor tab of the flow file `path`.
    fn show_flow_editor(&mut self, path: &std::path::Path) {
        let Some(Slot::Ready(editor)) = self.flows.editors.get(path) else {
            return;
        };
        let (root, file) = (editor.root.clone(), editor.file.clone());
        let project = self
            .state
            .projects
            .iter()
            .find(|project| project.path == root)
            .map(|project| project.id.clone());
        if let Some(project) = project {
            self.open_flow_tab(&project, &root, &file);
        }
    }

    /// The in-app question shown while a quit waits on flows whose
    /// content is not saved.
    pub(crate) fn flow_quit_dialog(&mut self, ctx: &egui::Context) {
        if self.flows.quit != Quit::Asking {
            return;
        }
        let names = self.unsaved_flows();
        if names.is_empty() {
            // The user saved or discarded them meanwhile.
            self.flows.quit = Quit::Done;
            return;
        }
        let (noun, verb) = if names.len() == 1 {
            ("The flow", "has")
        } else {
            ("The flows", "have")
        };
        let window = egui::Window::new("Quit Terminator")
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO);
        let dialog = appearance::Confirm::new(
            format!(
                "{noun} {} {verb} changes that are not saved.",
                names.join(", ")
            ),
            "Save checks each flow first. A flow with an error stops the quit.",
        )
        .destructive(appearance::ConfirmButton::new("Discard and quit"))
        .default_action(appearance::ConfirmButton::new("Save and quit"));
        if let Some(answer) = appearance::confirm(ctx, window, dialog, |_| {}) {
            self.answer_flow_quit(ctx, answer);
        }
    }

    pub(crate) fn answer_flow_quit(
        &mut self,
        ctx: &egui::Context,
        answer: appearance::ConfirmAnswer,
    ) {
        match answer {
            appearance::ConfirmAnswer::Cancel => {
                self.cancel_app_quit();
                return;
            }
            appearance::ConfirmAnswer::Destructive => self.flows.quit = Quit::Done,
            appearance::ConfirmAnswer::Default => {
                for slot in self.flows.editors.values_mut() {
                    if let Slot::Ready(editor) = slot
                        && editor.changed()
                    {
                        editor.save();
                    }
                }
                self.flows.quit = Quit::Saving;
            }
        }
        self.continue_app_quit(ctx);
    }

    /// Ends the flow part of the current quit: the next quit asks again.
    pub(crate) fn reset_flow_quit(&mut self) {
        self.flows.quit = Quit::Idle;
    }

    /// Puts a new flow `file` of the project folder `root` in GUI memory,
    /// as an editor whose tab was closed. A wrong flow has one ID twice.
    #[cfg(test)]
    pub(crate) fn unsaved_flow_for_test(
        &mut self,
        root: &std::path::Path,
        file: &str,
        correct: bool,
    ) {
        use terminator_core::flow::files;
        let mut editor = super::FlowEditor::new(root.into(), file);
        if !correct {
            editor.snarl.insert_node(
                egui::Pos2::ZERO,
                super::model::Node::step("plan", "claude", ""),
            );
        }
        let path = files::path_of(root, file).expect("a flow name");
        self.flows
            .editors
            .insert(path, Slot::Ready(Box::new(editor)));
    }
}
