//! `:s` / `:%s` substitution.
use super::super::doc::Buffer;
use super::mode::{Cursor, Effect};
use super::operators::find_matches;
use super::state::VimEngine;

/// A parsed `:s/pat/rep/[g]`, before the pattern falls back to the last
/// search.
struct SubstituteSpec {
    /// `%`: every line, not just the cursor line.
    all: bool,
    pattern: String,
    replacement: String,
    global: bool,
}

/// One replacement in buffer char offsets, collected against the original
/// text.
struct SubstituteEdit {
    line: usize,
    from: usize,
    to: usize,
    text: String,
}

impl VimEngine {
    /// `:s/pat/rep/[g]` on the cursor line, `:%s/pat/rep/[g]` on the
    /// buffer. Patterns are literal (like `/` search); an empty pattern
    /// reuses the last search. `&` in the replacement splices the match,
    /// `\n` a newline. One undo unit; unknown flags and zero matches
    /// bell. Never `.`-repeatable (ex commands aren't).
    pub(super) fn substitute<B: Buffer>(&mut self, doc: &mut B, command: &str) -> Vec<Effect> {
        let Some(mut spec) = Self::parse_substitute(command) else {
            return vec![Effect::Bell];
        };
        if spec.pattern.is_empty() {
            spec.pattern.clone_from(&self.search_last);
        }
        if spec.pattern.is_empty() || spec.pattern.contains('\n') {
            return vec![Effect::Bell];
        }
        self.search_last.clone_from(&spec.pattern);
        let edits = self.substitute_edits(doc, &spec);
        let Some(first) = edits.first() else {
            return vec![Effect::Bell];
        };
        doc.begin_undo_group();
        for edit in edits.iter().rev() {
            doc.delete(edit.from..edit.to);
            if !edit.text.is_empty() {
                doc.insert(edit.from, &edit.text);
            }
        }
        doc.end_undo_group();
        self.cursor = Cursor {
            line: first.line,
            col: first
                .from
                .saturating_sub(doc.char_at_line_col(first.line, 0)),
        };
        self.scroll_request = true;
        self.clamp(doc);
        doc.end_run();
        Vec::new()
    }

    /// Split `[%]s<delim>pat<delim>rep[<delim>flags]` on unescaped
    /// delimiters (`\X` escapes to `X`); `None` for malformed commands and
    /// flags other than `g`.
    fn parse_substitute(command: &str) -> Option<SubstituteSpec> {
        let all = command.starts_with('%');
        let mut body = command.strip_prefix('%').unwrap_or(command);
        body = body.strip_prefix('s').unwrap_or(body);
        let mut chars = body.chars();
        let delim = chars.next()?;
        let mut parts: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut escaped = false;
        for c in chars {
            if escaped {
                current.push(match c {
                    'n' => '\n',
                    't' => '\t',
                    _ => c,
                });
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == delim {
                parts.push(std::mem::take(&mut current));
            } else {
                current.push(c);
            }
        }
        // A trailing backslash escapes nothing.
        if escaped {
            return None;
        }
        parts.push(current);
        if parts.len() < 2 || parts.len() > 3 {
            return None;
        }
        let flags = parts.get(2).map_or("", String::as_str);
        if !flags.chars().all(|c| c == 'g') {
            return None;
        }
        let global = flags.contains('g');
        let mut parts = parts.into_iter();
        Some(SubstituteSpec {
            all,
            pattern: parts.next()?,
            replacement: parts.next()?,
            global,
        })
    }

    /// Every replacement, in line order, against the current text.
    fn substitute_edits<B: Buffer>(&self, doc: &B, spec: &SubstituteSpec) -> Vec<SubstituteEdit> {
        let lines: Vec<usize> = if spec.all {
            (0..doc.line_count()).collect()
        } else {
            vec![self.cursor.line.min(doc.line_count().saturating_sub(1))]
        };
        let ignorecase = self.ignorecase();
        let mut edits = Vec::new();
        for line in lines {
            let text = doc.line_text(line);
            let base = doc.char_at_line_col(line, 0);
            let matches = find_matches(&text, &spec.pattern, ignorecase);
            let take = if spec.global { matches.len() } else { 1 };
            for (byte, len) in matches.into_iter().take(take) {
                let col = text.get(..byte).map_or(0, |p| p.chars().count());
                let matched = text.get(byte..byte.saturating_add(len)).unwrap_or("");
                edits.push(SubstituteEdit {
                    line,
                    from: base.saturating_add(col),
                    to: base.saturating_add(col.saturating_add(matched.chars().count())),
                    text: Self::expand_replacement(&spec.replacement, matched),
                });
            }
        }
        edits
    }

    /// The splitter already resolved escapes, so a bare `&` always means
    /// the whole match.
    fn expand_replacement(replacement: &str, matched: &str) -> String {
        let mut rep = String::new();
        for c in replacement.chars() {
            if c == '&' {
                rep.push_str(matched);
            } else {
                rep.push(c);
            }
        }
        rep
    }
}
