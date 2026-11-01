//! Shell integration: a second, light `vte` pass over PTY output that
//! finds OSC 133/633/1341 marks and the printed text between them. The
//! vt100 screen model stays the only terminal state; this pass keeps no
//! screen and never answers queries.
use terminator_core::command_blocks::{self, CommandBlock, Tracker};

#[derive(Default)]
struct Perform {
    tracker: Tracker,
    finished: Vec<CommandBlock>,
    now_ms: u64,
    text: String,
}

impl Perform {
    fn flush_text(&mut self) {
        if !self.text.is_empty() {
            self.tracker.text(&self.text);
            self.text.clear();
        }
    }
}

impl vte::Perform for Perform {
    fn print(&mut self, c: char) {
        self.text.push(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => self.text.push('\n'),
            b'\t' => self.text.push('\t'),
            0x08 if self.text.pop().is_none() => self.tracker.backspace(),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        let marks = command_blocks::parse_osc(params);
        if marks.is_empty() {
            return;
        }
        self.flush_text();
        for mark in marks {
            self.finished.extend(self.tracker.apply(mark, self.now_ms));
        }
    }
}

/// Command blocks of one live session.
pub(crate) struct Commands {
    parser: vte::Parser,
    perform: Perform,
}

impl Default for Commands {
    fn default() -> Self {
        Self {
            parser: vte::Parser::new(),
            perform: Perform::default(),
        }
    }
}

impl Commands {
    /// Feed PTY output. Returns blocks that finished in it.
    pub(crate) fn process(&mut self, bytes: &[u8], now_ms: u64) -> Vec<CommandBlock> {
        self.perform.now_ms = now_ms;
        self.parser.advance(&mut self.perform, bytes);
        self.perform.flush_text();
        std::mem::take(&mut self.perform.finished)
    }

    pub(crate) fn tracker(&self) -> &Tracker {
        &self.perform.tracker
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_split_across_reads_still_make_a_block() {
        let mut commands = Commands::default();
        let stream = b"\x1b]133;A\x07$ \x1b]133;B\x07\x1b]633;E;cargo test\x07\x1b]133;C\x07\x1b[31mfail\x1b[0m\r\nok\r\n\x1b]133;D;101\x07";
        let mut finished = Vec::new();
        for (at, chunk) in stream.chunks(3).enumerate() {
            finished.extend(commands.process(chunk, at as u64));
        }
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].command, "cargo test");
        assert_eq!(finished[0].output, "fail\nok\n");
        assert_eq!(finished[0].exit_code, Some(101));
        assert_eq!(commands.tracker().blocks().count(), 1);
    }

    #[test]
    fn output_without_marks_creates_no_blocks() {
        let mut commands = Commands::default();
        let finished = commands.process(b"plain output\r\n\x1b]0;title\x07more", 1);
        assert!(finished.is_empty());
        assert_eq!(commands.tracker().blocks().count(), 0);
    }

    #[test]
    fn st_terminated_marks_are_recognized() {
        let mut commands = Commands::default();
        let finished = commands.process(
            b"\x1b]133;B\x1b\\ls\x1b]133;C\x1b\\a\n\x1b]133;D;0\x1b\\",
            1,
        );
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].command, "ls");
        assert_eq!(finished[0].exit_code, Some(0));
    }
}
