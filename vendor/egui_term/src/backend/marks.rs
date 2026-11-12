//! Shell integration marks (OSC 133 and OSC 633 `A`/`B`/`C`/`D`) in the
//! GUI terminal.
//!
//! alacritty ignores these sequences. The PTY reader ([`MarkedPty`])
//! rewrites each mark into an OSC 8 hyperlink with a private URI
//! ([`MARK_SCHEME`]): the prompt text (`A` to `B`) and the command output
//! (`C` to `D`) carry the link. The link is stored in the cells, so a mark
//! follows scrollback, reflow and history trimming without bookkeeping.
//! Hover and link opening ignore these links (see [`is_mark_uri`]).
//!
//! The marks describe shell commands only. They never feed agent state.
use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::tty::{self, ChildEvent, EventedPty, EventedReadWrite};
use polling::{Event as PollEvent, PollMode, Poller};
use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// URI scheme of the links that stand for shell marks.
pub const MARK_SCHEME: &str = "terminator-mark:";
/// Longest mark sequence kept back while it arrives in parts. A longer OSC
/// passes through unchanged.
const MAX_MARK: usize = 64;
/// Raw bytes per read are at most this fraction of the caller's buffer, so
/// the rewritten bytes always fit (see [`MarkedPty::read`]).
const GROWTH: usize = 12;
/// Room for the bytes kept back from an earlier read.
const RESERVE: usize = MAX_MARK + 64;
/// Finished commands kept until the app reads them.
const MAX_ENDED: usize = 32;

/// True for a link that a shell mark made.
pub fn is_mark_uri(uri: &str) -> bool {
    uri.starts_with(MARK_SCHEME)
}

/// A shell command that finished while this terminal was attached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandEnded {
    /// `None` when the shell did not report it.
    pub exit_code: Option<i32>,
    pub duration: Duration,
}

/// Finished commands, shared between the PTY reader and the backend.
#[derive(Clone, Default)]
pub struct CommandLog(Arc<Mutex<VecDeque<CommandEnded>>>);

impl CommandLog {
    fn push(&self, ended: CommandEnded) {
        let mut queue = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if queue.len() >= MAX_ENDED {
            queue.pop_front();
        }
        queue.push_back(ended);
    }

