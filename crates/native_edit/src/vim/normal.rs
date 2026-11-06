//! Normal-mode key dispatch: counts, prefixes, motions and commands.
use super::super::doc::Buffer;
use super::mode::{Bit, Effect, Key, Mode, PendingOp, Place, Prefix};
use super::motion::{Count, Mo, Motion};
use super::state::VimEngine;
use super::words::{WordClass, current_char, file_name_at, word_class};

impl VimEngine {
    pub(super) fn normal_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        // Vim parity: Space moves right like `l`, including under operators.
        let key = if key == Key::Char(' ') {
            Key::Char('l')
        } else {
            key
        };
        // `g` waits for its key but still takes count digits (`g3…`).
        if let Some(prefix) = self.x.prefix.take_if(|p| *p != Prefix::G) {
            return self.prefix_key(doc, prefix, key);
        }
        if self.count_digit(key) {
            return Vec::new();
        }
        if self.x.prefix.take().is_some() {
            return self.g_command(doc, key);
        }
        let count = self.take_count();
        if let Some(mo) = Self::motion_for_key(key) {
            return self.run_motion(doc, mo, count);
        }
        if self.pending.is_some() {
            return self.pending_key(doc, key, count);
        }
        if let Some(effects) = self.edit_command(doc, key, count) {
            return effects;
        }
        self.normal_command(doc, key, count)
    }

    /// Digits accumulate a count; a bare 0 is the line-start motion.
    fn count_digit(&mut self, key: Key) -> bool {
        if let Key::Char(c) = key
            && c.is_ascii_digit()
            && !(c == '0' && self.count.is_empty())
        {
            self.count.push(c);
            self.rec_bit(Bit::Key(key));
            return true;
        }
        false
    }

    /// Prefix keys shared by Normal, Visual and operator-pending mode:
    /// `g`, `f`/`t`/`F`/`T`, mark jumps, and `i`/`a` text objects (Normal
    /// mode takes `i`/`a` as inserts before asking).
    pub(super) fn prefix_for_key(key: Key) -> Option<Prefix> {
        Some(match key {
            Key::Char('g') => Prefix::G,
            Key::Char(c @ ('f' | 't' | 'F' | 'T')) => Prefix::Find(c),
            Key::Char(c @ ('\'' | '`')) => Prefix::Jump { exact: c == '`' },
            Key::Char(c @ ('i' | 'a')) => Prefix::Object { inner: c == 'i' },
            _ => return None,
        })
    }

    /// Keys that are motions in Normal, Visual and operator-pending mode.
    pub(super) fn motion_for_key(key: Key) -> Option<Mo> {
        Some(match key {
            Key::Char('h') | Key::Left | Key::Backspace | Key::Ctrl('h') => Mo::Left,
            Key::Char('l') | Key::Right => Mo::Right,
            Key::Char('j') | Key::Down | Key::Ctrl('j' | 'n') => Mo::Down,
            Key::Char('k') | Key::Up | Key::Ctrl('p') => Mo::Up,
            Key::Char(c @ ('w' | 'W')) => Mo::Word { big: c == 'W' },
            Key::Char(c @ ('b' | 'B')) => Mo::Back { big: c == 'B' },
            Key::Char(c @ ('e' | 'E')) => Mo::End { big: c == 'E' },
            Key::Char('0') | Key::Home => Mo::LineStart,
            Key::Char('$') | Key::End => Mo::LineEnd,
            Key::Char('^') => Mo::FirstNonBlank,
            Key::Char('_') => Mo::CurrentLine,
            Key::Char('+') | Key::Enter => Mo::NextLine,
            Key::Char('-') => Mo::PrevLine,
            Key::Char('|') => Mo::Column,
            Key::Char('G') => Mo::GotoLine { last: true },
            Key::Char(';') => Mo::FindAgain { reverse: false },
            Key::Char(',') => Mo::FindAgain { reverse: true },
            Key::Char('%') => Mo::MatchPair,
            Key::Char('}') => Mo::ParaFwd,
            Key::Char('{') => Mo::ParaBack,
            Key::Char('H') => Mo::ScreenTop,
            Key::Char('M') => Mo::ScreenMid,
            Key::Char('L') => Mo::ScreenBottom,
            Key::Char('n') => Mo::Search { reverse: false },
            Key::Char('N') => Mo::Search { reverse: true },
            Key::Char('*') => Mo::Star {
                forward: true,
                whole: true,
            },
            Key::Char('#') => Mo::Star {
                forward: false,
                whole: true,
            },
            _ => return None,
        })
    }

    /// The char after a prefix key (`fx`, `ma`, `"a`, `qa`, `@a`, `zz`, …).
    fn prefix_key<B: Buffer>(&mut self, doc: &mut B, prefix: Prefix, key: Key) -> Vec<Effect> {
        let count = self.take_count();
        let ch = match key {
            Key::Char(c) => Some(c),
            Key::Enter if matches!(prefix, Prefix::Replace | Prefix::Z) => Some('\n'),
            // Ctrl-W Ctrl-H is Ctrl-W h, as in vim.
            Key::Ctrl(c) if prefix == Prefix::Window => Some(c),
            _ => None,
        };
        let Some(ch) = ch.filter(|_| key != Key::Escape) else {
            self.clear_pending();
            self.rec_discard();
            return if key == Key::Escape {
                Vec::new()
            } else {
                vec![Effect::Bell]
            };
        };
        match prefix {
            Prefix::Find(kind) => self.run_motion(doc, Mo::Find { kind, ch }, count),
            Prefix::Jump { exact } => self.run_motion(doc, Mo::Mark { name: ch, exact }, count),
            Prefix::Object { inner } => self.object_op(doc, (ch, inner), count),
            Prefix::Replace => self.replace_chars(doc, ch, count),
            Prefix::Register => self.select_register(ch),
            Prefix::Record => self.macro_start(ch),
            Prefix::Play => self.macro_play(doc, ch, count),
            Prefix::Mark => {
                let set = self.set_mark(ch);
                self.finish_simple(set)
            }
            Prefix::Z => {
                let done = self.z_key(doc, key);
                self.finish_simple(done)
            }
            Prefix::G => self.g_command(doc, key),
            Prefix::Window => self.window_command(ch),
        }
    }

    /// End a command that changes nothing: bell when it failed.
    pub(super) fn finish_simple(&mut self, ok: bool) -> Vec<Effect> {
        self.clear_pending();
        self.rec_discard();
        if ok { Vec::new() } else { vec![Effect::Bell] }
    }

    /// `"x`: the register for the next command.
    fn select_register(&mut self, c: char) -> Vec<Effect> {
        if c.is_ascii_alphanumeric() || matches!(c, '"' | '-' | '_' | '+' | '*') {
            self.x.reg_sel = Some(c);
            return Vec::new();
        }
        self.finish_simple(false)
    }

    /// Resolve and apply a motion: a move, or an operator's target.
    pub(super) fn run_motion<B: Buffer>(
        &mut self,
        doc: &mut B,
        mo: Mo,
        count: usize,
    ) -> Vec<Effect> {
        let typed = self.has_count();
        self.run_motion_counted(doc, mo, Count { n: count, typed })
    }

    pub(super) fn run_motion_counted<B: Buffer>(
        &mut self,
        doc: &mut B,
        mo: Mo,
        count: Count,
    ) -> Vec<Effect> {
        // Vim special case: `cw` on a word changes to its end, like `ce`.
        let mo = match mo {
            Mo::Word { big }
                if self.pending == Some(PendingOp::Change)
                    && word_class(current_char(doc, self.cursor)) != WordClass::Blank =>
            {
                Mo::ChangeWord { big }
            }
            _ => mo,
        };
        let Some(motion) = self.resolve(doc, mo, count) else {
            self.clear_pending();
            self.rec_discard();
            return vec![Effect::Bell];
        };
        if let Some(op) = self.pending.take() {
            match mo.legacy() {
                Some(c) => self.rec_bit(Bit::Motion(c)),
                None => self.x.keys_change = true,
            }
            self.clear_pending();
            self.operate_motion(doc, op, (mo, motion));
            return Vec::new();
        }
        self.clear_pending();
        self.rec_discard();
        self.move_to(doc, mo, motion);
        Vec::new()
    }

    /// Plain cursor move (Normal and Visual).
    pub(super) fn move_to<B: Buffer>(&mut self, doc: &mut B, mo: Mo, motion: Motion) {
        if motion.jump {
            self.push_jump();
            let view = self.x.viewport;
            let line = motion.target.line;
            if (view.top == 0 && view.bottom == 0) || line < view.top || line > view.bottom {
                self.scroll_request = true;
            }
        }
        self.cursor = motion.target;
        if !motion.keep_col {
            self.preferred_col = if mo == Mo::LineEnd {
                usize::MAX
            } else {
                motion.target.col
            };
        }
        self.clamp(doc);
        doc.end_run();
    }

    /// Recorded-motion replay entry (`.` of `dw`, `cG`, …) and the `s`
    /// command: `kind` is a legacy motion char.
    pub(super) fn apply_motion<B: Buffer>(&mut self, doc: &mut B, kind: char, count: usize) {
        if let Some(mo) = Mo::from_legacy(kind) {
            let typed = !(kind == 'G' && count == 0);
            let _ = self.run_motion_counted(
                doc,
                mo,
                Count {
                    n: count.max(1),
                    typed,
                },
            );
        }
    }

    /// Keys after an operator that are not motions.
    fn pending_key<B: Buffer>(&mut self, doc: &mut B, key: Key, count: usize) -> Vec<Effect> {
        let op = self.pending.unwrap_or(PendingOp::Delete);
        if let Some(prefix) = Self::prefix_for_key(key) {
            self.x.prefix = Some(prefix);
            return Vec::new();
        }
        match key {
            Key::Char(c) if c == op.letter() => self.doubled_operator(doc, op, count),
            Key::Escape => self.finish_simple(true),
            _ => self.finish_simple(false),
        }
    }

    /// `dd`, `cc`, `yy`, `>>`, `<<`, `guu`, `gUU`, `g~~`: `count` lines.
    pub(super) fn doubled_operator<B: Buffer>(
        &mut self,
        doc: &mut B,
        op: PendingOp,
        count: usize,
    ) -> Vec<Effect> {
        if matches!(op, PendingOp::Delete | PendingOp::Change | PendingOp::Yank) {
            self.rec_bit(Bit::Op(op.letter()));
        } else {
            self.x.keys_change = true;
        }
        self.clear_pending();
        let first = self.cursor.line;
        let last = first.saturating_add(count.saturating_sub(1));
        self.operate_lines(doc, op, (first, last));
        Vec::new()
    }

    /// Text object under an operator (`diw`, `ci(`, `yap`).
    fn object_op<B: Buffer>(
        &mut self,
        doc: &mut B,
        (obj, inner): (char, bool),
        count: usize,
    ) -> Vec<Effect> {
        let Some(op) = self.pending.take() else {
            return self.finish_simple(false);
        };
        let Some(span) = self.text_object(doc, (obj, inner), count) else {
            return self.finish_simple(false);
        };
        self.x.keys_change = true;
        self.clear_pending();
        if span.linewise {
            let (first, _) = doc.line_col_at(span.from);
            let (last, _) = doc.line_col_at(span.to);
            self.operate_lines(doc, op, (first, last));
        } else {
            self.operate_chars(doc, op, (span.from, span.to));
        }
        Vec::new()
    }

    /// `g` commands: `gg`, `gd`, `gf`, `ge`, `gJ`, `gu`, `gU`, `g~`, `gv`,
    /// `g*`, `g#`.
    fn g_command<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        let count = self.take_count();
        let motion = match key {
            Key::Char('g') => Some(Mo::GotoLine { last: false }),
            Key::Char(c @ ('e' | 'E')) => Some(Mo::EndBack { big: c == 'E' }),
            Key::Char('*') => Some(Mo::Star {
                forward: true,
                whole: false,
            }),
            Key::Char('#') => Some(Mo::Star {
                forward: false,
                whole: false,
            }),
            _ => None,
        };
        if let Some(mo) = motion {
            return self.run_motion(doc, mo, count);
        }
        let op = match key {
            Key::Char('u') => Some(PendingOp::Lower),
            Key::Char('U') => Some(PendingOp::Upper),
            Key::Char('~') => Some(PendingOp::Toggle),
            _ => None,
        };
        if let Some(op) = op {
            if self.pending == Some(op) {
                return self.doubled_operator(doc, op, count);
            }
            if self.pending.is_none() {
                return self.operator_key(doc, op);
            }
        }
        if self.pending.is_some() {
            return self.finish_simple(false);
        }
        match key {
            Key::Char('d') => {
                self.finish_simple(true);
                self.scroll_request = true;
                vec![Effect::GotoDefinition]
            }
            Key::Char('f') => self.goto_file(doc),
            Key::Char('J') => self.join_key(doc, count, false),
            Key::Char('v') => self.reselect_visual(doc),
            _ => self.finish_simple(false),
        }
    }

    /// Commands that are neither motions nor edits: modes, prompts,
    /// prefixes, scrolling, jumps and host effects.
    fn normal_command<B: Buffer>(&mut self, doc: &mut B, key: Key, count: usize) -> Vec<Effect> {
        let prefix = Self::prefix_for_key(key).or(match key {
            Key::Char('m') => Some(Prefix::Mark),
            Key::Char('"') => Some(Prefix::Register),
            Key::Char('@') => Some(Prefix::Play),
            Key::Char('z') => Some(Prefix::Z),
            Key::Ctrl('w') => Some(Prefix::Window),
            Key::Char('q') if self.x.recording_macro.is_none() => Some(Prefix::Record),
            _ => None,
        });
        if let Some(prefix) = prefix {
            self.x.prefix = Some(prefix);
            return Vec::new();
        }
        match key {
            Key::Char('q') => {
                self.macro_stop();
                self.finish_simple(true)
            }
            Key::Char('v') => self.start_visual(key, Mode::Visual),
            Key::Char('V') => self.start_visual(key, Mode::VisualLine),
            Key::CtrlV => self.start_visual(key, Mode::VisualBlock),
            Key::Char(':') => self.open_command_line(""),
            Key::Char('/') => self.open_command_line("/"),
            Key::Char('?') => self.open_command_line("?"),
            Key::Ctrl(c @ ('d' | 'u' | 'f' | 'b' | 'e' | 'y')) => {
                let typed = self.has_count();
                self.scroll_key(doc, c, (count, typed));
                self.finish_simple(true)
            }
            Key::Ctrl('o') => self.walk_jumps_or_history(doc, true, count),
            Key::Ctrl('i') | Key::Tab => self.walk_jumps_or_history(doc, false, count),
            // Ctrl-^ (Ctrl-6): edit the alternate file; the GUI owns files.
            Key::CtrlCaret => {
                self.finish_simple(true);
                vec![Effect::AlternateFile]
            }
            // `K` looks up the cursor word, like vim with `keywordprg` on
            // an LSP: no buffer change, nothing to repeat or record.
            Key::Char('K') => {
                self.finish_simple(true);
                vec![Effect::Hover]
            }
            Key::Escape => self.finish_simple(true),
            _ => self.finish_simple(false),
        }
    }

    /// `gf`: open the file name under the cursor; the GUI resolves it.
    fn goto_file<B: Buffer>(&mut self, doc: &mut B) -> Vec<Effect> {
        self.finish_simple(true);
        match file_name_at(&doc.line_text(self.cursor.line), self.cursor.col) {
            Some(name) => vec![Effect::GotoFile(name)],
            None => vec![Effect::Bell],
        }
    }

    pub(super) fn start_visual(&mut self, key: Key, mode: Mode) -> Vec<Effect> {
        self.rec_bit(Bit::Key(key));
        self.mode = mode;
        self.anchor = self.cursor;
        self.x.block_eol = false;
        self.clear_pending();
        Vec::new()
    }

    /// `gv`: the last visual selection again.
    fn reselect_visual<B: Buffer>(&mut self, doc: &mut B) -> Vec<Effect> {
        let Some((mode, anchor, cursor)) = self.x.last_visual else {
            return self.finish_simple(false);
        };
        self.finish_simple(true);
        self.mode = mode;
        self.anchor = anchor;
        self.cursor = cursor;
        self.clamp(doc);
        Vec::new()
    }

    /// `:` opens an empty command line, `/` and `?` search prompts.
    fn open_command_line(&mut self, prefix: &str) -> Vec<Effect> {
        self.mode = Mode::Command;
        self.command.clear();
        self.command.push_str(prefix);
        self.clear_pending();
        self.rec_discard();
        Vec::new()
    }

    pub(super) fn operator_key<B: Buffer>(&mut self, doc: &mut B, op: PendingOp) -> Vec<Effect> {
        if self.pending == Some(op) {
            let count = self.take_count();
            return self.doubled_operator(doc, op, count);
        }
        if self.pending.is_some() {
            return self.finish_simple(false);
        }
        if matches!(op, PendingOp::Delete | PendingOp::Change | PendingOp::Yank) {
            self.rec_bit(Bit::Op(op.letter()));
        } else {
            self.x.keys_change = true;
        }
        // The count before an operator multiplies the motion's count
        // (`2d3w` is six words), never concatenates with it.
        if !self.count.is_empty() {
            let typed = self.take_count();
            self.x.op_count = Some(typed);
            self.count.clear();
        }
        self.pending = Some(op);
        Vec::new()
    }

    /// Insert-entering keys, with a count repeating the typed text.
    pub(super) fn insert_key_at<B: Buffer>(
        &mut self,
        doc: &mut B,
        (key, place): (Key, Place),
        count: usize,
    ) {
        self.rec_bit(Bit::Key(key));
        self.start_insert_place(doc, place);
        if count > 1 {
            self.x.insert_repeat = Some((count, place));
        }
    }
}
