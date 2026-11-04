//! End-of-line marks on changed rows: trailing whitespace, a carriage
//! return, and a missing final newline (`\ No newline at end of file`).
use super::*;

pub(in super::super) struct PaintLineEndInput<'a> {
    pub(in super::super) ui: &'a egui::Ui,
    /// Top-left of the code text.
    pub(in super::super) pos: egui::Pos2,
    pub(in super::super) code: &'a egui::Galley,
    pub(in super::super) line: &'a diff::DiffLine,
    pub(in super::super) colors: DiffColors,
    pub(in super::super) font: &'a egui::FontId,
}

/// Columns of trailing spaces and tabs across the line's `pieces`:
/// `(first, end)` in chars, or `None` when the line has none.
pub(in super::super) fn trailing_whitespace<'a>(
    pieces: impl DoubleEndedIterator<Item = &'a str> + Clone,
) -> Option<(usize, usize)> {
    let trailing = pieces
        .clone()
        .rev()
        .flat_map(|piece| piece.chars().rev())
        .take_while(|ch| matches!(ch, ' ' | '\t'))
        .count();
    let end: usize = pieces.map(|piece| piece.chars().count()).sum();
    (trailing > 0).then(|| (end.saturating_sub(trailing), end))
}

pub(in super::super) fn paint_line_end(input: PaintLineEndInput<'_>) {
    let PaintLineEndInput {
        ui,
        pos,
        code,
        line,
        colors,
        font,
    } = input;
    let color = match line.kind {
        diff::LineKind::Insert => colors.added,
        diff::LineKind::Delete => colors.deleted,
        _ => return,
    };
    let painter = ui.painter();
    let height = code.size().y;
    let pieces = line.spans.iter().map(|span| span.text.as_str());
    if let Some((first, end)) = trailing_whitespace(pieces) {
        let x0 = code.pos_from_cursor(egui::text::CCursor::new(first)).min.x;
        let x1 = code.pos_from_cursor(egui::text::CCursor::new(end)).min.x;
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(x0, 0.0), egui::pos2(x1, height))
                .translate(pos.to_vec2()),
            0.0,
            tint(color, 110),
        );
    }
    let weak = colors.text.gamma_multiply(0.5);
    let mut x = pos.x + code.size().x + 6.0;
    if line.end.cr {
        let rect = painter.text(
            egui::pos2(x, pos.y),
            egui::Align2::LEFT_TOP,
            "CR",
            font.clone(),
            color,
        );
        painter.rect_stroke(
            rect.expand(1.0),
            2.0,
            egui::Stroke::new(1.0, tint(color, 160)),
            egui::StrokeKind::Outside,
        );
        x = rect.right() + 8.0;
    }
    if line.end.no_newline {
        painter.text(
            egui::pos2(x, pos.y),
            egui::Align2::LEFT_TOP,
            "\\ No newline at end of file",
            font.clone(),
            weak,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::trailing_whitespace;

    #[test]
    fn trailing_spaces_and_tabs_are_found_by_column() {
        assert_eq!(
            trailing_whitespace(["let a = 1;  \t"].into_iter()),
            Some((10, 13))
        );
        assert_eq!(trailing_whitespace(["ünï "].into_iter()), Some((3, 4)));
        assert_eq!(trailing_whitespace(["clean"].into_iter()), None);
        assert_eq!(
            trailing_whitespace(["a ", "\t", ""].into_iter()),
            Some((1, 3))
        );
        assert_eq!(trailing_whitespace(["a ", "b"].into_iter()), None);
    }
}
