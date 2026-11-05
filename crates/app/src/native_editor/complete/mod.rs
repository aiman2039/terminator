//! Code completion popup (`IntelliJ` basic completion). Typing an
//! identifier char or one of the server's trigger characters (or
//! Ctrl+Space) asks the language server. Further typing filters the
//! answer locally, or asks again while the server says its list is
//! incomplete. Up/Down pick, Enter/Tab insert, Escape closes. A docs
//! panel beside the list shows the selected item's docs ([`docs`]).

mod docs;
mod popup;
mod signature;
#[cfg(test)]
mod tests;

use super::*;
use crate::lsp_manager::{CompletionResults, ResolveFor};
pub(crate) use docs::{Due, docs_ui, imports_fit, resolve_due};
use native_lsp::CompletionTrigger;
use popup::{PopupAction, popup_ui};
pub(crate) use signature::{
    SignatureAfterFrameInput, SignaturePopup, signature_after_frame, signature_ui,
};
use terminator_native_edit::vim::Cursor;

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
    /// The typed prefix, painted in the match color on each row.
    pub prefix: String,
    /// The server said `items` is incomplete: typing asks it again.
    pub incomplete: bool,
    /// Generation of the request that answered `items`. Resolve answers
    /// for an older list are dropped.
    pub list: u64,
    /// Where the list was drawn this frame; the docs panel goes beside it.
    pub rect: Option<egui::Rect>,
    /// When the last request for this popup went out, until answered.
    pub waiting: Option<Instant>,
    /// The selected item's debounced docs lookup.
    pub docs: docs::DocsLookup,
}

impl CompletionPopup {
    /// An empty popup waiting for request `generation`, for the prefix
    /// that starts at char column `start` of `line`.
    pub(crate) fn new(
        path: &Path,
        generation: u64,
        (line, start): (usize, usize),
        anchor: Option<egui::Rect>,
    ) -> Self {
        Self {
            path: path.to_owned(),
            generation: Some(generation),
            items: Vec::new(),
            line,
            start,
            visible: Vec::new(),
            selected: 0,
            anchor,
            prefix: String::new(),
            incomplete: false,
            list: generation,
            rect: None,
            waiting: Some(Instant::now()),
            docs: docs::DocsLookup::default(),
        }
    }

    /// Show the answer to the newest request. The selected item stays
    /// selected, and what resolve filled in for items it lists again stays.
    pub(crate) fn adopt(&mut self, results: CompletionResults) {
        let selected = self
            .visible
            .get(self.selected)
            .and_then(|index| self.items.get(*index))
            .map(|item| item.label.clone());
        let mut items = results.items;
        docs::carry_resolved(&self.items, &mut items);
        self.items = items;
        self.incomplete = results.incomplete;
        self.list = results.generation;
        self.generation = None;
        self.waiting = None;
        self.docs = docs::DocsLookup::default();
        self.visible = rank(&self.items, &self.prefix);
        self.selected = selected
            .and_then(|label| {
                self.visible.iter().position(|index| {
                    self.items
                        .get(*index)
                        .is_some_and(|item| item.label == label)
                })
            })
            .unwrap_or(0);
    }

