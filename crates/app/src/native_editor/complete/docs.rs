//! The docs panel beside the completion list, and the lazy item parts
//! behind it: the selected item is resolved (`completionItem/resolve`)
//! once the selection rests on it, and auto-import edits that arrive
//! after an item was accepted apply only while they still fit.
use super::*;
use std::hash::Hasher as _;

/// How long the selection rests on an item before its docs are asked for.
pub(crate) const RESOLVE_DELAY: Duration = Duration::from_millis(100);
/// Docs panel width and height limit, as the `IntelliJ` documentation
/// popup beside the lookup.
const DOCS_WIDTH: f32 = 420.0;
const DOCS_MAX_HEIGHT: f32 = 320.0;
/// Space between the list and the panel.
const DOCS_GAP: f32 = 4.0;

/// The selected item's debounced docs lookup.
#[derive(Default)]
pub(crate) struct DocsLookup {
    /// The selected item (an index into `items`), and since when.
    focus: Option<(usize, Instant)>,
    /// Items asked about already: each is asked once per list.
    asked: Vec<usize>,
}

/// What [`resolve_due`] decided for the selected item.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Due {
    /// Nothing to ask: no selection, docs known, or asked already.
    Never,
    /// Ask about item `index` now.
    Now(usize),
    /// Look again after this wait, if the selection stays.
    Later(Duration),
}

/// Whether to resolve the selected item at `now`: once it rested there
/// for [`RESOLVE_DELAY`] without docs. Not while a newer list is on its
/// way, because its answer replaces these items.
pub(crate) fn resolve_due(popup: &mut CompletionPopup, now: Instant) -> Due {
    let Some(index) = popup.visible.get(popup.selected).copied() else {
        return Due::Never;
    };
    let since = match popup.docs.focus {
        Some((focused, since)) if focused == index => since,
        _ => {
            popup.docs.focus = Some((index, now));
            now
        }
    };
    let known = popup
        .items
        .get(index)
        .is_none_or(|item| item.documentation.is_some() || item.resolved);
    if known || popup.generation.is_some() || popup.docs.asked.contains(&index) {
        return Due::Never;
    }
    let rested = now.saturating_duration_since(since);
    if rested < RESOLVE_DELAY {
        return Due::Later(RESOLVE_DELAY.saturating_sub(rested));
    }
    popup.docs.asked.push(index);
    Due::Now(index)
}

/// Copy what resolve already filled in onto the same items in a new list.
/// An incomplete list is asked again on every keystroke: the panel keeps
/// its docs instead of blinking, and no item is resolved twice.
pub(crate) fn carry_resolved(
    old: &[native_lsp::CompletionItem],
    new: &mut [native_lsp::CompletionItem],
) {
    for known in old
        .iter()
        .filter(|item| item.resolved || item.documentation.is_some())
    {
        let same = new.iter_mut().find(|item| {
            !item.resolved
                && item.label == known.label
                && item.kind == known.kind
                && item.label_detail == known.label_detail
                && item.label_description == known.label_description
        });
        if let Some(same) = same {
            take_resolved(same, known);
        }
    }
}

/// The docs, detail, and (once resolved) auto-import edits of `known`
/// that `same` lacks.
fn take_resolved(same: &mut native_lsp::CompletionItem, known: &native_lsp::CompletionItem) {
    if same.documentation.is_none() {
        same.documentation.clone_from(&known.documentation);
    }
    if same.detail.is_none() {
        same.detail.clone_from(&known.detail);
    }
    if known.resolved {
        if same.additional.is_empty() {
            same.additional.clone_from(&known.additional);
        }
        same.resolved = true;
    }
}

/// Hash of all text before char column `col` of `line`.
pub(crate) fn text_before_hash(buffer: &Doc, line: usize, col: usize) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for above in 0..line.min(buffer.line_count()) {
        hasher.write_u64(buffer.line_hash(above));
    }
    let head: String = buffer.line_text(line).chars().take(col).collect();
    hasher.write(head.as_bytes());
    hasher.finish()
}

/// Whether auto-import edits that arrived after their item was accepted
/// still apply: each ends at or before char column `col` of `line`,
/// where the item was inserted, and the text before that point still
/// hashes to `before` (see [`text_before_hash`]).
pub(crate) fn imports_fit(
    buffer: &Doc,
    (line, col, before): (usize, usize, u64),
    edits: &[native_lsp::RenameTextEdit],
) -> bool {
    let ends_before = |edit: &native_lsp::RenameTextEdit| {
        let end = usize::try_from(edit.range.end.line).unwrap_or(usize::MAX);
        end < line
            || (end == line
                && native_lsp::utf16_to_char_col(&buffer.line_text(line), edit.range.end.character)
                    <= col)
    };
    edits.iter().all(ends_before) && text_before_hash(buffer, line, col) == before
}

/// Where the docs panel goes: right of the list, or left of it when the
/// screen has no room on the right. Returns the point and the panel
/// corner placed on it.
pub(crate) fn docs_place(list: egui::Rect, screen: egui::Rect) -> (egui::Pos2, egui::Align2) {
    if list.right() + DOCS_GAP + DOCS_WIDTH <= screen.right() {
        (
            egui::pos2(list.right() + DOCS_GAP, list.top()),
            egui::Align2::LEFT_TOP,
        )
    } else {
        (
            egui::pos2(list.left() - DOCS_GAP, list.top()),
            egui::Align2::RIGHT_TOP,
        )
    }
}

/// The selected item's detail (in the code font) and docs beside the
/// list. `render` draws the docs: the Hover window's renderer. Drawn only
/// when the list was drawn this frame and the item has either.
pub(crate) fn docs_ui(
    ui: &egui::Ui,
    popup: &CompletionPopup,
    render: impl FnOnce(&mut egui::Ui, &native_lsp::Hover),
) {
    let Some(list) = popup.rect else {
        return;
    };
    let Some(index) = popup.visible.get(popup.selected).copied() else {
        return;
    };
    let Some(item) = popup
        .items
        .get(index)
        .filter(|item| item.documentation.is_some() || item.detail.is_some())
    else {
        return;
    };
    let (at, pivot) = docs_place(list, ui.ctx().content_rect());
    egui::Area::new(egui::Id::unique(("native-completion-docs", &popup.path)))
        .order(egui::Order::Foreground)
        .pivot(pivot)
        .fixed_pos(at)
        .show(ui.ctx(), |ui| {
            appearance::popup_text_style(ui.style_mut());
            appearance::popup_frame(ui.style()).show(ui, |ui| {
                ui.set_width(DOCS_WIDTH);
                egui::ScrollArea::vertical()
                    .id_salt(("native-completion-docs", index))
                    .max_height(DOCS_MAX_HEIGHT)
                    .show(ui, |ui| docs_body(ui, item, render));
            });
        });
}

/// The detail, a rule, then the docs.
fn docs_body(
    ui: &mut egui::Ui,
    item: &native_lsp::CompletionItem,
    render: impl FnOnce(&mut egui::Ui, &native_lsp::Hover),
) {
    if let Some(detail) = &item.detail {
        ui.add(egui::Label::new(egui::RichText::new(detail).monospace()).wrap());
    }
    if let Some(doc) = &item.documentation {
        if item.detail.is_some() {
            ui.separator();
        }
        render(ui, doc);
    }
}
