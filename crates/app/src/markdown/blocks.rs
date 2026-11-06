//! Preview virtualization: the document splits into its top-level Markdown
//! blocks, each block remembers its painted height, and blocks far outside
//! the view only take that height. A long README then lays out only what
//! is near the view, while the scroll range (and split scroll sync) stays
//! the same.
use pulldown_cmark::{Event, Options, Parser, Tag};
use std::collections::HashMap;
use std::ops::Range;

/// Top-level blocks of `text` as contiguous byte ranges that cover it all
/// (blank lines go with the block after them). One block for the whole
/// text when blocks cannot render apart: reference-style link definitions
/// and footnotes resolve across the document.
pub(super) fn split_blocks(text: &str) -> Vec<Range<usize>> {
    let whole = || std::iter::once(0..text.len()).collect::<Vec<_>>();
    let mut parser = Parser::new_ext(text, Options::all()).into_offset_iter();
    let mut ends = Vec::new();
    let mut depth = 0usize;
    for (event, range) in parser.by_ref() {
        match event {
            Event::Start(Tag::FootnoteDefinition(_)) | Event::FootnoteReference(_) => {
                return whole();
            }
            Event::Start(_) => depth = depth.saturating_add(1),
            Event::End(_) => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    ends.push(range.end);
                }
            }
            // A top-level leaf such as a rule.
            _ if depth == 0 => ends.push(range.end),
            _ => {}
        }
    }
    if parser.reference_definitions().iter().next().is_some() {
        return whole();
    }
    let mut blocks = Vec::with_capacity(ends.len());
    let mut start = 0;
    for end in ends {
        if end > start {
            blocks.push(start..end);
            start = end;
        }
    }
    match blocks.last_mut() {
        Some(last) => last.end = text.len(),
        None => return whole(),
    }
    blocks
}

/// Painted block heights, for one document and one width.
#[derive(Default)]
pub(super) struct BlockHeights {
    key: (u64, usize),
    heights: HashMap<usize, f32>,
}

impl BlockHeights {
    /// Forget every height when the document or the width changed.
    pub(super) fn begin(&mut self, document: u64, width: f32) {
        let key = (document, super::html::usize_from_f32(width.round()));
        if self.key != key {
            self.key = key;
            self.heights.clear();
        }
    }

    /// The height of the block starting at `start`, once it was painted.
    pub(super) fn get(&self, start: usize) -> Option<f32> {
        self.heights.get(&start).copied()
    }

    pub(super) fn set(&mut self, start: usize, height: f32) {
        self.heights.insert(start, height);
    }
}

#[cfg(test)]
mod tests {
    use super::split_blocks;

    fn pieces(text: &str) -> Vec<&str> {
        split_blocks(text)
            .into_iter()
            .map(|range| text.get(range).unwrap_or_default())
            .collect()
    }

    #[test]
    fn top_level_blocks_cover_the_whole_text() {
        let text =
            "# Title\n\nA paragraph\nwith two lines.\n\n- one\n- two\n\n---\n\n```\ncode\n```\n";
        let parts = pieces(text);
        assert_eq!(parts.concat(), text);
        assert_eq!(parts.len(), 5, "{parts:?}");
        assert!(parts[2].contains("- one\n- two"), "a list stays one block");
    }

    #[test]
    fn reference_links_and_footnotes_keep_one_block() {
        let reference = "See [the docs][d].\n\nMore.\n\n[d]: https://example.com\n";
        assert_eq!(split_blocks(reference).len(), 1);
        let footnote = "Text[^1].\n\nMore.\n\n[^1]: Note.\n";
        assert_eq!(split_blocks(footnote).len(), 1);
    }
}