    /// Merge a resolve answer into item `index` of the list that request
    /// `generation` answered; an answer for an older list is dropped.
    pub(crate) fn merge_resolved(
        &mut self,
        (generation, index): (u64, usize),
        item: native_lsp::CompletionItem,
    ) {
        if generation != self.list {
            return;
        }
        if let Some(listed) = self.items.get_mut(index) {
            listed.merge_resolved(item);
        }
        // A newer list may still be on its way: keep looking for it.
        if self.generation.is_none() {
            self.waiting = None;
        }
    }
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

/// How the char just typed before `col` opens completion: one of the
/// server's `triggers` (sent as that character), an identifier char
/// (invoked), or, when the server lists no triggers, `.` or the second
/// `:` of `::`. `None` when it does not open completion.
pub(crate) fn trigger_at(line: &str, col: usize, triggers: &[char]) -> Option<CompletionTrigger> {
    let chars: Vec<char> = line.chars().collect();
    let last = col
        .checked_sub(1)
        .and_then(|index| chars.get(index))
        .copied()?;
    if triggers.contains(&last) {
        return Some(CompletionTrigger::Character(last));
    }
    if identifier(last) {
        return Some(CompletionTrigger::Invoked);
    }
    let path = last == ':' && col >= 2 && chars.get(col.saturating_sub(2)) == Some(&':');
    (triggers.is_empty() && (last == '.' || path)).then_some(CompletionTrigger::Invoked)
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
/// this line), as one undo step. A snippet selects its first stop, and
/// Tab moves on ([`VimEngine::start_snippet`]). Returns where the
/// inserted text starts, or `None` when the caret left the line (nothing
/// is inserted).
pub(crate) struct AcceptInput<'a> {
    pub(crate) buffer: &'a mut Doc,
    pub(crate) engine: &'a mut VimEngine,
    /// The file, for snippet variables such as `TM_FILENAME`.
    pub(crate) path: &'a Path,
    pub(crate) line: usize,
    pub(crate) start: usize,
    pub(crate) item: &'a native_lsp::CompletionItem,
}

pub(crate) fn accept(input: AcceptInput<'_>) -> Option<(usize, usize)> {
    let AcceptInput {
        buffer,
        engine,
        path,
        line,
        start,
        item,
    } = input;
    let caret = engine.cursor();
    if caret.line != line {
        return None;
    }
    let text = buffer.line_text(line);
    let on_line = |range: &native_lsp::Range| range.start.line as usize == line;
    let from_col = item
        .edit
        .filter(on_line)
        .map_or(start, |range| {
            native_lsp::utf16_to_char_col(&text, range.start.character)
        })
        .min(caret.col);
    // An insert-and-replace item also takes the rest of the word after
    // the caret, so accepting inside a word does not duplicate its tail.
    let to_col = item
        .replace
        .filter(|range| on_line(range) && range.end.line == range.start.line)
        .map_or(caret.col, |range| {
            native_lsp::utf16_to_char_col(&text, range.end.character)
        })
        .max(caret.col);
    let snippet = if item.snippet {
        let vars = native_lsp::SnippetVars {
            path: Some(path.to_owned()),
            line,
            word: engine.word_at_caret(buffer).unwrap_or_default(),
            line_text: text.clone(),
            selected: String::new(),
        };
        native_lsp::parse_snippet(&item.insert, &vars)
    } else {
        native_lsp::Snippet::plain(&item.insert)
    };
    buffer.begin_undo_group();
    // Auto-imports first, in the original coordinates; the caret follows
    // its text, and the item's own range is taken relative to it.
    super::actions::apply_edits_keep_caret(buffer, engine, &item.additional);
    let moved = engine.cursor();
    let from = buffer.char_at_line_col(
        moved.line,
        moved.col.saturating_sub(caret.col.saturating_sub(from_col)),
    );
    let to = buffer.char_at_line_col(
        moved.line,
        moved.col.saturating_add(to_col.saturating_sub(caret.col)),
    );
    buffer.delete(from..to);
    buffer.insert(from, &snippet.text);
    buffer.end_undo_group();
    engine.start_snippet(buffer, from, &snippet);
    Some(buffer.line_col_at(from))
}

/// An item accepted before its auto-import edits were known: the app
/// resolves it, and the edits follow when they still fit
/// ([`imports_fit`]).
pub(crate) struct AcceptedItem {
    pub(crate) raw: serde_json::Value,
    pub(crate) target: ResolveFor,
}

/// Insert `item`. When it came without auto-import edits and the server
/// may still have some, say what to resolve so they follow the insert.
/// An earlier resolve without edits does not count: one made while the
/// server still loaded finds no import yet.
fn accept_item(
    buffer: &mut Doc,
    engine: &mut VimEngine,
    (path, line, start): (&Path, usize, usize),
    item: native_lsp::CompletionItem,
) -> Option<AcceptedItem> {
    let (line, col) = accept(AcceptInput {
        buffer,
        engine,
        path,
        line,
        start,
        item: &item,
    })?;
    if !item.additional.is_empty() || item.raw.is_null() {
        return None;
    }
    Some(AcceptedItem {
        raw: item.raw,
        target: accepted_at(buffer, (line, col)),
    })
}

/// The resolve target for an item inserted at char column `col` of
/// `line`: its late edits apply while the text before stays the same.
pub(crate) fn accepted_at(buffer: &Doc, (line, col): (usize, usize)) -> ResolveFor {
    let before = docs::text_before_hash(buffer, line, col);
    ResolveFor::Accepted { line, col, before }
}

/// A completion request for the app to send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompletionAsk {
    pub(crate) position: native_lsp::Position,
    pub(crate) line: usize,
    /// Char column where the typed prefix starts.
    pub(crate) start: usize,
    pub(crate) trigger: CompletionTrigger,
}

