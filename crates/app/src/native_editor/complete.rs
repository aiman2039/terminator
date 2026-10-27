//! Code completion popup (`IntelliJ` basic completion). Typing an
//! identifier char or `.` (or Ctrl+Space) asks the language server once;
//! further typing filters the answer locally. Up/Down pick, Enter/Tab
//! insert, Escape closes.

use super::*;

/// Rows shown at once; the list scrolls past them.
const VISIBLE_ROWS: usize = 12;

pub(crate) struct CompletionPopup {
    pub path: PathBuf,
    /// Generation of the in-flight request (`None` once answered).
    pub generation: Option<u64>,
    pub items: Vec<native_lsp::CompletionItem>,
    /// Line and char column where the typed prefix starts.
    pub line: usize,
    pub start: usize,
    /// Indices into `items` that match the current prefix, best first.
    pub visible: Vec<usize>,
    pub selected: usize,
    pub anchor: Option<egui::Rect>,
}

fn identifier(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Char column where the identifier ending at `col` starts.
pub(crate) fn prefix_start(line: &str, col: usize) -> usize {
    let chars: Vec<char> = line.chars().collect();
    let mut start = col.min(chars.len());
    while start > 0
        && chars
            .get(start.saturating_sub(1))
            .copied()
            .is_some_and(identifier)
    {
        start = start.saturating_sub(1);
    }
    start
}

/// Whether the char just typed before `col` should open completion: an
/// identifier char, `.`, or the second `:` of `::`.
pub(crate) fn triggers(line: &str, col: usize) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let Some(last) = col
        .checked_sub(1)
        .and_then(|index| chars.get(index))
        .copied()
    else {
        return false;
    };
    identifier(last)
        || last == '.'
        || (last == ':' && col >= 2 && chars.get(col.saturating_sub(2)) == Some(&':'))
}

fn subsequence(text: &str, query: &str) -> bool {
    let mut text = text.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .all(|wanted| text.any(|c| c == wanted))
}

/// Items matching `prefix`: prefix matches first (exact case before any
/// case), then subsequence matches, each in server order.
pub(crate) fn rank(items: &[native_lsp::CompletionItem], prefix: &str) -> Vec<usize> {
    let lower = prefix.to_lowercase();
    let mut ranked: Vec<(u8, &str, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let tier = if item.filter.starts_with(prefix) {
                0
            } else if item.filter.to_lowercase().starts_with(&lower) {
                1
            } else if subsequence(&item.filter, prefix) {
                2
            } else {
                return None;
            };
            Some((tier, item.sort.as_str(), index))
        })
        .collect();
    ranked.sort_unstable();
    ranked.into_iter().map(|(_, _, index)| index).collect()
}

/// Insert `item`, replacing the typed prefix (or the server's range on
/// this line), as one undo step; the caret lands on the snippet's stop.
pub(crate) fn accept(
    buffer: &mut Doc,
    engine: &mut VimEngine,
    line: usize,
    start: usize,
    item: &native_lsp::CompletionItem,
) {
    let caret = engine.cursor();
    if caret.line != line {
        return;
    }
    let text = buffer.line_text(line);
    let from_col = item
        .edit
        .filter(|range| range.start.line as usize == line)
        .map_or(start, |range| {
            native_lsp::utf16_to_char_col(&text, range.start.character)
        })
        .min(caret.col);
    let (insert, stop) = if item.snippet {
        native_lsp::expand_snippet(&item.insert)
    } else {
        let count = item.insert.chars().count();
        (item.insert.clone(), count)
    };
    let from = buffer.char_at_line_col(line, from_col);
    let to = buffer.char_at_line_col(line, caret.col);
    buffer.begin_undo_group();
    buffer.delete(from..to);
    buffer.insert(from, &insert);
    buffer.end_undo_group();
    let (line, col) = buffer.line_col_at(from.saturating_add(stop));
    let _ = engine.place_cursor(buffer, line, col);
}

/// Per-frame completion upkeep for one editor view, after the view drew:
/// close the popup when the caret leaves its prefix, re-rank as the
/// prefix grows, handle its keys, and say whether to ask the server
/// (`(position, line, prefix start)`): Ctrl+Space, or typing an
/// identifier char, `.`, or `::` while no popup is open.
#[allow(clippy::too_many_arguments)]
pub(crate) fn after_frame(
    ui: &egui::Ui,
    popup: &mut Option<CompletionPopup>,
    buffer: &mut Doc,
    engine: &mut VimEngine,
    revision_before: u64,
    manual: bool,
    caret_rect: Option<egui::Rect>,
    server: bool,
) -> Option<(native_lsp::Position, usize, usize)> {
    use terminator_native_edit::vim::{ModalEngine as _, Mode};
    let caret = engine.cursor();
    let text = buffer.line_text(caret.line);
    let inserting = engine.mode() == Mode::Insert;
    if let Some(open) = popup.as_mut() {
        let chars: Vec<char> = text.chars().collect();
        let prefix: String = chars
            .get(open.start..caret.col)
            .unwrap_or_default()
            .iter()
            .collect();
        if caret.line != open.line
            || caret.col < open.start
            || !inserting
            || !prefix.chars().all(identifier)
        {
            *popup = None;
        } else {
            open.visible = rank(&open.items, &prefix);
            if caret_rect.is_some() {
                open.anchor = caret_rect;
            }
            match popup_ui(ui, open) {
                PopupAction::Accept(row) => {
                    let chosen = open
                        .visible
                        .get(row)
                        .and_then(|index| open.items.get(*index))
                        .cloned();
                    let (line, start) = (open.line, open.start);
                    *popup = None;
                    if let Some(item) = chosen {
                        accept(buffer, engine, line, start, &item);
                    }
                    return None;
                }
                PopupAction::Close => *popup = None,
                PopupAction::None => {}
            }
        }
    }
    let typed = buffer.revision() != revision_before && inserting && triggers(&text, caret.col);
    if popup.is_none() && server && (manual || typed) {
        let position = native_lsp::Position {
            line: u32::try_from(caret.line).ok()?,
            character: native_lsp::char_col_to_utf16(&text, caret.col),
        };
        return Some((position, caret.line, prefix_start(&text, caret.col)));
    }
    None
}

