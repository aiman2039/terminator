//! Ex commands with line ranges: `:1,5s`, `:'<,'>d`, `:%y`, `:m`, `:t`,
//! `:j`, `:>`, `:g/pat/cmd`, `:v`, `:noh`, `:e path`.
use super::super::doc::Buffer;
use super::mode::{Cursor, Effect};
use super::state::VimEngine;

/// Lines (0-based, inclusive) an ex command applies to.
type Lines = (usize, usize);

impl VimEngine {
    /// Run one ex line (no leading `:`).
    pub(crate) fn run_ex<B: Buffer>(&mut self, doc: &mut B, line: &str) -> Vec<Effect> {
        let Some((range, rest)) = self.parse_range(doc, line.trim_start()) else {
            return vec![Effect::Bell];
        };
        self.ex_command(doc, range, rest.trim_start())
    }

    /// `[range]` at the start of `text`: `%`, `addr`, `addr,addr`.
    fn parse_range<'a, B: Buffer>(
        &self,
        doc: &B,
        text: &'a str,
    ) -> Option<(Option<Lines>, &'a str)> {
        let last = Self::last_addressable(doc);
        if let Some(rest) = text.strip_prefix('%') {
            return Some((Some((0, last)), rest));
        }
        let (first, rest) = self.parse_address(doc, text)?;
        let Some(first) = first else {
            return Some((None, rest));
        };
        let (second, rest) = match rest.strip_prefix([',', ';']) {
            Some(after) => {
                let (second, rest) = self.parse_address(doc, after)?;
                (second.unwrap_or(first), rest)
            }
            None => (first, rest),
        };
        let clamp = |line: usize| line.saturating_sub(1).min(last);
        let (a, b) = (clamp(first), clamp(second));
        Some((Some((a.min(b), a.max(b))), rest))
    }

    /// One address as a 1-based line (0 means "above the first line"):
    /// `.`, `$`, `N`, `'x`, `/pat/`, each with `+N`/`-N` offsets.
    fn parse_address<'a, B: Buffer>(
        &self,
        doc: &B,
        text: &'a str,
    ) -> Option<(Option<usize>, &'a str)> {
        let here = self.cursor.line.saturating_add(1);
        let digits = |s: &str| s.chars().take_while(char::is_ascii_digit).count();
        let (mut base, mut rest) = match text.chars().next() {
            Some('.') => (Some(here), text.get(1..)?),
            Some('$') => (
                Some(Self::last_addressable(doc).saturating_add(1)),
                text.get(1..)?,
            ),
            Some('\'') => {
                let mark = text.get(1..)?.chars().next()?;
                let pos = self.mark_pos(mark)?;
                (
                    Some(pos.line.saturating_add(1)),
                    text.get(1usize.saturating_add(mark.len_utf8())..)?,
                )
            }
            Some('/') => {
                let body = text.get(1..)?;
                let end = body.find('/').unwrap_or(body.len());
                let re = self.search_regex(body.get(..end)?)?;
                let hit = self.find_match(
                    doc,
                    &re,
                    (
                        Cursor {
                            line: self.cursor.line,
                            col: usize::MAX,
                        },
                        true,
                    ),
                )?;
                (
                    Some(hit.line.saturating_add(1)),
                    body.get(end.saturating_add(1).min(body.len())..)?,
                )
            }
            Some(c) if c.is_ascii_digit() => {
                let n = digits(text);
                (text.get(..n)?.parse().ok(), text.get(n..)?)
            }
            Some('+' | '-') => (Some(here), text),
            _ => (None, text),
        };
        while let Some(sign) = rest.chars().next().filter(|c| matches!(c, '+' | '-')) {
            let after = rest.get(1..)?;
            let n = digits(after);
            let amount: usize = if n == 0 {
                1
            } else {
                after.get(..n)?.parse().ok()?
            };
            let line = base.unwrap_or(here);
            base = Some(if sign == '+' {
                line.saturating_add(amount)
            } else {
                line.saturating_sub(amount)
            });
            rest = after.get(n..)?;
        }
        Some((base, rest))
    }

    /// Dispatch a command name after its range.
    fn ex_command<B: Buffer>(
        &mut self,
        doc: &mut B,
        range: Option<Lines>,
        cmd: &str,
    ) -> Vec<Effect> {
        let here = (self.cursor.line, self.cursor.line);
        let lines = range.unwrap_or(here);
        let name_len = cmd.chars().take_while(char::is_ascii_alphabetic).count();
        let (name, arg) = cmd.split_at(name_len.min(cmd.len()));
        let ok = |ok: bool| if ok { Vec::new() } else { vec![Effect::Bell] };
        match name {
            "" if cmd.is_empty() => match range {
                Some((_, last)) => {
                    self.push_jump();
                    self.cursor = Cursor {
                        line: last,
                        col: Self::first_non_blank(doc, last),
                    };
                    self.preferred_col = self.cursor.col;
                    self.scroll_request = true;
                    self.clamp(doc);
                    Vec::new()
                }
                None => Vec::new(),
            },
            "s" | "su" | "substitute" => self.substitute(doc, lines, arg),
            "d" | "de" | "del" | "delete" => {
                self.delete_line_range(doc, lines.0, lines.1);
                self.clamp(doc);
                Vec::new()
            }
            "y" | "ya" | "yank" => {
                let text = Self::lines_text(doc, lines.0, lines.1);
                self.write_yank(text, true, None);
                Vec::new()
            }
            "m" | "mo" | "move" => ok(self.ex_move(doc, lines, (arg, true))),
            "t" | "co" | "copy" => ok(self.ex_move(doc, lines, (arg, false))),
            "j" | "join" => {
                let last = if lines.0 == lines.1 {
                    lines.1.saturating_add(1)
                } else {
                    lines.1
                };
                let last = last.min(Self::last_addressable(doc));
                if last > lines.0 {
                    self.join_lines(doc, (lines.0, last), !arg.starts_with('!'));
                }
                Vec::new()
            }
            "" if cmd.starts_with(['>', '<']) => {
                self.shift_lines(doc, (lines.0, lines.1), cmd.starts_with('<'));
                Vec::new()
            }
            "g" | "global" | "v" | "vglobal" => {
                let invert = name.starts_with('v') || arg.starts_with('!');
                let arg = arg.strip_prefix('!').unwrap_or(arg);
                self.ex_global(
                    doc,
                    range.unwrap_or((0, Self::last_addressable(doc))),
                    (arg, invert),
                )
            }
            "noh" | "nohlsearch" => Vec::new(),
            "e" | "edit" => match arg.trim() {
                "" => vec![Effect::Reload { force: false }],
                "!" => vec![Effect::Reload { force: true }],
                "#" | "! #" | "!#" => vec![Effect::AlternateFile],
                path => {
                    let path = path.strip_prefix('!').map_or(path, str::trim_start);
                    vec![Effect::EditFile(path.to_string())]
                }
            },
            _ if range.is_none() => Self::ex_effects(cmd),
            _ => vec![Effect::Bell],
        }
    }

    /// `:m {addr}` moves lines below line `addr` (0: to the top); `:t`
    /// copies them there.
    fn ex_move<B: Buffer>(
        &mut self,
        doc: &mut B,
        (first, last): Lines,
        (arg, remove): (&str, bool),
    ) -> bool {
        let Some((Some(target), _)) = self.parse_address(doc, arg.trim()) else {
            return false;
        };
        let target = target.min(Self::last_addressable(doc).saturating_add(1));
        // Moving into itself is an error; to its own place a no-op.
        if remove && target > first && target <= last {
            return false;
        }
        let text = Self::lines_text(doc, first, last);
        let count = last.saturating_sub(first).saturating_add(1);
        doc.begin_undo_group();
        let insert_at = if remove {
            self.remove_lines(doc, first, last);
            if target > last {
                target.saturating_sub(count)
            } else {
                target
            }
        } else {
            target
        };
        self.insert_lines_at(doc, insert_at, &text);
        doc.end_undo_group();
        doc.end_run();
        let landed = insert_at.saturating_add(count).saturating_sub(1);
        self.cursor = Cursor {
            line: landed,
            col: Self::first_non_blank(doc, landed),
        };
        self.clamp(doc);
        true
    }

    /// Insert `text` (lines, no final newline) as new lines starting at
    /// line index `at` (past the end: appended).
    pub(crate) fn insert_lines_at<B: Buffer>(&mut self, doc: &mut B, at: usize, text: &str) {
        let real = Self::last_addressable(doc).saturating_add(1);
        if at < real && at < doc.line_count() {
            doc.insert(doc.char_at_line_col(at, 0), &format!("{text}\n"));
        } else if doc.line_count() > real || doc.len_chars() == 0 {
            // Ends with a newline (or empty): append before the phantom line.
            doc.insert(doc.len_chars(), &format!("{text}\n"));
        } else {
            doc.insert(doc.len_chars(), &format!("\n{text}"));
        }
    }

    /// Delete lines `first..=last` without touching registers.
    pub(crate) fn remove_lines<B: Buffer>(&mut self, doc: &mut B, first: usize, last: usize) {
        let from = doc.char_at_line_col(first, 0);
        let next = last.saturating_add(1);
        let to = if next < doc.line_count() {
            doc.char_at_line_col(next, 0)
        } else {
            doc.len_chars()
        };
        doc.delete(from..to);
    }

    /// `:g/pat/cmd` (`:v` / `:g!` for non-matching lines): run `cmd` on
    /// each marked line, following line shifts as earlier runs edit.
    fn ex_global<B: Buffer>(
        &mut self,
        doc: &mut B,
        (first, last): Lines,
        (arg, invert): (&str, bool),
    ) -> Vec<Effect> {
        let mut chars = arg.chars();
        let Some(delim) = chars.next().filter(|c| !c.is_alphanumeric()) else {
            return vec![Effect::Bell];
        };
        let body = chars.as_str();
        let end = body.find(delim).unwrap_or(body.len());
        let pattern = body.get(..end).unwrap_or_default();
        let cmd = body
            .get(end.saturating_add(1).min(body.len())..)
            .unwrap_or_default()
            .trim();
        let pattern = if pattern.is_empty() {
            self.search_last.clone()
        } else {
            pattern.to_string()
        };
        let Some(re) = self.search_regex(&pattern) else {
            return vec![Effect::Bell];
        };
        self.search_last = pattern;
        let marked: Vec<usize> = (first..=last)
            .filter(|line| re.is_match(&doc.line_text(*line)) != invert)
            .collect();
        let cmd = if cmd.is_empty() { "p" } else { cmd };
        doc.begin_undo_group();
        let mut shift: isize = 0;
        for line in marked {
            let Some(line) = line.checked_add_signed(shift) else {
                continue;
            };
            let before = doc.line_count();
            self.cursor = Cursor { line, col: 0 };
            if cmd != "p" {
                let _ = self.ex_command(doc, Some((line, line)), cmd);
            }
            let after = doc.line_count();
            shift = shift.saturating_add(
                isize::try_from(after)
                    .unwrap_or(0)
                    .saturating_sub(isize::try_from(before).unwrap_or(0)),
            );
        }
        doc.end_undo_group();
        doc.end_run();
        self.clamp(doc);
        Vec::new()
    }
}
