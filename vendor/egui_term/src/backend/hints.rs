//! Every link target on screen, for keyboard hints. Uses the same token
//! rules as pointer hover ([`super::target_at_content`]).
use super::{target_at_content, LinkTarget, RenderableContent, TerminalBackend};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;

impl TerminalBackend {
    /// The tokens in view that pointer hover would also find, top to
    /// bottom. Rects are in pane coordinates. No filesystem access.
    pub fn visible_targets(&self) -> Vec<LinkTarget> {
        visible_targets(self.last_content())
    }
}

pub(super) fn visible_targets(content: &RenderableContent) -> Vec<LinkTarget> {
    let grid = &content.grid;
    let size = &content.terminal_size;
    let (width, height) = (f32::from(size.cell_width), f32::from(size.cell_height));
    if width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    let mut out: Vec<LinkTarget> = Vec::new();
    let last = grid.last_column().0;
    for row in 0..grid.screen_lines() {
        let line = Line(row as i32);
        for column in 0..=last {
            let cell = &grid[Point::new(line, Column(column))];
            let blank = |c: char| c.is_whitespace() || c == '\0';
            if blank(cell.c) || cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let after_blank = match column.checked_sub(1) {
                Some(before) => blank(grid[Point::new(line, Column(before))].c),
                None => {
                    row == 0
                        || !grid[Point::new(line - 1, grid.last_column())]
                            .flags
                            .contains(Flags::WRAPLINE)
                }
            };
            if !after_blank {
                continue;
            }
            // A leading bracket or quote is not part of the token: try the
            // next cell too.
            let found = [column, column + 1].into_iter().find_map(|at| {
                let x = (at as f32 + 0.5) * width;
                let y = (row as f32 + 0.5) * height;
                target_at_content(content, x, y)
            });
            let Some(target) = found else {
                continue;
            };
            let first = target.rects.first().copied();
            if !out
                .iter()
                .any(|seen| seen.text == target.text && seen.rects.first().copied() == first)
            {
                out.push(target);
            }
        }
    }
    out
}
