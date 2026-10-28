//! Code completion popup (`IntelliJ` basic completion). Typing an
//! identifier char or `.` (or Ctrl+Space) asks the language server once;
//! further typing filters the answer locally. Up/Down pick, Enter/Tab
//! insert, Escape closes.

mod popup;
mod signature;
#[cfg(test)]
mod tests;

use super::*;
use popup::{PopupAction, popup_ui};
pub(crate) use signature::{
    SignatureAfterFrameInput, SignaturePopup, signature_after_frame, signature_ui,
};

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
pub(crate) struct AcceptInput<'a> {
    pub(crate) buffer: &'a mut Doc,
    pub(crate) engine: &'a mut VimEngine,
    pub(crate) line: usize,
    pub(crate) start: usize,
    pub(crate) item: &'a native_lsp::CompletionItem,
}

pub(crate) fn accept(input: AcceptInput<'_>) {
    let AcceptInput {
        buffer,
        engine,
        line,
        start,
        item,
    } = input;
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
pub(crate) struct AfterFrameInput<'a> {
    pub(crate) ui: &'a egui::Ui,
    pub(crate) popup: &'a mut Option<CompletionPopup>,
    pub(crate) buffer: &'a mut Doc,
    pub(crate) engine: &'a mut VimEngine,
    pub(crate) revision_before: u64,
    pub(crate) manual: bool,
    pub(crate) caret_rect: Option<egui::Rect>,
    pub(crate) server: bool,
}

pub(crate) fn after_frame(
    input: AfterFrameInput<'_>,
) -> Option<(native_lsp::Position, usize, usize)> {
    let AfterFrameInput {
        ui,
        popup,
        buffer,
        engine,
        revision_before,
        manual,
        caret_rect,
        server,
    } = input;
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
                        accept(AcceptInput {
                            buffer,
                            engine,
                            line,
                            start,
                            item: &item,
                        });
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