    /// Commands that finished since the last call, oldest first.
    pub fn take(&self) -> Vec<CommandEnded> {
        let mut queue = self.0.lock().unwrap_or_else(|e| e.into_inner());
        queue.drain(..).collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    PromptStart,
    PromptEnd,
    CommandStart,
    CommandEnd(Option<i32>),
}

/// The mark in one OSC payload (the bytes between `ESC ]` and the
/// terminator), or `None` when it is another sequence.
fn parse_mark(payload: &[u8]) -> Option<Mark> {
    let rest = payload
        .strip_prefix(b"133;")
        .or_else(|| payload.strip_prefix(b"633;"))?;
    let mut fields = rest.split(|b| *b == b';');
    match fields.next()? {
        b"A" => Some(Mark::PromptStart),
        b"B" => Some(Mark::PromptEnd),
        b"C" => Some(Mark::CommandStart),
        b"D" => {
            let code = fields
                .next()
                .and_then(|raw| std::str::from_utf8(raw).ok())
                .and_then(|text| text.trim().parse().ok());
            Some(Mark::CommandEnd(code))
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    /// After ESC.
    Escape,
    /// After `ESC ]`: the payload is kept back in `held`.
    Osc,
    /// ESC inside a kept-back payload.
    OscEscape,
}

/// Rewrites shell marks in a PTY byte stream (see the module docs).
pub struct MarkRewriter {
    state: State,
    held: Vec<u8>,
    next_id: u32,
    /// Exit code of the last `D` since the last prompt.
    last_exit: Option<i32>,
    started: Option<Instant>,
    log: CommandLog,
    on_end: Option<Box<dyn Fn() + Send>>,
}

impl MarkRewriter {
    pub fn new(log: CommandLog) -> Self {
        Self {
            state: State::Ground,
            held: Vec::new(),
            next_id: 0,
            last_exit: None,
            started: None,
            log,
            on_end: None,
        }
    }

    /// Called after a command ended (the app repaints to read it).
    pub fn on_end(mut self, wake: impl Fn() + Send + 'static) -> Self {
        self.on_end = Some(Box::new(wake));
        self
    }

    /// Rewrite `input` into `out`. Bytes of an incomplete mark stay held
    /// until the rest arrives.
    pub fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) {
        for &byte in input {
            self.byte(byte, out);
        }
    }

    fn byte(&mut self, byte: u8, out: &mut Vec<u8>) {
        match self.state {
            State::Ground => {
                if byte == 0x1b {
                    self.state = State::Escape;
                } else {
                    out.push(byte);
                }
            }
            State::Escape => {
                if byte == b']' {
                    self.state = State::Osc;
                    self.held.clear();
                } else {
                    out.push(0x1b);
                    self.state = State::Ground;
                    self.byte(byte, out);
                }
            }
            State::Osc => match byte {
                0x07 => self.finish(out, b"\x07"),
                0x1b => self.state = State::OscEscape,
                _ => {
                    self.held.push(byte);
                    if !self.could_be_mark() {
                        self.release(out);
                    }
                }
            },
            State::OscEscape => {
                if byte == b'\\' {
                    self.finish(out, b"\x1b\\");
                } else {
                    // Not a terminator: give back what was held and start a
                    // new escape sequence with this ESC.
                    self.release(out);
                    self.state = State::Escape;
                    self.byte(byte, out);
                }
            }
        }
    }

    /// The held payload can still become a mark.
    fn could_be_mark(&self) -> bool {
        let held = self.held.as_slice();
        if held.len() > MAX_MARK {
            return false;
        }
        let prefix = held.len().min(4);
        let head = held.get(..prefix).unwrap_or_default();
        b"133;".starts_with(head) || b"633;".starts_with(head)
    }

    /// Pass the held bytes through unchanged; the OSC continues as is.
    fn release(&mut self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"\x1b]");
        out.append(&mut self.held);
        self.state = State::Ground;
    }

    fn finish(&mut self, out: &mut Vec<u8>, terminator: &[u8]) {
        self.state = State::Ground;
        match parse_mark(&self.held) {
            Some(mark) => {
                self.held.clear();
                self.apply(mark, out);
            }
            None => {
                out.extend_from_slice(b"\x1b]");
                out.append(&mut self.held);
                out.extend_from_slice(terminator);
            }
        }
    }

    fn apply(&mut self, mark: Mark, out: &mut Vec<u8>) {
        match mark {
            Mark::PromptStart => {
                // A prompt without `D` (Ctrl-C in some shells) ends the command.
                self.end_command(None);
                self.next_id = self.next_id.wrapping_add(1);
                let last = self
                    .last_exit
                    .take()
                    .map(|code| format!("?last={code}"))
                    .unwrap_or_default();
                open_link(out, &format!("p{}", self.next_id), &format!("prompt{last}"));
            }
            Mark::PromptEnd => close_link(out),
            Mark::CommandStart => {
                self.started = Some(Instant::now());
                open_link(out, &format!("o{}", self.next_id), "output");
            }
            Mark::CommandEnd(code) => {
                close_link(out);
                self.last_exit = code;
                self.end_command(code);
            }
        }
    }

    fn end_command(&mut self, exit_code: Option<i32>) {
        let Some(started) = self.started.take() else {
            return;
        };
        self.log.push(CommandEnded {
            exit_code,
            duration: started.elapsed(),
        });
        if let Some(wake) = &self.on_end {
            wake();
        }
    }
}

fn open_link(out: &mut Vec<u8>, id: &str, kind: &str) {
    out.extend_from_slice(format!("\x1b]8;id=tmk-{id};{MARK_SCHEME}{kind}\x1b\\").as_bytes());
}

fn close_link(out: &mut Vec<u8>) {
    out.extend_from_slice(b"\x1b]8;;\x1b\\");
}

/// The PTY with a [`MarkRewriter`] on its output.
pub struct MarkedPty {
    pty: tty::Pty,
    rewriter: MarkRewriter,
    /// Rewritten bytes the caller had no room for.
    pending: VecDeque<u8>,
    raw: Vec<u8>,
    scratch: Vec<u8>,
}

impl MarkedPty {
    pub fn new(pty: tty::Pty, rewriter: MarkRewriter) -> Self {
        Self {
            pty,
            rewriter,
            pending: VecDeque::new(),
            raw: Vec::new(),
            scratch: Vec::new(),
        }
    }
}

impl Read for MarkedPty {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.pending.is_empty() {
            return Ok(drain_into(&mut self.pending, buf));
        }
        // Read only as much as surely fits after the rewrite: the event
        // loop goes back to poll after a large read, and bytes kept here
        // would wait for the next output.
        let room = buf.len().saturating_sub(RESERVE) / GROWTH;
        if room == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        self.raw.resize(room.min(0x4000), 0);
        let read = self.pty.reader().read(&mut self.raw)?;
        if read == 0 {
            return Ok(0);
        }
        self.scratch.clear();
        self.rewriter
            .feed(self.raw.get(..read).unwrap_or_default(), &mut self.scratch);
        if self.scratch.is_empty() {
            // Everything was a held part of a mark; nothing to parse yet.
            return Err(io::ErrorKind::WouldBlock.into());
        }
        self.pending.extend(self.scratch.drain(..));
        Ok(drain_into(&mut self.pending, buf))
    }
}

