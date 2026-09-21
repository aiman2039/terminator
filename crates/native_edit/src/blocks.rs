//! Rendered-block model for Markdown live preview editing.
//!
//! [`parse_blocks`] maps a [`Buffer`](super::doc::Buffer) to styled blocks
//! with line ranges, so the rendered pane can offer click-to-edit per block
//! while the source view edits the underlying lines. Rendering itself stays
//! with the existing `egui_commonmark` preview until session plumbing lands;
//! this module owns the structure clicks resolve against.

use super::doc::Buffer;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockKind {
    Heading(u8),
    Paragraph,
    Bullet,
    Numbered,
    Quote,
    Code,
    Blank,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// First grid line of the block (0-based, inclusive).
    pub start_line: usize,
    /// Last grid line of the block (inclusive).
    pub end_line: usize,
}

impl Block {
    #[must_use]
    pub fn contains(&self, line: usize) -> bool {
        line >= self.start_line && line <= self.end_line
    }
}

fn fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

fn heading_level(line: &str) -> Option<u8> {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ') {
        Some(hashes as u8)
    } else {
        None
    }
}

fn list_kind(line: &str) -> Option<BlockKind> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        return Some(BlockKind::Bullet);
    }
    let mut digits = 0;
    for b in trimmed.bytes() {
        if b.is_ascii_digit() {
            digits += 1;
        } else {
            break;
        }
    }
    if digits > 0 && trimmed.as_bytes().get(digits..digits + 2) == Some(b". ") {
        return Some(BlockKind::Numbered);
    }
    None
}

/// Parse buffer lines into rendered blocks. Paragraphs merge consecutive
/// plain lines; fenced code spans to the closing fence (or end of buffer).
pub fn parse_blocks<B: Buffer>(doc: &B) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut line = 0;
    let count = doc.line_count();
    while line < count {
        let text = doc.line_text(line);
        let trimmed = text.trim();
        if fence(&text) {
            let start = line;
            line += 1;
            while line < count && !fence(&doc.line_text(line)) {
                line += 1;
            }
            let end = line.min(count.saturating_sub(1));
            blocks.push(Block {
                kind: BlockKind::Code,
                start_line: start,
                end_line: end.max(start),
            });
            line += 1;
            continue;
        }
        if trimmed.is_empty() {
            blocks.push(Block {
                kind: BlockKind::Blank,
                start_line: line,
                end_line: line,
            });
            line += 1;
            continue;
        }
        if let Some(level) = heading_level(trimmed) {
            blocks.push(Block {
                kind: BlockKind::Heading(level),
                start_line: line,
                end_line: line,
            });
            line += 1;
            continue;
        }
        if trimmed.starts_with("> ") || trimmed == ">" {
            blocks.push(Block {
                kind: BlockKind::Quote,
                start_line: line,
                end_line: line,
            });
            line += 1;
            continue;
        }
        if let Some(kind) = list_kind(&text) {
            blocks.push(Block {
                kind,
                start_line: line,
                end_line: line,
            });
            line += 1;
            continue;
        }
        let start = line;
        while line < count {
            let text = doc.line_text(line);
            if text.trim().is_empty()
                || fence(&text)
                || heading_level(text.trim()).is_some()
                || text.trim().starts_with("> ")
                || list_kind(&text).is_some()
            {
                break;
            }
            line += 1;
        }
        blocks.push(Block {
            kind: BlockKind::Paragraph,
            start_line: start,
            end_line: line.saturating_sub(1).max(start),
        });
    }
    blocks
}

/// Find the block covering a grid line (e.g. from a rendered-pane click).
#[must_use]
pub fn block_at(blocks: &[Block], line: usize) -> Option<&Block> {
    blocks.iter().find(|block| block.contains(line))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;

    fn parse(text: &str) -> Vec<Block> {
        parse_blocks(&Doc::new(text))
    }

    #[test]
    fn headings_paragraphs_and_lists_classify() {
        let blocks = parse("# Title\n\nHello *world*\ncontinues\n\n- a\n1. b\n");
        let kinds: Vec<BlockKind> = blocks.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BlockKind::Heading(1),
                BlockKind::Blank,
                BlockKind::Paragraph,
                BlockKind::Blank,
                BlockKind::Bullet,
                BlockKind::Numbered,
                BlockKind::Blank,
            ]
        );
        assert_eq!((blocks[2].start_line, blocks[2].end_line), (2, 3));
    }

    #[test]
    fn code_fence_spans_to_close_or_end() {
        let blocks = parse("```rust\nlet x = 1;\n```\nafter\n");
        assert_eq!(blocks[0].kind, BlockKind::Code);
        assert_eq!((blocks[0].start_line, blocks[0].end_line), (0, 2));
        let open = parse("```\nunclosed\n");
        assert_eq!((open[0].start_line, open[0].end_line), (0, 2));
    }

    #[test]
    fn quotes_classify_per_line() {
        let blocks = parse("> one\n> two\n");
        assert!(blocks.iter().take(2).all(|b| b.kind == BlockKind::Quote));
    }

    #[test]
    fn block_at_resolves_clicks() {
        let blocks = parse("# T\n\npara one\npara two\n");
        let block = block_at(&blocks, 3).expect("block");
        assert_eq!(block.kind, BlockKind::Paragraph);
        assert!(block_at(&blocks, 99).is_none());
    }

    #[test]
    fn empty_document_parses_clean() {
        assert_eq!(
            parse(""),
            vec![Block {
                kind: BlockKind::Blank,
                start_line: 0,
                end_line: 0,
            }]
        );
    }
}
