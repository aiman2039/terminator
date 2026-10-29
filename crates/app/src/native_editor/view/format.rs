//! Markdown formatting buttons: heading level, bold, italic and bullet list.
use super::*;
use terminator_native_edit::vim::Mode;

/// One formatting action from the header buttons.
#[derive(Clone, Copy)]
enum Format {
    /// Set the caret line's heading level; 0 makes it body text.
    Heading(usize),
    /// Wrap the selection (or an empty pair at the caret) in a marker.
    Wrap(&'static str),
    /// Add or remove a `- ` bullet on the caret line.
    List,
}

impl ViewPass<'_> {
    pub(super) fn markdown_format(&mut self, ui: &mut egui::Ui) {
        if self.read_only() || self.doc.markdown_mode == markdown::Mode::Preview {
            return;
        }
        // The header lays out right to left; draw in reverse there so the
        // buttons still read Heading, Bold, Italic, List.
        let mut buttons: [fn(&mut egui::Ui) -> Option<Format>; 4] =
            [heading_button, bold_button, italic_button, list_button];
        if ui.layout().prefer_right_to_left() {
            buttons.reverse();
        }
        let mut action = None;
        for button in buttons {
            action = button(ui).or(action);
        }
        let Some(action) = action else {
            return;
        };
        let focus = self.file_focus();
        let doc = &mut *self.doc;
        if let Some(buffer) = doc.doc.as_mut() {
            apply(&mut doc.engine, buffer, action);
        }
        ui.memory_mut(|memory| memory.request_focus(focus));
    }
}

fn heading_button(ui: &mut egui::Ui) -> Option<Format> {
    let mut action = None;
    appearance::icon_menu_button(ui, "Heading", |ui| {
        for (level, label) in [
            (1, "Heading 1"),
            (2, "Heading 2"),
            (3, "Heading 3"),
            (4, "Heading 4"),
            (0, "Body text"),
        ] {
            if ui.button(label).clicked() {
                action = Some(Format::Heading(level));
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Heading level");
    action
}

fn bold_button(ui: &mut egui::Ui) -> Option<Format> {
    appearance::sidebar_action(ui, "Bold", "Bold")
        .clicked()
        .then_some(Format::Wrap("**"))
}

fn italic_button(ui: &mut egui::Ui) -> Option<Format> {
    appearance::sidebar_action(ui, "Italic", "Italic")
        .clicked()
        .then_some(Format::Wrap("*"))
}

fn list_button(ui: &mut egui::Ui) -> Option<Format> {
    appearance::sidebar_action(ui, "List", "Bullet list")
        .clicked()
        .then_some(Format::List)
}

fn apply(engine: &mut VimEngine, buffer: &mut Doc, action: Format) {
    let line = engine.cursor().line;
    let text = buffer.line_text(line);
    match action {
        Format::Wrap(marker) => wrap(engine, buffer, marker),
        Format::Heading(level) => {
            let prefix = if level == 0 {
                String::new()
            } else {
                format!("{} ", "#".repeat(level))
            };
            set_prefix(engine, buffer, heading_prefix(&text), &prefix);
        }
        Format::List => {
            let old = bullet_prefix(&text);
            set_prefix(engine, buffer, old, if old > 0 { "" } else { "- " });
        }
    }
}

/// Wrap a charwise selection in `marker`, or insert an empty pair and park
/// the caret between the markers. One undo step either way.
fn wrap(engine: &mut VimEngine, buffer: &mut Doc, marker: &str) {
    let charwise = engine.block_span().is_none() && engine.mode() != Mode::VisualLine;
    if charwise && engine.selection().is_some() {
        buffer.begin_undo_group();
        let text = engine.cut(buffer);
        engine.paste_text(buffer, &format!("{marker}{text}{marker}"));
        buffer.end_undo_group();
        return;
    }
    let before = engine.cursor();
    engine.paste_text(buffer, &format!("{marker}{marker}"));
    engine.place_cursor(
        buffer,
        before.line,
        before.col.saturating_add(marker.chars().count()),
    );
}

/// Replace the first `old` chars of the caret line with `new`; the caret
/// keeps its place in the text after the prefix.
fn set_prefix(engine: &mut VimEngine, buffer: &mut Doc, old: usize, new: &str) {
    let cursor = engine.cursor();
    let start = buffer.char_at_line_col(cursor.line, 0);
    buffer.begin_undo_group();
    buffer.delete(start..start.saturating_add(old));
    buffer.insert(start, new);
    buffer.end_undo_group();
    buffer.end_run();
    let col = cursor
        .col
        .saturating_sub(old)
        .saturating_add(new.chars().count());
    engine.place_cursor(buffer, cursor.line, col);
}

/// Chars of a `#`–`######` heading marker and its space, or 0.
fn heading_prefix(line: &str) -> usize {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if (1..=6).contains(&hashes) && line.chars().nth(hashes) == Some(' ') {
        hashes.saturating_add(1)
    } else {
        0
    }
}

/// Chars of a `- `, `* ` or `+ ` bullet marker, or 0.
fn bullet_prefix(line: &str) -> usize {
    if ["- ", "* ", "+ "]
        .iter()
        .any(|bullet| line.starts_with(bullet))
    {
        2
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str, line: usize, col: usize, action: Format) -> (String, usize) {
        let mut buffer = Doc::new(text);
        let mut engine = VimEngine::default();
        engine.place_cursor(&mut buffer, line, col);
        apply(&mut engine, &mut buffer, action);
        (buffer.text(), engine.cursor().col)
    }

    #[test]
    fn heading_buttons_replace_the_line_marker_and_keep_the_caret_on_text() {
        assert_eq!(
            run("Title", 0, 2, Format::Heading(2)),
            ("## Title".into(), 5)
        );
        assert_eq!(
            run("### Title", 0, 6, Format::Heading(1)),
            ("# Title".into(), 4)
        );
        assert_eq!(
            run("# Title", 0, 3, Format::Heading(0)),
            ("Title".into(), 1)
        );
        assert_eq!(run("#tag", 0, 0, Format::Heading(1)).0, "# #tag");
    }

    #[test]
    fn list_button_toggles_a_bullet() {
        assert_eq!(run("item", 0, 0, Format::List).0, "- item");
        assert_eq!(run("* item", 0, 3, Format::List).0, "item");
    }

    #[test]
    fn bold_without_selection_inserts_a_pair_with_the_caret_inside() {
        let (text, col) = run("ab", 0, 1, Format::Wrap("**"));
        assert_eq!(text, "a****b");
        assert_eq!(col, 3);
    }

    #[test]
    fn heading_change_is_one_undo_step() {
        let mut buffer = Doc::new("# Title");
        let mut engine = VimEngine::default();
        apply(&mut engine, &mut buffer, Format::Heading(3));
        assert_eq!(buffer.text(), "### Title");
        assert!(buffer.undo());
        assert_eq!(buffer.text(), "# Title");
    }
}