/// Parameter info for the call around the caret.
pub(crate) struct SignaturePopup {
    pub path: PathBuf,
    pub generation: u64,
    pub help: Option<native_lsp::SignatureHelp>,
    pub line: usize,
    /// Column just after the `(` that opened the call.
    pub open: usize,
    pub anchor: Option<egui::Rect>,
}

/// Column after the innermost unclosed `(` before `col` on this line.
pub(crate) fn open_paren(line: &str, col: usize) -> Option<usize> {
    let chars: Vec<char> = line.chars().collect();
    let mut depth = 0usize;
    for index in (0..col.min(chars.len())).rev() {
        match chars.get(index) {
            Some(')') => depth = depth.saturating_add(1),
            Some('(') if depth == 0 => return Some(index.saturating_add(1)),
            Some('(') => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

/// Signature upkeep after the view drew: close when the caret leaves the
/// call, and say when to ask (typing `(` or `,` inside a call).
pub(crate) fn signature_after_frame(
    popup: &mut Option<SignaturePopup>,
    buffer: &Doc,
    engine: &VimEngine,
    revision_before: u64,
    server: bool,
) -> Option<(native_lsp::Position, usize, usize)> {
    use terminator_native_edit::vim::{ModalEngine as _, Mode};
    let caret = engine.cursor();
    let text = buffer.line_text(caret.line);
    let open = open_paren(&text, caret.col);
    if let Some(shown) = popup.as_ref()
        && (shown.line != caret.line || open != Some(shown.open) || engine.mode() != Mode::Insert)
    {
        *popup = None;
    }
    let typed = buffer.revision() != revision_before
        && engine.mode() == Mode::Insert
        && caret
            .col
            .checked_sub(1)
            .and_then(|index| text.chars().nth(index))
            .is_some_and(|c| c == '(' || c == ',');
    let open = open?;
    if !(server && typed) {
        return None;
    }
    let position = native_lsp::Position {
        line: u32::try_from(caret.line).ok()?,
        character: native_lsp::char_col_to_utf16(&text, caret.col),
    };
    Some((position, caret.line, open))
}

/// Paint parameter info above the caret, the active parameter in bold.
pub(crate) fn signature_ui(ui: &egui::Ui, popup: &SignaturePopup) {
    let (Some(help), Some(anchor)) = (&popup.help, popup.anchor) else {
        return;
    };
    let font = egui::FontId::monospace(12.0);
    let mut job = egui::text::LayoutJob::default();
    let chars: Vec<char> = help.label.chars().collect();
    let (start, end) = help.active.as_ref().map_or((0, 0), |range| {
        (range.start.min(chars.len()), range.end.min(chars.len()))
    });
    let plain = ui.visuals().text_color();
    let strong = ui.visuals().strong_text_color();
    for (range, color, underline) in [
        (0..start, plain, false),
        (start..end, strong, true),
        (end..chars.len(), plain, false),
    ] {
        let text: String = chars.get(range).unwrap_or_default().iter().collect();
        if text.is_empty() {
            continue;
        }
        job.append(
            &text,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color,
                underline: if underline {
                    egui::Stroke::new(1.0, strong)
                } else {
                    egui::Stroke::NONE
                },
                ..Default::default()
            },
        );
    }
    egui::Area::new(egui::Id::unique(("native-signature", &popup.path)))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(anchor.left_top())
        .interactable(false)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(640.0);
                ui.label(job);
                if let Some(doc) = &help.documentation {
                    let first = doc
                        .lines()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or_default();
                    ui.weak(first);
                }
            });
        });
}

/// What the popup asked for this frame.
pub(crate) enum PopupAction {
    None,
    Close,
    Accept(usize),
}

