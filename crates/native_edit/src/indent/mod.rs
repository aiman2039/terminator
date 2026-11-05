//! Document indentation: tabs or spaces, the indent width and the tab
//! width. Each document has one setting ([`crate::doc::Buffer::indent_style`]);
//! typing, indent commands, LSP formatting, tab stops and indent guides
//! follow it. A file's setting comes from `.editorconfig`
//! ([`EditorConfig`]), else from the text it already holds.
mod editorconfig;
#[cfg(test)]
mod tests;

pub use editorconfig::{EditorConfig, EndOfLine};

use crate::lsp::FormatOptions;
use crate::vim::VimrcOptions;
use std::path::Path;

/// Indent and tab width when nothing else sets them.
pub const DEFAULT_WIDTH: usize = 4;

/// Lines [`IndentStyle::detect`] reads from the top of a file.
const DETECT_LINES: usize = 2000;

/// Widest indent or tab width a setting may ask for; wider values (and
/// zero) are ignored.
const MAX_WIDTH: usize = 32;

/// How one document indents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndentStyle {
    /// One tab per indent level; else `width` spaces.
    pub tabs: bool,
    /// Columns of one level of space indentation.
    pub width: usize,
    /// Columns a tab spans: the tab stops, and one level with `tabs`.
    pub tab_width: usize,
}

impl Default for IndentStyle {
    fn default() -> Self {
        Self::spaces(DEFAULT_WIDTH)
    }
}

impl IndentStyle {
    /// `width` spaces per level; tabs show [`DEFAULT_WIDTH`] wide.
    #[must_use]
    pub const fn spaces(width: usize) -> Self {
        Self {
            tabs: false,
            width,
            tab_width: DEFAULT_WIDTH,
        }
    }

    /// One tab per level, shown `tab_width` wide.
    #[must_use]
    pub const fn tabs(tab_width: usize) -> Self {
        Self {
            tabs: true,
            width: tab_width,
            tab_width,
        }
    }

    /// Columns of one indent level (at least 1).
    #[must_use]
    pub fn level(self) -> usize {
        let columns = if self.tabs {
            self.tab_width
        } else {
            self.width
        };
        columns.max(1)
    }

    /// Columns of a tab stop (at least 1).
    #[must_use]
    pub fn tab_cells(self) -> usize {
        self.tab_width.max(1)
    }

    /// One indent level as text: a tab, or one level of spaces.
    #[must_use]
    pub fn unit(self) -> String {
        if self.tabs {
            "\t".to_owned()
        } else {
            " ".repeat(self.level())
        }
    }

    /// What Tab inserts at display column `column`: a tab, or the spaces
    /// up to the next indent stop.
    #[must_use]
    pub fn tab_text(self, column: usize) -> String {
        if self.tabs {
            return "\t".to_owned();
        }
        let level = self.level();
        let past = column.checked_rem(level).unwrap_or(0);
        " ".repeat(level.saturating_sub(past))
    }

    /// Chars one outdent removes from a line starting with `text`: a
    /// leading tab, else up to one level of spaces.
    #[must_use]
    pub fn outdent_width(self, text: &str) -> usize {
        if text.starts_with('\t') {
            1
        } else {
            text.chars()
                .take(self.level())
                .take_while(|c| *c == ' ')
                .count()
        }
    }

    /// `FormattingOptions` for language-server format requests.
    #[must_use]
    pub fn format_options(self) -> FormatOptions {
        FormatOptions {
            tab_size: u32::try_from(self.level()).unwrap_or(u32::MAX),
            insert_spaces: !self.tabs,
        }
    }

    /// Indentation `lines` already use, from the first 2000: tabs when
    /// more lines start with a tab than with spaces, else the step (2, 4
    /// or 8 spaces) that most often opens a deeper line. Steps back out
    /// are not counted: they may close several levels, or end an
    /// aligned continuation line. `None` when nothing is indented.
    pub fn detect<I>(lines: I) -> Option<Self>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let (mut tabs, mut spaces, mut previous) = (0usize, 0usize, 0usize);
        let mut steps = [0usize; 9];
        for line in lines.into_iter().take(DETECT_LINES) {
            let text = line.as_ref();
            if text.starts_with('\t') {
                tabs = tabs.saturating_add(1);
                continue;
            }
            let width = text.chars().take_while(|c| *c == ' ').count();
            if !text.chars().nth(width).is_some_and(|c| !c.is_whitespace()) {
                // Blank, or spaces before a tab: no step to read.
                continue;
            }
            if width >= 2 {
                spaces = spaces.saturating_add(1);
            }
            if let Some(count) = width
                .checked_sub(previous)
                .and_then(|step| steps.get_mut(step))
            {
                *count = count.saturating_add(1);
            }
            previous = width;
        }
        if tabs == 0 && spaces == 0 {
            return None;
        }
        if tabs > spaces {
            return Some(Self::tabs(DEFAULT_WIDTH));
        }
        let count = |step: usize| steps.get(step).copied().unwrap_or(0);
        let mut width = DEFAULT_WIDTH;
        for step in [2, 8] {
            if count(step) > count(width) {
                width = step;
            }
        }
        Some(Self::spaces(width))
    }

    /// The language default for a file with no indentation yet: tabs for
    /// Go and Makefiles, `None` for everything else.
    #[must_use]
    pub fn for_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?.to_ascii_lowercase();
        let ext = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let tabs = matches!(
            name.as_str(),
            "makefile" | "gnumakefile" | "go.mod" | "go.work"
        ) || matches!(ext.as_str(), "go" | "mk" | "mak");
        tabs.then_some(Self::tabs(DEFAULT_WIDTH))
    }

    /// The `.vimrc` indentation (`shiftwidth`, `tabstop`, `expandtab`);
    /// `None` when the vimrc sets none of them.
    #[must_use]
    pub fn from_vimrc(options: &VimrcOptions) -> Option<Self> {
        let size = |value: Option<u32>| {
            value
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| (1..=MAX_WIDTH).contains(n))
        };
        let (shift, stop) = (size(options.shiftwidth), size(options.tabstop));
        if shift.is_none() && stop.is_none() && options.expandtab.is_none() {
            return None;
        }
        let tabs = options.expandtab == Some(false);
        let width = shift.or(stop).unwrap_or(DEFAULT_WIDTH);
        Some(Self {
            tabs,
            width,
            tab_width: stop.unwrap_or(if tabs { width } else { DEFAULT_WIDTH }),
        })
    }
}
