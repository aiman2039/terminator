//! `.editorconfig` for one file, read through `ec4rs`: its indentation and
//! the rules a save applies (final newline, trailing whitespace, line
//! breaks of a new file).
use super::{IndentStyle, MAX_WIDTH};
use crate::doc::Buffer;
use ec4rs::property::{FinalNewline, IndentSize, TabWidth, TrimTrailingWs};
use std::ops::Range;
use std::path::Path;

/// Line breaks `end_of_line` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndOfLine {
    Lf,
    Crlf,
    Cr,
}

/// What the `.editorconfig` files above one file set for it: `None` (or
/// `false`) where they set nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditorConfig {
    /// `indent_style`: `tab` or `space`.
    pub tabs: Option<bool>,
    /// `indent_size`.
    pub width: Option<usize>,
    /// `tab_width` (defaults to `indent_size`, as the spec says).
    pub tab_width: Option<usize>,
    pub end_of_line: Option<EndOfLine>,
    /// `insert_final_newline = true`.
    pub final_newline: bool,
    /// `trim_trailing_whitespace = true`.
    pub trim_trailing_whitespace: bool,
}

impl EditorConfig {
    /// Read the `.editorconfig` files from `path`'s folder up to the
    /// first `root = true`. Blocking file IO: call it off the GUI thread.
    /// A missing or broken file reads as nothing set.
    #[must_use]
    pub fn for_file(path: &Path) -> Self {
        let Ok(mut props) = ec4rs::properties_of(path) else {
            return Self::default();
        };
        props.use_fallbacks();
        let size = |value: usize| (1..=MAX_WIDTH).contains(&value).then_some(value);
        Self {
            tabs: props
                .get::<ec4rs::property::IndentStyle>()
                .ok()
                .map(|style| style == ec4rs::property::IndentStyle::Tabs),
            width: match props.get::<IndentSize>() {
                Ok(IndentSize::Value(value)) => size(value),
                _ => None,
            },
            tab_width: match props.get::<TabWidth>() {
                Ok(TabWidth::Value(value)) => size(value),
                Err(_) => None,
            },
            end_of_line: props
                .get::<ec4rs::property::EndOfLine>()
                .ok()
                .map(|eol| match eol {
                    ec4rs::property::EndOfLine::Lf => EndOfLine::Lf,
                    ec4rs::property::EndOfLine::CrLf => EndOfLine::Crlf,
                    ec4rs::property::EndOfLine::Cr => EndOfLine::Cr,
                }),
            final_newline: matches!(props.get::<FinalNewline>(), Ok(FinalNewline::Value(true))),
            trim_trailing_whitespace: matches!(
                props.get::<TrimTrailingWs>(),
                Ok(TrimTrailingWs::Value(true))
            ),
        }
    }

    /// The file's indentation: the fields set here, the rest from what
    /// `text` already uses, else tabs for Go and Makefiles, else
    /// `fallback` (the `.vimrc`), else four spaces.
    #[must_use]
    pub fn indent(&self, path: &Path, text: &str, fallback: Option<IndentStyle>) -> IndentStyle {
        let base = IndentStyle::detect(text.lines())
            .or_else(|| IndentStyle::for_path(path))
            .or(fallback)
            .unwrap_or_default();
        IndentStyle {
            tabs: self.tabs.unwrap_or(base.tabs),
            width: self.width.unwrap_or(base.width),
            tab_width: self.tab_width.unwrap_or(base.tab_width),
        }
    }

    /// Buffer edits the save rules make, last first (each one leaves the
    /// positions before it valid): spaces and tabs off the end of every
    /// line not in `keep` (`trim_trailing_whitespace`), and a newline
    /// after a last line that has none (`insert_final_newline`).
    #[must_use]
    pub fn save_edits<B: Buffer>(&self, doc: &B, keep: &[usize]) -> Vec<(Range<usize>, String)> {
        let mut edits = Vec::new();
        let last = doc.line_count().saturating_sub(1);
        if self.final_newline && !doc.is_empty() && !doc.line_text(last).is_empty() {
            let end = doc.len_chars();
            edits.push((end..end, "\n".to_owned()));
        }
        if !self.trim_trailing_whitespace {
            return edits;
        }
        for line in (0..doc.line_count())
            .rev()
            .filter(|line| !keep.contains(line))
        {
            let text = doc.line_text(line);
            let len = text.chars().count();
            let kept = text.trim_end_matches([' ', '\t']).chars().count();
            if kept < len {
                let from = doc.char_at_line_col(line, kept);
                edits.push((from..doc.char_at_line_col(line, len), String::new()));
            }
        }
        edits
    }
}
