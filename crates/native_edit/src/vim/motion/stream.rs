//! Lazy char access over a [`Buffer`]: one line is cached at a time, so
//! motions never copy the whole buffer.
use super::super::super::doc::Buffer;
use super::super::words::CharSource;

pub(crate) struct Stream<'a, B> {
    doc: &'a B,
    len: usize,
    start: usize,
    chars: Vec<char>,
    loaded: bool,
}

impl<'a, B: Buffer> Stream<'a, B> {
    pub(crate) fn new(doc: &'a B) -> Self {
        Self {
            doc,
            len: doc.len_chars(),
            start: 0,
            chars: Vec::new(),
            loaded: false,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }
}

impl<B: Buffer> CharSource for Stream<'_, B> {
    fn at(&mut self, i: usize) -> Option<char> {
        if i >= self.len {
            return None;
        }
        let end = self.start.saturating_add(self.chars.len());
        if !self.loaded || i < self.start || i > end {
            let (line, _) = self.doc.line_col_at(i);
            self.start = self.doc.char_at_line_col(line, 0);
            self.chars = self.doc.line_text(line).chars().collect();
            self.loaded = true;
        }
        // One past the line's chars is its newline.
        Some(
            self.chars
                .get(i.saturating_sub(self.start))
                .copied()
                .unwrap_or('\n'),
        )
    }
}
