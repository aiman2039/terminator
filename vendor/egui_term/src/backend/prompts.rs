//! Command navigation over the shell marks in the grid (see [`super::marks`]):
//! prompt jumps, command selection and the last command's output. All of
//! it is local: nothing here writes to the PTY.
use super::marks::{self, CommandEnded, PromptMark};
use super::TerminalBackend;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Shortest time between two full-grid scans for the painted marks.
const RESCAN: Duration = Duration::from_millis(250);

/// The painted prompt marks: a full-grid scan, redone at most every
/// [`RESCAN`] after output changed the grid.
#[derive(Default)]
pub(super) struct PromptCache {
    marks: Vec<PromptMark>,
    stale: bool,
    scanned: Option<Instant>,
}

impl PromptCache {
    pub(super) fn mark_stale(&mut self) {
        self.stale = true;
    }

    fn due(&self, now: Instant) -> bool {
        self.scanned.is_none()
            || (self.stale
                && self
                    .scanned
                    .is_some_and(|at| now.saturating_duration_since(at) >= RESCAN))
    }
}

impl TerminalBackend {
    /// Shell commands that ended since the last call (needs the shell's
    /// `C` and `D` marks while this terminal is attached).
    pub fn take_ended_commands(&self) -> Vec<CommandEnded> {
        self.command_log.take()
    }

    /// Every prompt in the grid, oldest first. Cached; rescanned at most
    /// every 250 ms while output arrives. Grid lines, not viewport rows.
    pub fn prompt_marks(&mut self) -> &[PromptMark] {
        let now = Instant::now();
        if self.prompt_cache.due(now) {
            let term = self.term.lock();
            self.prompt_cache.marks = if term.mode().contains(TermMode::ALT_SCREEN) {
                Vec::new()
            } else {
                marks::prompt_marks(term.grid())
            };
            self.prompt_cache.stale = false;
            self.prompt_cache.scanned = Some(now);
        }
        &self.prompt_cache.marks
    }

    /// Scroll the view so the previous (or next) prompt is its top row.
    /// False when there is no prompt that way or the alternate screen is up.
    pub fn jump_to_prompt(&mut self, forward: bool) -> bool {
        let term = self.term.clone();
        let mut term = term.lock();
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return false;
        }
        let marks = marks::prompt_marks(term.grid());
        let current = term.grid().display_offset() as i32;
        let target = marks::jump_target(&marks, -current, forward);
        let desired = match target {
            Some(line) => (-line).clamp(0, term.grid().history_size() as i32),
            None if forward => 0,
            None => return false,
        };
        if desired == current {
            return false;
        }
        term.scroll_display(Scroll::Delta(desired - current));
        self.grid_dirty.store(true, Ordering::Relaxed);
        true
    }

    /// Select one command with its output: the prompt at the top of a
    /// scrolled view, else the last command that ended. False when the
    /// shell left no marks.
    pub fn select_command(&mut self) -> bool {
        let term = self.term.clone();
        let mut term = term.lock();
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return false;
        }
        let marks = marks::prompt_marks(term.grid());
        let offset = term.grid().display_offset() as i32;
        let Some(index) = marks::current_command(&marks, -offset, offset > 0) else {
            return false;
        };
        let Some(mark) = marks.get(index) else {
            return false;
        };
        let bottom = term.grid().bottommost_line().0;
        let end = marks
            .get(index + 1)
            .map_or(bottom, |next| next.line - 1)
            .max(mark.line);
        let start = Point::new(Line(mark.line), Column(0));
        let mut selection = Selection::new(SelectionType::Lines, start, Side::Left);
        selection.update(
            Point::new(Line(end), term.grid().last_column()),
            Side::Right,
        );
        term.selection = Some(selection);
        term.scroll_to_point(start);
        self.capture_selection(&term);
        self.grid_dirty.store(true, Ordering::Relaxed);
        true
    }

    /// Output text of the last command that ended, when the shell marked
    /// where it starts. Trailing blank lines are dropped.
    pub fn last_command_output(&self) -> Option<String> {
        let term = self.term.lock();
        if term.mode().contains(TermMode::ALT_SCREEN) {
            return None;
        }
        let marks = marks::prompt_marks(term.grid());
        let index = marks.iter().rposition(|m| m.finished)?;
        let next = marks.get(index + 1)?;
        let start = marks.get(index)?.output?;
        if start.line.0 >= next.line {
            return Some(String::new());
        }
        let end = Point::new(Line(next.line - 1), term.grid().last_column());
        let text = term.bounds_to_string(start, end);
        Some(text.trim_end().to_owned())
    }
}

#[cfg(all(test, unix))]
#[path = "prompts_tests.rs"]
mod tests;