fn drain_into(pending: &mut VecDeque<u8>, buf: &mut [u8]) -> usize {
    let count = pending.len().min(buf.len());
    for (slot, byte) in buf.iter_mut().zip(pending.drain(..count)) {
        *slot = byte;
    }
    count
}

impl EventedReadWrite for MarkedPty {
    type Reader = MarkedPty;
    type Writer = <tty::Pty as EventedReadWrite>::Writer;

    unsafe fn register(
        &mut self,
        poll: &Arc<Poller>,
        interest: PollEvent,
        mode: PollMode,
    ) -> io::Result<()> {
        // SAFETY: the caller's contract passes through to the wrapped PTY.
        unsafe { self.pty.register(poll, interest, mode) }
    }

    fn reregister(
        &mut self,
        poll: &Arc<Poller>,
        interest: PollEvent,
        mode: PollMode,
    ) -> io::Result<()> {
        self.pty.reregister(poll, interest, mode)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.pty.deregister(poll)
    }

    fn reader(&mut self) -> &mut MarkedPty {
        self
    }

    fn writer(&mut self) -> &mut Self::Writer {
        self.pty.writer()
    }
}

impl EventedPty for MarkedPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.pty.next_child_event()
    }
}

impl OnResize for MarkedPty {
    fn on_resize(&mut self, window_size: WindowSize) {
        self.pty.on_resize(window_size);
    }
}

/// One shell prompt found in the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptMark {
    /// Grid line of the prompt's first cell (negative in history).
    pub line: i32,
    /// First cell of the command output, when the shell marked it.
    pub output: Option<Point>,
    /// Exit code of the command run at this prompt. `None` while it runs,
    /// or when the shell did not report it.
    pub exit_code: Option<i32>,
    /// A later prompt exists: the command at this prompt has ended.
    pub finished: bool,
}

/// Every prompt in the grid, oldest first.
pub fn prompt_marks(grid: &Grid<Cell>) -> Vec<PromptMark> {
    let mut marks: Vec<PromptMark> = Vec::new();
    let mut last_codes: Vec<Option<i32>> = Vec::new();
    let mut prompt_id = String::new();
    for line in grid.topmost_line().0..=grid.bottommost_line().0 {
        let row = &grid[Line(line)];
        for column in 0..grid.columns() {
            let cell = &row[Column(column)];
            if cell.extra.is_none() {
                continue;
            }
            let Some(link) = cell.hyperlink() else {
                continue;
            };
            let Some(kind) = link.uri().strip_prefix(MARK_SCHEME) else {
                continue;
            };
            if let Some(query) = kind.strip_prefix("prompt") {
                if link.id() == prompt_id {
                    continue;
                }
                link.id().clone_into(&mut prompt_id);
                last_codes.push(query.strip_prefix("?last=").and_then(|c| c.parse().ok()));
                marks.push(PromptMark {
                    line,
                    output: None,
                    exit_code: None,
                    finished: false,
                });
            } else if kind == "output" {
                if let Some(mark) = marks.last_mut().filter(|m| m.output.is_none()) {
                    mark.output = Some(Point::new(Line(line), Column(column)));
                }
            }
        }
    }
    let count = marks.len();
    for index in 1..count {
        let code = last_codes.get(index).copied().flatten();
        if let Some(mark) = marks.get_mut(index - 1) {
            mark.exit_code = code;
            mark.finished = true;
        }
    }
    marks
}

/// Where the prompt jump goes from a viewport whose top is grid line `top`:
/// the target line, or `None` when no prompt lies that way.
pub fn jump_target(marks: &[PromptMark], top: i32, forward: bool) -> Option<i32> {
    if forward {
        marks.iter().map(|m| m.line).find(|line| *line > top)
    } else {
        marks.iter().rev().map(|m| m.line).find(|line| *line < top)
    }
}

/// The command a "select command" acts on: the prompt at the top of a
/// scrolled view, else the last command that ended. Returns its index.
pub fn current_command(marks: &[PromptMark], top: i32, scrolled: bool) -> Option<usize> {
    if scrolled {
        if let Some(index) = marks.iter().rposition(|m| m.line <= top) {
            return Some(index);
        }
    }
    marks.iter().rposition(|m| m.finished)
}

#[cfg(test)]
#[path = "marks_tests.rs"]
mod tests;
