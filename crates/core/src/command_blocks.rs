//! Shell integration marks and the command blocks they describe.
//!
//! Shells report prompt and command boundaries with OSC 133 (FinalTerm),
//! OSC 633 (VS Code) or OSC 1341 (JetBrains, hex-encoded values). The
//! daemon parses them from the PTY stream and keeps a bounded list of
//! [`CommandBlock`]s per live session. Blocks are never inferred from
//! terminal text: a shell without the marks has no blocks.
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, path::PathBuf};

/// The daemon answers [`crate::Request::CommandBlocks`].
pub const CAPABILITY: &str = "command-blocks-v1";
/// Blocks kept per session; the oldest are dropped first.
pub const MAX_BLOCKS: usize = 500;
/// Output text kept per block; the start is dropped first.
pub const MAX_OUTPUT: usize = 64 * 1024;
/// Command line length kept per block.
const MAX_COMMAND: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mark {
    /// `A`: the prompt starts.
    PromptStart,
    /// `B`: the prompt ends; the user types the command.
    PromptEnd,
    /// `C`: the command starts and its output follows.
    CommandStart { command: Option<String> },
    /// `D`: the command finished.
    CommandFinished { exit_code: Option<i32> },
    /// The command line, sent before `C` (633 `E`).
    CommandLine(String),
    /// The shell's working directory (633 `P;Cwd=`, 1341 `current_directory`).
    Cwd(PathBuf),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandBlock {
    /// Increases per session; stable while the session lives.
    pub id: u64,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// `None` while running, or when the shell did not report it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub started_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_ms: Option<u64>,
    /// Printed output without escape sequences, at most [`MAX_OUTPUT`]
    /// bytes (the tail is kept).
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub output_truncated: bool,
}

impl CommandBlock {
    pub fn running(&self) -> bool {
        self.finished_ms.is_none()
    }

    pub fn failed(&self) -> bool {
        self.exit_code.is_some_and(|code| code != 0)
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.finished_ms
            .map(|end| end.saturating_sub(self.started_ms))
    }
}

/// Parse one OSC sequence's `;`-separated parameters. Empty when it is
/// not a shell integration mark.
pub fn parse_osc(params: &[&[u8]]) -> Vec<Mark> {
    let Some((code, rest)) = params.split_first() else {
        return Vec::new();
    };
    match *code {
        b"133" => parse_final_term(rest).into_iter().collect(),
        b"633" => parse_vscode(rest).into_iter().collect(),
        b"1341" => parse_jetbrains(rest),
        _ => Vec::new(),
    }
}

fn exit_code(value: Option<&&[u8]>) -> Option<i32> {
    std::str::from_utf8(value?).ok()?.trim().parse().ok()
}

fn parse_final_term(rest: &[&[u8]]) -> Option<Mark> {
    match *rest.first()? {
        b"A" => Some(Mark::PromptStart),
        b"B" => Some(Mark::PromptEnd),
        b"C" => Some(Mark::CommandStart { command: None }),
        b"D" => Some(Mark::CommandFinished {
            exit_code: exit_code(rest.get(1)),
        }),
        _ => None,
    }
}

fn parse_vscode(rest: &[&[u8]]) -> Option<Mark> {
    match *rest.first()? {
        b"E" => {
            // A `;` inside the command is escaped as `\x3b`, but rejoin in
            // case a shell sent it raw.
            let raw = rest.get(1..)?.join(&b';');
            Some(Mark::CommandLine(unescape_vscode(&raw)))
        }
        b"P" => {
            let property = rest.get(1)?;
            let value = property.strip_prefix(b"Cwd=")?;
            Some(Mark::Cwd(PathBuf::from(unescape_vscode(value))))
        }
        _ => parse_final_term(rest),
    }
}

/// `\\` is a backslash and `\xAB` is one byte; other text is literal.
fn unescape_vscode(raw: &[u8]) -> String {
    let mut out = Vec::with_capacity(raw.len());
    let mut rest = raw;
    while let Some((&byte, tail)) = rest.split_first() {
        match (byte, tail) {
            (b'\\', [b'\\', after @ ..]) => {
                out.push(b'\\');
                rest = after;
            }
            (b'\\', [b'x', high, low, after @ ..]) => {
                if let Some(value) = hex_byte(*high, *low) {
                    out.push(value);
                    rest = after;
                } else {
                    out.push(byte);
                    rest = tail;
                }
            }
            _ => {
                out.push(byte);
                rest = tail;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_byte(high: u8, low: u8) -> Option<u8> {
    let digit = |c: u8| {
        char::from(c)
            .to_digit(16)
            .and_then(|d| u8::try_from(d).ok())
    };
    digit(high)?.checked_mul(16)?.checked_add(digit(low)?)
}

fn hex_text(value: &[u8]) -> Option<String> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    let bytes = value
        .chunks_exact(2)
        .map(|pair| match pair {
            [high, low] => hex_byte(*high, *low),
            _ => None,
        })
        .collect::<Option<Vec<u8>>>()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn parse_jetbrains(rest: &[&[u8]]) -> Vec<Mark> {
    let Some((event, fields)) = rest.split_first() else {
        return Vec::new();
    };
    let field = |name: &str| {
        fields.iter().find_map(|field| {
            let (key, value) = std::str::from_utf8(field).ok()?.split_once('=')?;
            (key == name).then(|| hex_text(value.as_bytes())).flatten()
        })
    };
    match *event {
        b"prompt_started" => vec![Mark::PromptStart],
        b"prompt_finished" => vec![Mark::PromptEnd],
        b"command_started" => vec![Mark::CommandStart {
            command: field("command"),
        }],
        b"command_finished" => {
            // The exit belongs to the finished block; the cwd to the next.
            let mut marks = vec![Mark::CommandFinished {
                exit_code: field("exit_code").and_then(|code| code.trim().parse().ok()),
            }];
            marks.extend(field("current_directory").map(|cwd| Mark::Cwd(cwd.into())));
            marks
        }
        _ => Vec::new(),
    }
}

/// What the tracker is doing with printed text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    /// Between `B` and `C`: the echoed command line.
    Input,
    /// Between `C` and `D`: command output.
    Output,
}

/// Per-session state machine fed with marks and printed text.
#[derive(Debug, Default)]
pub struct Tracker {
    blocks: VecDeque<CommandBlock>,
    next_id: u64,
    phase: Phase,
    cwd: Option<PathBuf>,
    pending_command: Option<String>,
    echoed: String,
}

impl Tracker {
    pub fn blocks(&self) -> impl Iterator<Item = &CommandBlock> {
        self.blocks.iter()
    }

    pub fn cwd(&self) -> Option<&PathBuf> {
        self.cwd.as_ref()
    }

    /// Apply one mark. Returns the block that just finished, if any.
    pub fn apply(&mut self, mark: Mark, now_ms: u64) -> Option<CommandBlock> {
        match mark {
            Mark::PromptStart => {
                // A prompt without `D` (Ctrl-C at some shells) ends the block.
                let finished = self.finish(None, now_ms);
                self.phase = Phase::Idle;
                finished
            }
            Mark::PromptEnd => {
                self.phase = Phase::Input;
                self.echoed.clear();
                None
            }
            Mark::CommandLine(command) => {
                self.pending_command = Some(command);
                None
            }
            Mark::Cwd(cwd) => {
                self.cwd = Some(cwd);
                None
            }
            Mark::CommandStart { command } => {
                let finished = self.finish(None, now_ms);
                let echoed = std::mem::take(&mut self.echoed);
                let command = command
                    .or_else(|| self.pending_command.take())
                    .unwrap_or_else(|| echoed.trim().to_owned());
                self.pending_command = None;
                self.start(command, now_ms);
                finished
            }
            Mark::CommandFinished { exit_code } => {
                let finished = self.finish(exit_code, now_ms);
                self.phase = Phase::Idle;
                finished
            }
        }
    }

    fn start(&mut self, command: String, now_ms: u64) {
        if self.blocks.len() >= MAX_BLOCKS {
            self.blocks.pop_front();
        }
        self.next_id = self.next_id.saturating_add(1);
        self.blocks.push_back(CommandBlock {
            id: self.next_id,
            command: command.chars().take(MAX_COMMAND).collect(),
            cwd: self.cwd.clone(),
            started_ms: now_ms,
            ..CommandBlock::default()
        });
        self.phase = Phase::Output;
    }

    fn finish(&mut self, exit_code: Option<i32>, now_ms: u64) -> Option<CommandBlock> {
        if self.phase != Phase::Output {
            return None;
        }
        let block = self.blocks.back_mut()?;
        if !block.running() {
            return None;
        }
        block.exit_code = exit_code;
        block.finished_ms = Some(now_ms);
        self.phase = Phase::Idle;
        Some(block.clone())
    }

    /// Printed text from the PTY. Only command lines and output are kept.
    pub fn text(&mut self, text: &str) {
        match self.phase {
            Phase::Idle => {}
            Phase::Input => {
                if self.echoed.len() < MAX_COMMAND {
                    self.echoed.push_str(text);
                }
            }
            Phase::Output => {
                let Some(block) = self.blocks.back_mut() else {
                    return;
                };
                block.output.push_str(text);
                if block.output.len() > MAX_OUTPUT {
                    let mut cut = block.output.len().saturating_sub(MAX_OUTPUT);
                    while !block.output.is_char_boundary(cut) {
                        cut = cut.saturating_add(1);
                    }
                    block.output.drain(..cut);
                    block.output_truncated = true;
                }
            }
        }
    }

    /// A backspace while the command line is echoed.
    pub fn backspace(&mut self) {
        if self.phase == Phase::Input {
            self.echoed.pop();
        }
    }
}

#[cfg(test)]
#[path = "command_blocks_tests.rs"]
mod tests;