/// Keys and clicks for a visible popup: Up/Down move, Enter/Tab accept,
/// Escape closes. Paints the list under the caret.
pub(crate) fn popup_ui(ui: &egui::Ui, popup: &mut CompletionPopup) -> PopupAction {
    if popup.visible.is_empty() {
        return PopupAction::None;
    }
    popup.selected = popup.selected.min(popup.visible.len().saturating_sub(1));
    let mut action = PopupAction::None;
    let mut moved = false;
    ui.input_mut(|input| {
        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
            popup.selected = popup
                .selected
                .saturating_add(1)
                .checked_rem(popup.visible.len())
                .unwrap_or(0);
            moved = true;
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
            popup.selected = popup
                .selected
                .checked_sub(1)
                .unwrap_or(popup.visible.len().saturating_sub(1));
            moved = true;
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            || input.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
        {
            action = PopupAction::Accept(popup.selected);
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            action = PopupAction::Close;
        }
    });
    let Some(anchor) = popup.anchor else {
        return action;
    };
    egui::Area::new(egui::Id::unique(("native-completion", &popup.path)))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor.left_bottom())
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width(460.0);
                #[allow(clippy::cast_precision_loss)] // small row count
                let max_height = VISIBLE_ROWS as f32 * 20.0;
                egui::ScrollArea::vertical()
                    .max_height(max_height)
                    .show(ui, |ui| {
                        for (row, index) in popup.visible.iter().enumerate().take(200) {
                            let Some(item) = popup.items.get(*index) else {
                                continue;
                            };
                            let selected = row == popup.selected;
                            let response = ui
                                .horizontal(|ui| {
                                    let kind = item
                                        .kind
                                        .as_deref()
                                        .and_then(|kind| kind.chars().next())
                                        .unwrap_or('•');
                                    ui.weak(egui::RichText::new(kind.to_string()).monospace());
                                    let label = ui.selectable_label(
                                        selected,
                                        egui::RichText::new(&item.label).monospace(),
                                    );
                                    if let Some(detail) = &item.detail {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(detail).weak().small(),
                                            )
                                            .truncate(),
                                        );
                                    }
                                    label
                                })
                                .inner;
                            if selected && moved {
                                response.scroll_to_me(None);
                            }
                            if response.clicked() {
                                action = PopupAction::Accept(row);
                            }
                        }
                    });
            });
        });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, sort: &str) -> native_lsp::CompletionItem {
        native_lsp::CompletionItem {
            label: label.into(),
            kind: Some("Function".into()),
            detail: None,
            insert: label.into(),
            snippet: false,
            filter: label.into(),
            sort: sort.into(),
            edit: None,
        }
    }

    #[test]
    fn signature_opens_inside_calls_and_closes_outside() {
        assert_eq!(open_paren("f(a, g(b), ", 11), Some(2));
        assert_eq!(open_paren("f(a)", 4), None);
        let mut buffer = Doc::new("f(\n");
        let mut engine = VimEngine::new();
        engine.set_modeless(true);
        let _ = engine.place_cursor(&mut buffer, 0, 2);
        engine.modeless_key(&mut buffer, terminator_native_edit::vim::Key::Char('x'));
        let before = buffer.revision();
        engine.type_text(&mut buffer, ",");
        let mut popup = None;
        let asked = signature_after_frame(&mut popup, &buffer, &engine, before, true);
        assert_eq!(asked.map(|(_, line, open)| (line, open)), Some((0, 2)));
        popup = Some(SignaturePopup {
            path: PathBuf::from("/f.rs"),
            generation: 1,
            help: None,
            line: 0,
            open: 2,
            anchor: None,
        });
        // Leaving the call (past its `)`) closes the popup.
        let _ = engine.place_cursor(&mut buffer, 0, 0);
        let rev = buffer.revision();
        assert!(signature_after_frame(&mut popup, &buffer, &engine, rev, true).is_none());
        assert!(popup.is_none());
    }

    #[test]
    fn prefix_and_trigger_rules() {
        assert_eq!(prefix_start("let foo_ba", 10), 4);
        assert!(triggers("x.", 2));
        assert!(triggers("std::", 5));
        assert!(!triggers("a :", 3));
        assert!(!triggers("a ", 2));
    }

    #[test]
    fn ranking_prefers_prefix_matches_then_server_order() {
        let items = [
            item("to_string", "2"),
            item("Tokio", "1"),
            item("into_iter", "0"),
            item("as_ref", "0"),
        ];
        // `to`: exact-case prefix, then any-case prefix, then subsequence.
        assert_eq!(rank(&items, "to"), [0, 1, 2]);
    }

    #[test]
    fn accepting_replaces_the_prefix_and_lands_on_the_snippet_stop() {
        let mut buffer = Doc::new("let v = pri\n");
        let mut engine = VimEngine::new();
        engine.set_modeless(true);
        let _ = engine.place_cursor(&mut buffer, 0, 11);
        let mut chosen = item("println!", "0");
        chosen.insert = "println!(\"$1\")$0".into();
        chosen.snippet = true;
        accept(&mut buffer, &mut engine, 0, 8, &chosen);
        assert_eq!(buffer.text(), "let v = println!(\"\")\n");
        assert_eq!(engine.cursor().col, 18);
        engine.undo(&mut buffer);
        assert_eq!(buffer.text(), "let v = pri\n");
    }
}