/// Ask at the caret, for the prefix that starts at char column `start`.
fn ask_at(text: &str, caret: Cursor, start: usize, trigger: CompletionTrigger) -> AfterFrame {
    let ask = u32::try_from(caret.line).ok().map(|line| CompletionAsk {
        position: native_lsp::Position {
            line,
            character: native_lsp::char_col_to_utf16(text, caret.col),
        },
        line: caret.line,
        start,
        trigger,
    });
    AfterFrame {
        ask,
        accepted: None,
    }
}

/// What an open popup does this frame.
enum Upkeep {
    Keep,
    Close,
    /// Enter, Tab, or a click: the index of the item to insert (`None`
    /// when its row is gone).
    Accept(Option<usize>),
}

/// Close the popup when the caret leaves its prefix, re-rank it as the
/// prefix changes, then draw it and handle its keys.
fn upkeep(
    ui: &egui::Ui,
    open: &mut CompletionPopup,
    (caret, text, inserting): (Cursor, &str, bool),
    caret_rect: Option<egui::Rect>,
) -> Upkeep {
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
        return Upkeep::Close;
    }
    open.visible = rank(&open.items, &prefix);
    open.prefix = prefix;
    if caret_rect.is_some() {
        open.anchor = caret_rect;
    }
    match popup_ui(ui, open) {
        PopupAction::Accept(row) => Upkeep::Accept(open.visible.get(row).copied()),
        PopupAction::Close => Upkeep::Close,
        PopupAction::None => Upkeep::Keep,
    }
}

/// Per-frame completion upkeep for one editor view, after the view drew:
/// close the popup when the caret leaves its prefix, re-rank as the
/// prefix grows, handle its keys, and say whether to ask the server:
/// Ctrl+Space or a trigger ([`trigger_at`]) while no popup is open, or
/// any edit of the prefix while the open list is incomplete.
pub(crate) struct AfterFrameInput<'a> {
    pub(crate) ui: &'a egui::Ui,
    pub(crate) popup: &'a mut Option<CompletionPopup>,
    pub(crate) buffer: &'a mut Doc,
    pub(crate) engine: &'a mut VimEngine,
    pub(crate) revision_before: u64,
    pub(crate) manual: bool,
    pub(crate) caret_rect: Option<egui::Rect>,
    pub(crate) server: bool,
    /// The server's trigger characters.
    pub(crate) triggers: &'a [char],
}

/// What completion upkeep asks the app for after a frame.
#[derive(Default)]
pub(crate) struct AfterFrame {
    /// Ask the server for completions.
    pub(crate) ask: Option<CompletionAsk>,
    /// An accepted item whose auto-import edits the server may still add.
    pub(crate) accepted: Option<AcceptedItem>,
}

pub(crate) fn after_frame(input: AfterFrameInput<'_>) -> AfterFrame {
    let AfterFrameInput {
        ui,
        popup,
        buffer,
        engine,
        revision_before,
        manual,
        caret_rect,
        server,
        triggers,
    } = input;
    use terminator_native_edit::vim::{ModalEngine as _, Mode};
    let caret = engine.cursor();
    let text = buffer.line_text(caret.line);
    let inserting = engine.mode() == Mode::Insert;
    let edited = buffer.revision() != revision_before && inserting;
    if let Some(open) = popup.as_mut() {
        let (line, start, incomplete) = (open.line, open.start, open.incomplete);
        match upkeep(ui, open, (caret, &text, inserting), caret_rect) {
            // The server wants to see the new prefix, not only filter.
            Upkeep::Keep if incomplete && edited && server => {
                return ask_at(&text, caret, start, CompletionTrigger::Incomplete);
            }
            Upkeep::Keep => {}
            Upkeep::Close => *popup = None,
            Upkeep::Accept(index) => {
                let chosen = index.and_then(|index| open.items.get(index)).cloned();
                let path = open.path.clone();
                *popup = None;
                let at = (path.as_path(), line, start);
                let accepted = chosen.and_then(|item| accept_item(buffer, engine, at, item));
                return AfterFrame {
                    ask: None,
                    accepted,
                };
            }
        }
    }
    let trigger = if manual {
        Some(CompletionTrigger::Invoked)
    } else {
        edited
            .then(|| trigger_at(&text, caret.col, triggers))
            .flatten()
    };
    match trigger {
        Some(trigger) if popup.is_none() && server => {
            ask_at(&text, caret, prefix_start(&text, caret.col), trigger)
        }
        _ => AfterFrame::default(),
    }
}
