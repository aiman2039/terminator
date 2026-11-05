//! Screen-relative motions (`H`/`M`/`L`) and scrolling (Ctrl-D/U/F/B/E/Y,
//! `zz`/`zt`/`zb`). The view reports what it shows and applies the scroll.
use super::super::super::doc::Buffer;
use super::super::mode::Cursor;
use super::super::state::{ViewScroll, VimEngine};
use super::Mo;
use super::{MKind, Motion};

impl VimEngine {
    /// First and last shown lines, clamped to the buffer; the whole
    /// buffer before the view reported anything.
    fn shown<B: Buffer>(&self, doc: &B) -> (usize, usize) {
        let last = Self::last_addressable(doc);
        let view = self.x.viewport;
        if view.bottom == 0 && view.top == 0 {
            return (0, last);
        }
        (
            view.top.min(last),
            view.bottom.min(last).max(view.top.min(last)),
        )
    }

    pub(super) fn screen_motion<B: Buffer>(&self, doc: &B, mo: Mo, count: usize) -> Motion {
        let (top, bottom) = self.shown(doc);
        let off = count.saturating_sub(1);
        let line = match mo {
            Mo::ScreenTop => top.saturating_add(off).min(bottom),
            Mo::ScreenBottom => bottom.saturating_sub(off).max(top),
            _ => top.saturating_add(bottom.saturating_sub(top) / 2),
        };
        let col = Self::first_non_blank(doc, line);
        Motion::new(Cursor { line, col }, MKind::Linewise).jump()
    }

    /// Ctrl-D/U (half page), Ctrl-F/B (page), Ctrl-E/Y (one line): the
    /// view scrolls and the cursor follows.
    pub(crate) fn scroll_key<B: Buffer>(
        &mut self,
        doc: &B,
        key: char,
        (count, typed): (usize, bool),
    ) {
        let (top, bottom) = self.shown(doc);
        let rows = bottom.saturating_sub(top).saturating_add(1);
        let last = Self::last_addressable(doc);
        let half = if typed { count } else { (rows / 2).max(1) };
        let page = rows.saturating_sub(2).max(1).saturating_mul(count);
        let (lines, line): (isize, usize) = match key {
            'd' => (signed(half), self.cursor.line.saturating_add(half)),
            'u' => (
                signed(half).saturating_neg(),
                self.cursor.line.saturating_sub(half),
            ),
            'f' => (signed(page), top.saturating_add(page)),
            'b' => (signed(page).saturating_neg(), bottom.saturating_sub(page)),
            'e' => (
                signed(count),
                self.cursor.line.max(top.saturating_add(count)),
            ),
            _ => (
                signed(count).saturating_neg(),
                self.cursor.line.min(bottom.saturating_sub(count)),
            ),
        };
        self.x.view_scroll = Some(ViewScroll::Lines(lines));
        self.cursor.line = line.min(last);
        self.cursor.col = self.preferred_col;
        self.clamp(doc);
    }

    /// `zz` / `zt` / `zb` (and `z.`, `z<CR>`, `z-`): place the cursor row.
    pub(crate) fn z_key<B: Buffer>(&mut self, doc: &B, key: super::super::mode::Key) -> bool {
        use super::super::mode::Key;
        let (scroll, first_non_blank) = match key {
            Key::Char('z') => (ViewScroll::Center, false),
            Key::Char('t') => (ViewScroll::Top, false),
            Key::Char('b') => (ViewScroll::Bottom, false),
            Key::Char('.') => (ViewScroll::Center, true),
            Key::Enter => (ViewScroll::Top, true),
            Key::Char('-') => (ViewScroll::Bottom, true),
            _ => return false,
        };
        if first_non_blank {
            self.cursor.col = Self::first_non_blank(doc, self.cursor.line);
            self.preferred_col = self.cursor.col;
        }
        self.x.view_scroll = Some(scroll);
        true
    }

    /// Report the shown lines (the view calls this every frame).
    pub fn set_viewport(&mut self, top: usize, bottom: usize) {
        self.x.viewport = super::super::state::Viewport {
            top,
            bottom,
            ..self.x.viewport
        };
    }

    /// Report the line of the row at the top edge, rounded to the nearest
    /// row, so a restored scroll position reads back unchanged.
    pub fn set_scroll_line(&mut self, line: usize) {
        self.x.viewport.scroll = line;
    }

    /// The line at the top edge as the view last reported it (0 before).
    #[must_use]
    pub fn scroll_line(&self) -> usize {
        self.x.viewport.scroll
    }

    /// Take a pending view scroll (`zz`, Ctrl-D, …) for the view to apply.
    pub fn take_view_scroll(&mut self) -> Option<ViewScroll> {
        self.x.view_scroll.take()
    }
}

fn signed(n: usize) -> isize {
    isize::try_from(n).unwrap_or(isize::MAX)
}
