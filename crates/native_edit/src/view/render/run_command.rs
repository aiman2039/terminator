//! A command the host queued for this view (the Find Action palette), run
//! the way its chord runs it in a focused view.
use super::SourceView;
use super::canvas::Canvas;
use super::helpers::RenderHelpers;
use crate::doc::Buffer;
use crate::view::command::take_command;
use crate::view::{EditorCommand, IdeAction, IdeEdit};
use crate::vim::{Effect, ModalEngine, Mode};

impl<B: Buffer> SourceView<'_, B> {
    /// Run the command queued for this view, once. The view takes keyboard
    /// focus, so typing goes on here after the palette closes.
    pub(super) fn run_queued_command(&mut self, canvas: &Canvas<'_>) {
        let ctx = canvas.ui.ctx();
        let Some(command) = take_command(ctx, self.id) else {
            return;
        };
        canvas.request_focus();
        ctx.request_repaint();
        let options = self.options;
        if command.available(self.language, options.vim, options.read_only) {
            self.run_command(canvas, command);
        }
    }

    fn run_command(&mut self, canvas: &Canvas<'_>, command: EditorCommand) {
        use EditorCommand as C;
        match command {
            C::DuplicateLine => self.command_edit(IdeEdit::Duplicate),
            C::DeleteLine => self.command_edit(IdeEdit::DeleteLines),
            C::MoveLineUp => self.command_edit(IdeEdit::MoveLines { up: true }),
            C::MoveLineDown => self.command_edit(IdeEdit::MoveLines { up: false }),
            C::CommentLine => self.command_edit(IdeEdit::ToggleComment),
            C::JoinLines => self.command_edit(IdeEdit::JoinLines),
            C::ExtendSelection => self.command_edit(IdeEdit::ExtendSelection),
            C::ShrinkSelection => self.command_edit(IdeEdit::ShrinkSelection),
            C::Fold => self.command_edit(IdeEdit::Fold { all: false }),
            C::Unfold => self.command_edit(IdeEdit::Unfold { all: false }),
            C::FoldAll => self.command_edit(IdeEdit::Fold { all: true }),
            C::UnfoldAll => self.command_edit(IdeEdit::Unfold { all: true }),
            C::SelectAllOccurrences => self.engine.add_all_occurrences(self.doc),
            C::AddNextOccurrence => {
                self.engine.add_next_occurrence(self.doc);
            }
            C::UnselectOccurrence => self.engine.remove_last_occurrence(),
            C::GotoLine => self.engine.begin_ex(""),
            C::Find => self.run_ide_action(canvas, IdeAction::Find),
            C::Replace => self.run_ide_action(canvas, IdeAction::Replace),
            C::Undo => self.run_ide_action(canvas, IdeAction::Undo),
            C::Redo => self.run_ide_action(canvas, IdeAction::Redo),
            C::QuickDocumentation => self.run_ide_action(canvas, IdeAction::Hover),
            C::NextProblem => self.jump_to_problem(false),
            C::PreviousProblem => self.jump_to_problem(true),
            C::CopyReference => self.copy_reference(canvas),
            C::ContextActions => self.out.effects.push(Effect::CodeActions),
            C::Reformat => self.out.effects.push(Effect::Format),
            C::GotoDeclaration => self.out.effects.push(Effect::GotoDefinition),
            C::GotoImplementation => self.out.effects.push(Effect::GotoImplementation),
            C::GotoTypeDeclaration => self.out.effects.push(Effect::GotoTypeDefinition),
            C::FindUsages => self.out.effects.push(Effect::FindReferences),
            C::Rename => self.out.effects.push(Effect::RenameSymbol),
            C::CallHierarchy => self.out.effects.push(Effect::Hierarchy { types: false }),
            C::TypeHierarchy => self.out.effects.push(Effect::Hierarchy { types: true }),
            C::Completion => self.out.effects.push(Effect::Complete),
        }
    }

    /// One IDE edit, as its key runs it: folds wait for the frame's end, and
    /// nothing runs while vim's command line is open.
    fn command_edit(&mut self, edit: IdeEdit) {
        if self.engine.mode() == Mode::Command {
            return;
        }
        if matches!(edit, IdeEdit::Fold { .. } | IdeEdit::Unfold { .. }) {
            self.out.fold_actions.push(edit);
        } else {
            self.apply_ide_edit(edit);
        }
    }

    /// `reference:line` for the caret, as Cmd/Ctrl+Alt+Shift+C copies it.
    fn copy_reference(&self, canvas: &Canvas<'_>) {
        if let Some(reference) = &self.options.reference {
            canvas
                .ui
                .ctx()
                .copy_text(RenderHelpers::caret_reference(reference, self.engine));
        }
    }
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
