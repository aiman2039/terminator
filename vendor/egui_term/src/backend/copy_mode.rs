//! Keyboard copy mode: alacritty's vi-mode cursor and selection, driven by
//! the embedding app. Nothing here writes to the PTY.
use super::TerminalBackend;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vi_mode::ViMotion;
use std::sync::atomic::Ordering;

/// One copy-mode command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyCommand {
    Motion(ViMotion),
    /// Scroll the view and cursor by this many lines (positive is up).
    Scroll(i32),
    /// First row of the retained history.
    Top,
    /// Last row of the live screen.
    Bottom,
    /// Start a selection of this kind at the cursor, or end the one of
    /// the same kind.
    Select(SelectionType),
    /// Drop the selection; the mode stays on.
    ClearSelection,
}

impl TerminalBackend {
    /// Copy mode is on: the vi cursor moves, the PTY gets no keys.
    pub fn copy_mode(&self) -> bool {
        self.term.lock().mode().contains(TermMode::VI)
    }

    /// Turn copy mode on or off. Off drops the selection and returns to the
    /// live screen.
    pub fn set_copy_mode(&mut self, on: bool) {
        let term = self.term.clone();
        let mut term = term.lock();
        if term.mode().contains(TermMode::VI) != on {
            term.toggle_vi_mode();
        }
        if !on {
            term.selection = None;
            term.scroll_display(Scroll::Bottom);
        }
        self.capture_selection(&term);
        self.grid_dirty.store(true, Ordering::Relaxed);
    }

    pub fn copy_mode_command(&mut self, command: CopyCommand) {
        let term = self.term.clone();
        let mut term = term.lock();
        if !term.mode().contains(TermMode::VI) {
            return;
        }
        match command {
            CopyCommand::Motion(motion) => term.vi_motion(motion),
            CopyCommand::Scroll(lines) => {
                term.scroll_display(Scroll::Delta(lines));
                term.vi_mode_cursor = term.vi_mode_cursor.scroll(&term, lines);
                let cursor = term.vi_mode_cursor.point;
                term.vi_goto_point(cursor);
            }
            CopyCommand::Top => {
                let point = Point::new(term.grid().topmost_line(), Column(0));
                term.vi_goto_point(point);
            }
            CopyCommand::Bottom => {
                let point = Point::new(term.grid().bottommost_line(), Column(0));
                term.vi_goto_point(point);
            }
            CopyCommand::Select(kind) => {
                let same = term.selection.as_ref().is_some_and(|s| s.ty == kind);
                term.selection = if same {
                    None
                } else {
                    let cursor = term.vi_mode_cursor.point;
                    let mut selection = Selection::new(kind, cursor, Side::Left);
                    selection.include_all();
                    Some(selection)
                };
            }
            CopyCommand::ClearSelection => term.selection = None,
        }
        let cursor = term.vi_mode_cursor.point;
        term.scroll_to_point(cursor);
        self.capture_selection(&term);
        self.grid_dirty.store(true, Ordering::Relaxed);
    }

    /// The copy-mode cursor as a viewport row and column, when visible.
    pub fn copy_mode_cursor(&self) -> Option<(usize, usize)> {
        let term = self.term.lock();
        if !term.mode().contains(TermMode::VI) {
            return None;
        }
        let point = term.vi_mode_cursor.point;
        let row = point.line.0 + term.grid().display_offset() as i32;
        let row = usize::try_from(row).ok()?;
        (row < term.screen_lines()).then_some((row, point.column.0))
    }

    /// Whether any text is selected.
    pub fn has_selection(&self) -> bool {
        self.term
            .lock()
            .selection
            .as_ref()
            .is_some_and(|s| !s.is_empty())
    }
}

#[cfg(all(test, unix))]
#[path = "copy_mode_tests.rs"]
mod tests;
