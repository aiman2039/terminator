use super::super::doc::Buffer;
use super::super::highlight::{HighlightCache, Language, Span, SpanStyle};
use super::super::vim::{Cursor, Effect, Key as VKey, ModalEngine, Mode, VimEngine};
use super::keys::{
    IdeAction, IdeEdit, command_prompt, comment_token, ctrl_h_is_motion, ctrl_v_is_block,
    echo_of_handled_key, egui_key, ide_edit_in_vim, ide_edit_key, ide_shortcut, mode_name,
    source_focus_id,
};
use super::{BOLD_FAMILY, ITALIC_FAMILY};
use crate::rows::{RowCells, RowHint, display_x, expand_tabs, row_hints};
use egui;
use egui::{Color32, FontId, Pos2, Rect, Sense, Vec2};
/// Worst severity on one buffer row, for the diagnostics gutter.
pub struct DiagnosticMark {
    /// Zero-based buffer row.
    pub line: usize,
    pub severity: crate::lsp::Severity,
}

/// How a block of lines differs from the VCS base (gutter markers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    /// Lines removed just above `start` (the block itself is empty).
    Deleted,
}

/// One changed block in buffer lines `start..end` (empty for deletions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChangeMark {
    pub start: usize,
    pub end: usize,
    pub kind: ChangeKind,
}

/// One line's gutter annotation (blame): `text` in the column, `hover`
/// on hover. Lines of one commit share `group`, which alternates the
/// column shading between commits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Annotation {
    pub text: String,
    pub hover: String,
    pub group: u32,
}

/// Widest annotation column, in cells.
const ANNOTATION_CELLS: usize = 28;

/// Width of the gutter band that shows and accepts clicks on changes.
const CHANGE_BAND: f32 = 8.0;

/// One diagnostic underline: a char-column span on one line, with the
/// message shown when hovered. Multi-line ranges underline their first
/// line to its end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Squiggle {
    pub line: usize,
    pub start: usize,
    pub end: usize,
    pub severity: crate::lsp::Severity,
    pub message: String,
}

/// Underlines for a buffer's diagnostics, sorted by position. Empty or
/// zero-width ranges widen to the word (or char) they start on, so every
/// diagnostic has something to hover.
#[must_use]
pub fn squiggles<B: Buffer>(doc: &B, diagnostics: &[crate::lsp::Diagnostic]) -> Vec<Squiggle> {
    let mut out: Vec<Squiggle> = diagnostics
        .iter()
        .take(1000)
        .filter_map(|diagnostic| {
            let line = usize::try_from(diagnostic.range.start.line).ok()?;
            if line >= doc.line_count() {
                return None;
            }
            let text = doc.line_text(line);
            let len = text.chars().count();
            let start =
                crate::lsp::utf16_to_char_col(&text, diagnostic.range.start.character).min(len);
            let mut end = if diagnostic.range.end.line == diagnostic.range.start.line {
                crate::lsp::utf16_to_char_col(&text, diagnostic.range.end.character).min(len)
            } else {
                len
            };
            if end <= start {
                let chars: Vec<char> = text.chars().collect();
                end = start;
                while chars
                    .get(end)
                    .is_some_and(|c| c.is_alphanumeric() || *c == '_')
                {
                    end = end.saturating_add(1);
                }
                end = end.max(start.saturating_add(1));
            }
            Some(Squiggle {
                line,
                start,
                end,
                severity: diagnostic.severity.unwrap_or(crate::lsp::Severity::Hint),
                message: diagnostic.message.clone(),
            })
        })
        .collect();
    out.sort_by_key(|squiggle| (squiggle.line, squiggle.start));
    out
}

pub type MenuItemRenderer = fn(&mut egui::Ui, &str, &str, &str) -> egui::Response;

pub struct SourceOptions {
    pub font_size: f32,
    pub show_line_numbers: bool,
    pub show_status: bool,
    /// Gutter marks, sorted by row; binary-searched per painted row, so
    /// an empty vec costs nothing on large files.
    pub diagnostics: Vec<DiagnosticMark>,
    /// Underlined diagnostic ranges, sorted (see [`squiggles`]); also
    /// drive the scrollbar error stripe and F8 navigation.
    pub squiggles: Vec<Squiggle>,
    /// Find-bar matches `(line, start, end)` in char columns, sorted, and
    /// the index of the selected one.
    pub find_matches: std::sync::Arc<Vec<(usize, usize, usize)>>,
    pub find_current: Option<usize>,
    /// A popup (completion) owns Up/Down/Enter/Tab/Escape: the editor
    /// leaves those keys in the input for the host to read.
    pub popup_keys: bool,
    /// VCS change markers for the gutter, sorted by `start`.
    pub change_marks: std::sync::Arc<Vec<ChangeMark>>,
    /// Theme colors for added / modified / deleted markers.
    pub change_colors: [Color32; 3],
    /// Gutter annotations by buffer line (blame), or `None` for no
    /// column. Lines past the end get a blank cell.
    pub annotations: Option<std::sync::Arc<Vec<Annotation>>>,
    /// Offer "Annotate with Git Blame" in the context menu: `Some(on)`
    /// when the file is under Git, `on` when annotations are showing.
    pub blame: Option<bool>,
    /// The file as "Copy reference" names it (project-relative path):
    /// copies `reference:line` (`IntelliJ` Copy Reference).
    pub reference: Option<String>,
    /// Inlay hints, sorted by line and column (see [`crate::inlay`]).
    pub inlay_hints: std::sync::Arc<Vec<crate::inlay::InlayHint>>,
    /// Take keyboard focus when nothing else holds it, so an opened file is
    /// immediately typeable (`IntelliJ` focuses the editor on open).
    pub autofocus: bool,
    /// Vim mode: Ctrl-V/Ctrl-H belong to vim in its modes, and printable
    /// keys are vim commands in Normal mode. Off is fully modeless:
    /// printable keys (and Enter/Backspace/…) always edit, and Ctrl-V/H
    /// always paste/prompt.
    pub vim: bool,
    /// A filesystem operation owns this buffer; navigation and copying
    /// stay available, while edits and saves wait for its result.
    pub read_only: bool,
    /// Host menu widgets keep the editor menu consistent with other panes.
    pub menu_style: Option<fn(&mut egui::Style)>,
    pub menu_header: Option<fn(&mut egui::Ui, &str, &str)>,
    pub menu_item: Option<MenuItemRenderer>,
    /// Wrap long source lines inside the pane while keeping logical cursor columns.
    pub soft_wrap: bool,
    /// Set the visible position as a fraction of the scrollable document.
    pub scroll_fraction: Option<f32>,
}

impl Default for SourceOptions {
    fn default() -> Self {
        Self {
            font_size: 13.0,
            show_line_numbers: true,
            show_status: true,
            autofocus: true,
            vim: true,
            read_only: false,
            menu_style: None,
            menu_header: None,
            menu_item: None,
            soft_wrap: false,
            scroll_fraction: None,
            diagnostics: Vec::new(),
            squiggles: Vec::new(),
            find_matches: std::sync::Arc::new(Vec::new()),
            find_current: None,
            popup_keys: false,
            change_marks: std::sync::Arc::new(Vec::new()),
            change_colors: [
                Color32::from_rgb(0x4C, 0x9A, 0x52),
                Color32::from_rgb(0x4A, 0x7F, 0xC1),
                Color32::from_rgb(0x8C, 0x8C, 0x8C),
            ],
            inlay_hints: std::sync::Arc::new(Vec::new()),
            reference: None,
            annotations: None,
            blame: None,
        }
    }
}

pub struct SourceOutcome {
    pub effects: Vec<Effect>,
    /// Painted file area, for fixture targeting and reveal requests.
    pub content_rect: egui::Rect,
    /// A primary or secondary click made this source view the input target.
    pub activated: bool,
    /// True while the editor context menu is open.
    pub context_menu_opened: bool,
    /// Rename action geometry for an optional GUI fixture target.
    pub rename_action_rect: Option<egui::Rect>,
    /// Hover action geometry for an optional GUI fixture target.
    pub hover_action_rect: Option<egui::Rect>,
    /// Command/search row, fixed to the pane bottom when enabled.
    pub footer_rect: Option<egui::Rect>,
    /// Current vertical position in the scrollable document, from 0 to 1.
    pub scroll_fraction: f32,
    /// Number of painted rows after optional source wrapping.
    pub visual_rows: usize,
    /// Screen rect of the caret cell when it was painted, for anchoring
    /// popups (completion) at the caret.
    pub caret_rect: Option<egui::Rect>,
    /// A gutter change marker was clicked: its index in
    /// `SourceOptions::change_marks` and where, for the rollback popup.
    pub change_clicked: Option<(usize, egui::Pos2)>,
    /// A gutter annotation was clicked: its buffer line.
    pub annotation_clicked: Option<usize>,
    /// The context menu asked to show or hide blame annotations.
    pub toggle_blame: bool,
}

/// Keys a host popup owns while `SourceOptions::popup_keys` is set.
fn popup_key(key: egui::Key) -> bool {
    matches!(
        key,
        egui::Key::ArrowUp
            | egui::Key::ArrowDown
            | egui::Key::Enter
            | egui::Key::Tab
            | egui::Key::Escape
            | egui::Key::PageUp
            | egui::Key::PageDown
    )
}

/// Render an editable source view. Returns engine effects (`Save`/`Quit`/…)
/// for the session layer to act on. Highlighting derives from the buffer
/// revision into the caller's [`HighlightCache`]: frames without edits
/// repaint from the cache, edits re-derive once.
pub struct PieceInput<'a> {
    col: usize,
    piece: &'a str,
    font: &'a FontId,
    color: Color32,
}

pub struct SelectionSpanForLineInput {
    a: Cursor,
    b: Cursor,
    line: usize,
    line_len: usize,
    inclusive: bool,
}

pub struct ShowSourceInput<'a> {
    pub options: &'a SourceOptions,
    pub highlight: &'a mut HighlightCache,
    pub language: Language,
    pub id: &'a str,
}

pub fn show_source<B: Buffer>(
    ui: &mut egui::Ui,
    doc: &mut B,
    engine: &mut VimEngine,
    input: ShowSourceInput<'_>,
) -> SourceOutcome {
    let ShowSourceInput {
        options,
        highlight,
        language,
        id,
    } = input;
    let font = FontId::monospace(options.font_size);
    let italic_font = FontId::new(
        options.font_size,
        egui::FontFamily::Name(ITALIC_FAMILY.into()),
    );
    let bold_font = FontId::new(
        options.font_size,
        egui::FontFamily::Name(BOLD_FAMILY.into()),
    );
    let (advance, row_height) = ui.fonts_mut(|fonts| {
        (
            fonts.glyph_width(&font, 'm').max(1.0),
            fonts.row_height(&font).max(1.0),
        )
    });
    // Blame column: as wide as the widest annotation (capped), between
    // the change band and the line numbers.
    let annotation_w = match &options.annotations {
        Some(annotations) if options.show_line_numbers => {
            let cells = annotations
                .iter()
                .map(|annotation| annotation.text.chars().count())
                .max()
                .unwrap_or(0)
                .clamp(8, ANNOTATION_CELLS);
            cells.saturating_add(1) as f32 * advance
        }
        _ => 0.0,
    };
    let gutter = if options.show_line_numbers {
        let digits = doc.line_count().max(1).to_string().len() as f32;
        (digits + 2.0) * advance + annotation_w
    } else {
        0.0
    };
    let mut annotation_clicked = None;
    let mut toggle_blame = false;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "Pane geometry becomes a bounded number of monospace cells."
    )]
    let wrap_columns = options.soft_wrap.then(|| {
        ((ui.available_width() - gutter - 16.0).max(advance) / advance)
            .floor()
            .max(1.0) as usize
    });
    let mut effects = Vec::new();
    // Vim off is modeless: IDE caret and selection semantics.
    engine.set_modeless(!options.vim);

    // Highlight derive, once per buffer revision: unchanged frames repaint
    // straight from the cache without rebuilding the text. The row layout
    // is cached on the same key, so an idle frame does no per-line work.
    let dark = ui.visuals().dark_mode;
    if !highlight.is_current(language, dark, doc.revision()) {
        let mut text = String::new();
        for line in 0..doc.line_count() {
            text.push_str(&doc.line_text(line));
            text.push('\n');
        }
        highlight.ensure(crate::highlight::EnsureInput {
            language,
            dark,
            revision: doc.revision(),
            text: &text,
        });
    }
    highlight.ensure_rows(doc, wrap_columns);
    // Fold changes wait until the row layout is no longer borrowed; they
    // re-lay the rows next frame.
    let mut fold_actions: Vec<IdeEdit> = Vec::new();
    let mut fold_toggles: Vec<usize> = Vec::new();
    // Direction of a vertical caret move this frame (`true` = down), so a
    // caret landing in a folded block hops over it instead of opening it.
    let mut vertical_move: Option<bool> = None;
    let folds = highlight.folds();
    let visual_rows = highlight.rows();
    let row_count = visual_rows.len();
    let content_height = visual_rows.len().max(1) as f32 * row_height;

    // Keep one footer row outside the source viewport. The viewport must
    // fill its allotted height even when the file has only a few lines;
    // otherwise the command/search prompt follows the end of the file.
    let footer_height = if options.show_status { row_height } else { 0.0 };
    let max_scroll =
        (ui.available_height() - footer_height - ui.spacing().item_spacing.y).max(row_height);
    // Long lines pan instead of clipping: without soft wrap the content
    // keeps its full width and the scroll area owns both axes.
    let h_scroll = !options.soft_wrap;
    let content_width =
        (gutter + highlight.max_cells() as f32 * advance + advance).max(ui.available_width());
    let mut content_rect = egui::Rect::NOTHING;
    let mut context_menu_opened = false;
    let mut rename_action_rect = None;
    let mut hover_action_rect = None;
    let mut caret_rect = None;
    let mut change_clicked = None;
    let mut activated = false;
    let focus_id = source_focus_id(id);
    // Set while a drag selection owns the pointer: the next plain click
    // collapses it instead of extending it like a `v` selection would.
    let mouse_drag_key = focus_id.with("native-source-drag");
    let mut scroll = if h_scroll {
        egui::ScrollArea::both()
    } else {
        egui::ScrollArea::vertical()
    };
    scroll = scroll
        .id_salt(("native-source", id))
        .max_height(max_scroll)
        .auto_shrink([!h_scroll, false]);
    if let Some(fraction) = options.scroll_fraction {
        scroll = scroll.vertical_scroll_offset(
            (content_height - max_scroll).max(0.0) * fraction.clamp(0.0, 1.0),
        );
    }
    let scroll_output = scroll.show(ui, |ui| {
        let width = if h_scroll {
            content_width
        } else {
            ui.available_width().max(gutter + advance)
        };
        let (response, painter) = ui.allocate_painter(
            Vec2::new(width, content_height.max(max_scroll)),
            Sense::click_and_drag(),
        );
        // I-beam over file text; the default Sense::click pointer reads
        // as "clickable chrome" instead of an editor.
        let response = response.on_hover_cursor(egui::CursorIcon::Text);
        content_rect = response.rect;
        activated = response.clicked() || response.secondary_clicked();
        ui.ctx().data_mut(|data| {
            data.insert_temp(focus_id.with("native-source"), true);
        });
        // Register the stable keyboard target each frame. Otherwise egui
        // drops it as a disappeared widget after the focus request.
        let focus = ui.interact(response.rect, focus_id, Sense::focusable_noninteractive());
        focus.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::TextInput, ui.is_enabled(), "Source editor")
        });
        // Pointer mapping: painted row and display cell back to a buffer
        // position. A pointer below the last row clamps to it, so a drag
        // that leaves the pane keeps selecting.
        let cursor_at = |pos: egui::Pos2| -> Option<(usize, usize)> {
            let origin = response.rect.min;
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "UI coordinate")]
            let row_index = ((pos.y - origin.y) / row_height).floor().max(0.0) as usize;
            #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "UI coordinate")]
            let cells = ((pos.x - origin.x - gutter) / advance).round().max(0.0) as usize;
            // Wide chars occupy two cells: resolve the display cell back to
            // a char column on the hit row.
            let row = visual_rows.get(row_index).or_else(|| visual_rows.last())?;
            let hints = row_hints(&options.inlay_hints, row);
            let geometry = RowCells {
                text: &row.text,
                hints: &hints,
            };
            Some((row.line, row.start.saturating_add(geometry.col_at(cells))))
        };
        let caret_before = engine.cursor();
        let click_modifiers = ui.input(|input| input.modifiers);
        // A click in the change band opens that change instead of moving
        // the caret.
        let band_click = response
            .clicked()
            .then(|| response.interact_pointer_pos())
            .flatten()
            .filter(|pos| options.show_line_numbers && pos.x < response.rect.left() + CHANGE_BAND)
            .and_then(|pos| {
                let (line, _) = cursor_at(pos)?;
                options
                    .change_marks
                    .iter()
                    .position(|mark| {
                        (mark.start..mark.end.max(mark.start.saturating_add(1))).contains(&line)
                    })
                    .map(|index| (index, pos))
            });
        // A click on a fold arrow (the gutter's last cell) toggles it.
        let fold_click = response
            .clicked()
            .then(|| response.interact_pointer_pos())
            .flatten()
            .filter(|pos| {
                options.show_line_numbers
                    && pos.x >= response.rect.left() + gutter - advance
                    && pos.x < response.rect.left() + gutter
            })
            .and_then(&cursor_at)
            .map(|(line, _)| line)
            .filter(|line| folds.region_at(*line).is_some());
        if let Some(line) = fold_click {
            fold_toggles.push(line);
        } else if let Some(click) = band_click {
            change_clicked = Some(click);
        } else if response.clicked()
            && !options.vim
            && click_modifiers.alt
            && click_modifiers.shift
            && let Some((line, col)) = response.interact_pointer_pos().and_then(&cursor_at)
        {
            // Alt+Shift+click: add or remove a caret (`IntelliJ`).
            ui.memory_mut(|memory| memory.request_focus(focus_id));
            engine.toggle_caret_at(doc, line, col);
        } else if response.clicked()
            || (response.secondary_clicked() && engine.selection().is_none())
        {
            ui.memory_mut(|memory| memory.request_focus(focus_id));
            if response.clicked() {
                engine.clear_extra_carets();
            }
            let extend = response.clicked() && click_modifiers.shift;
            // A plain click after a drag collapses that selection instead of
            // extending it the way a `v` selection does; modeless clicks
            // always collapse (IDE carets never extend without Shift).
            let dragged = ui
                .ctx()
                .data_mut(|data| data.remove_temp::<bool>(mouse_drag_key).unwrap_or(false));
            if !extend && (dragged || !options.vim) {
                engine.clear_mouse_selection(doc);
            }
            if let Some(pos) = response.interact_pointer_pos()
                && let Some((line, col)) = cursor_at(pos)
            {
                if extend {
                    engine.extend_selection_to(doc, line, col);
                } else {
                    effects.extend(engine.place_cursor(doc, line, col));
                    // Cmd/Ctrl+click: go to the declaration under the
                    // pointer (`IntelliJ`), now that the caret sits on it.
                    if response.clicked() && click_modifiers.command {
                        effects.push(Effect::GotoDefinition);
                    }
                }
            }
        }
        if response.double_clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && let Some((line, col)) = cursor_at(pos)
        {
            engine.select_word_at(doc, line, col);
            ui.memory_mut(|memory| memory.request_focus(focus_id));
        }
        if response.triple_clicked()
            && let Some(pos) = response.interact_pointer_pos()
            && let Some((line, _)) = cursor_at(pos)
        {
            engine.select_line_at(doc, line);
        }
        if response.drag_started() {
            // The press point, not the position where egui decided this was
            // a drag: the anchor must be where the button went down.
            let anchor = ui.input(|input| input.pointer.press_origin());
            if let Some((line, col)) = anchor.and_then(&cursor_at) {
                engine.begin_mouse_selection(doc, line, col);
                ui.ctx()
                    .data_mut(|data| data.insert_temp(mouse_drag_key, true));
                ui.memory_mut(|memory| memory.request_focus(focus_id));
            }
        } else if response.dragged()
            && let Some(pos) = ui.input(|input| input.pointer.interact_pos())
            && let Some((line, col)) = cursor_at(pos)
        {
            engine.extend_mouse_selection(doc, line, col);
        }
        let mut popup = egui::Popup::context_menu(&response);
        if let Some(style) = options.menu_style {
            popup = popup.style(style);
        }
        popup.show(|ui| {
            if let Some(header) = options.menu_header {
                header(ui, id, id);
            }
            let item = |ui: &mut egui::Ui, label: &str, icon: &str, shortcut: &str| match options
                .menu_item
            {
                Some(draw) => draw(ui, label, icon, shortcut),
                None => ui.button(label),
            };
            let key = if cfg!(target_os = "macos") {
                "⌘"
            } else {
                "Ctrl+"
            };
            let shift = if cfg!(target_os = "macos") {
                "⇧⌘"
            } else {
                "Ctrl+Shift+"
            };
            if item(ui, "Copy", "Copy", &format!("{key}C")).clicked() {
                let text = engine.copy_text(doc);
                if !text.is_empty() {
                    ui.ctx().copy_text(text);
                }
                ui.close();
            }
            if let Some(reference) = &options.reference
                && item(
                    ui,
                    "Copy reference",
                    "Copy",
                    if cfg!(target_os = "macos") {
                        "⌥⇧⌘C"
                    } else {
                        "Ctrl+Alt+Shift+C"
                    },
                )
                .clicked()
            {
                ui.ctx().copy_text(caret_reference(reference, engine));
                ui.close();
            }
            if ui
                .add_enabled_ui(!options.read_only && engine.selection().is_some(), |ui| {
                    item(ui, "Cut", "Eraser", &format!("{key}X"))
                })
                .inner
                .clicked()
            {
                let text = engine.cut(doc);
                if !text.is_empty() {
                    ui.ctx().copy_text(text);
                }
                ui.close();
            }
            if ui
                .add_enabled_ui(!options.read_only, |ui| {
                    item(ui, "Paste", "ClipboardPaste", &format!("{key}V"))
                })
                .inner
                .clicked()
            {
                ui.memory_mut(|memory| memory.request_focus(focus_id));
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::RequestPaste);
                ui.close();
            }
            if item(ui, "Select all", "TextSelect", &format!("{key}A")).clicked() {
                engine.select_all(doc);
                ui.close();
            }
            ui.separator();
            if ui
                .add_enabled_ui(!options.read_only, |ui| {
                    item(ui, "Undo", "History", &format!("{key}Z"))
                })
                .inner
                .clicked()
            {
                engine.undo(doc);
                ui.close();
            }
            if ui
                .add_enabled_ui(!options.read_only, |ui| {
                    item(ui, "Redo", "RefreshCw", &format!("{shift}Z"))
                })
                .inner
                .clicked()
            {
                engine.redo(doc);
                ui.close();
            }
            ui.separator();
            if item(ui, "Find in file", "Search", &format!("{key}F")).clicked() {
                engine.begin_search();
                ui.close();
            }
            if ui
                .add_enabled_ui(!options.read_only, |ui| {
                    item(ui, "Replace in file", "Pencil", "")
                })
                .inner
                .clicked()
            {
                engine.begin_ex("%s/");
                ui.close();
            }
            if ui
                .add_enabled_ui(!options.read_only, |ui| {
                    item(ui, "Save", "Save", &format!("{key}S"))
                })
                .inner
                .clicked()
            {
                effects.push(Effect::Save);
                ui.close();
            }
            if let Some(on) = options.blame {
                ui.separator();
                let label = if on {
                    "Close annotations"
                } else {
                    "Annotate with Git Blame"
                };
                if item(ui, label, "History", "").clicked() {
                    toggle_blame = true;
                    ui.close();
                }
            }
            ui.separator();
            if item(ui, "Copy file path", "Copy", "").clicked() {
                ui.ctx().copy_text(id.to_owned());
                ui.close();
            }
            if item(ui, "Open file…", "File", &format!("{key}O")).clicked() {
                effects.push(Effect::OpenFilePicker);
                ui.close();
            }
            let lsp_supported = crate::lsp::server_language(language).is_some();
            if lsp_supported {
                ui.separator();
                if item(ui, "Go to definition", "ExternalLink", "").clicked() {
                    effects.push(Effect::GotoDefinition);
                    ui.close();
                }
                if item(ui, "Find references", "ListTree", "").clicked() {
                    effects.push(Effect::FindReferences);
                    ui.close();
                }
                let call_keys = if cfg!(target_os = "macos") {
                    "⌃⌥H"
                } else {
                    "Ctrl+Alt+H"
                };
                if item(ui, "Call hierarchy", "ListTree", call_keys).clicked() {
                    effects.push(Effect::Hierarchy { types: false });
                    ui.close();
                }
                if item(ui, "Type hierarchy", "ListTree", "").clicked() {
                    effects.push(Effect::Hierarchy { types: true });
                    ui.close();
                }
                let rename_button = ui
                    .add_enabled_ui(!options.read_only, |ui| {
                        item(ui, "Rename symbol…", "Pencil", "")
                    })
                    .inner;
                rename_action_rect = Some(rename_button.rect);
                if rename_button.clicked() {
                    effects.push(Effect::RenameSymbol);
                    ui.close();
                }
                let hover_button = item(ui, "Show hover", "Info", "");
                hover_action_rect = Some(hover_button.rect);
                if hover_button.clicked() {
                    effects.push(Effect::Hover);
                    ui.close();
                }
            }
        });
        context_menu_opened = response.context_menu_opened();
        if options.autofocus && ui.memory(|memory| memory.focused().is_none()) {
            ui.memory_mut(|memory| memory.request_focus(focus_id));
        }
        let focused = ui.memory(|memory| memory.has_focus(focus_id))
            && !ui.input(|input| input.pointer.any_pressed() && !response.contains_pointer());
        if focused {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    focus_id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                );
            });
        }
        let origin = response.rect.min;

        // Selection in line/col space, ordered. A visual block
        // paints its rectangle instead: (top, bottom, left, right+1).
        let selection = engine.selection().map(|(a, b)| {
            let (a_idx, b_idx) = (line_col_ord(a), line_col_ord(b));
            if a_idx <= b_idx { (a, b) } else { (b, a) }
        });
        let block = engine.block_span();
        // Extra carets (multi-caret editing) paint like the primary one.
        let extra_carets = engine.extra_carets().to_vec();

        let cursor = engine.cursor();
        let cursor_visible = !matches!(engine.mode(), Mode::Command);
        // Occurrences of the identifier at the caret, and the bracket pair
        // around it: both paint as quiet backgrounds/outlines.
        let caret_word = (cursor_visible && selection.is_none())
            .then(|| engine.word_at_caret(doc))
            .flatten();
        let brackets = cursor_visible
            .then(|| engine.matching_bracket(doc))
            .flatten();
        let occurrence_fill = ui.visuals().selection.bg_fill.gamma_multiply(0.45);
        let bracket_stroke = egui::Stroke::new(1.0, ui.visuals().weak_text_color());
        // Only the rows inside the clip rect: a long file's off-screen rows
        // cost nothing per frame.
        let clip = painter.clip_rect();
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "Row indices from UI coordinates.")]
        let (first_row, last_row) = (
            ((clip.top() - origin.y) / row_height).floor().max(0.0) as usize,
            (((clip.bottom() - origin.y) / row_height).ceil().max(0.0) as usize)
                .saturating_add(1)
                .min(visual_rows.len()),
        );
        // Indent guides: blank lines keep the guides of the line above.
        let guide_color = ui
            .visuals()
            .widgets
            .noninteractive
            .bg_stroke
            .color
            .gamma_multiply(0.6);
        let mut guide_cells = 0usize;
        for (visual_line, visual) in visual_rows
            .iter()
            .enumerate()
            .skip(first_row)
            .take(last_row.saturating_sub(first_row))
        {
            let line = visual.line;
            let hints = row_hints(&options.inlay_hints, visual);
            let geometry = RowCells {
                text: &visual.text,
                hints: &hints,
            };
            let y = origin.y + visual_line as f32 * row_height;
            if y + row_height < painter.clip_rect().top() || y > painter.clip_rect().bottom() {
                continue;
            }
            if visual.start == 0 && options.show_line_numbers {
                // VCS change band: a bar beside changed lines, a wedge
                // where lines were deleted.
                let at = options
                    .change_marks
                    .partition_point(|mark| mark.end.max(mark.start.saturating_add(1)) <= line);
                if let Some(mark) = options.change_marks.get(at)
                    && mark.start <= line
                {
                    let [added, modified, deleted] = options.change_colors;
                    match mark.kind {
                        ChangeKind::Added | ChangeKind::Modified => {
                            painter.rect_filled(
                                Rect::from_min_size(
                                    Pos2::new(origin.x + 3.0, y),
                                    Vec2::new(3.0, row_height),
                                ),
                                0.0,
                                if mark.kind == ChangeKind::Added {
                                    added
                                } else {
                                    modified
                                },
                            );
                        }
                        ChangeKind::Deleted => {
                            painter.add(egui::Shape::convex_polygon(
                                vec![
                                    Pos2::new(origin.x + 2.0, y - 3.0),
                                    Pos2::new(origin.x + 7.0, y),
                                    Pos2::new(origin.x + 2.0, y + 3.0),
                                ],
                                deleted,
                                egui::Stroke::NONE,
                            ));
                        }
                    }
                }
            }
            if visual.start == 0 {
                // Worst severity on the row, for both the line-number tint
                // and the gutter strip: errors shout, warnings caution,
                // lower severities stay quiet.
                let mark = options
                    .diagnostics
                    .binary_search_by(|mark| mark.line.cmp(&line))
                    .ok()
                    .and_then(|at| options.diagnostics.get(at));
                if let Some(mark) = mark {
                    let color = match mark.severity {
                        crate::lsp::Severity::Error => ui.visuals().error_fg_color,
                        crate::lsp::Severity::Warning => ui.visuals().warn_fg_color,
                        _ => ui.visuals().weak_text_color(),
                    };
                    // A 2px strip at the pane edge survives a scan of the
                    // line numbers, where a color alone does not.
                    painter.rect_filled(
                        Rect::from_min_size(Pos2::new(origin.x, y), Vec2::new(2.0, row_height)),
                        0.0,
                        color,
                    );
                }
                if options.show_line_numbers && folds.region_at(line).is_some() {
                    let folded = folds.is_folded(line);
                    let pointer_in_gutter = ui
                        .input(|input| input.pointer.hover_pos())
                        .is_some_and(|pos| {
                            response.rect.contains(pos) && pos.x < origin.x + gutter
                        });
                    if folded || pointer_in_gutter {
                        let center =
                            Pos2::new(origin.x + gutter - advance * 0.5, y + row_height * 0.5);
                        let size = (advance * 0.35).max(3.0);
                        let points = if folded {
                            vec![
                                Pos2::new(center.x - size * 0.5, center.y - size),
                                Pos2::new(center.x + size * 0.7, center.y),
                                Pos2::new(center.x - size * 0.5, center.y + size),
                            ]
                        } else {
                            vec![
                                Pos2::new(center.x - size, center.y - size * 0.5),
                                Pos2::new(center.x + size, center.y - size * 0.5),
                                Pos2::new(center.x, center.y + size * 0.7),
                            ]
                        };
                        painter.add(egui::Shape::convex_polygon(
                            points,
                            ui.visuals().weak_text_color(),
                            egui::Stroke::NONE,
                        ));
                    }
                }
                if let Some(annotations) = options.annotations.as_ref().filter(|_| annotation_w > 0.0)
                    && let Some(annotation) = annotations.get(line)
                {
                    let cell = Rect::from_min_size(
                        Pos2::new(origin.x + CHANGE_BAND, y),
                        Vec2::new(annotation_w - advance * 0.5, row_height),
                    );
                    if annotation.group % 2 == 1 {
                        painter.rect_filled(cell, 0.0, ui.visuals().faint_bg_color);
                    }
                    let text: String = annotation.text.chars().take(ANNOTATION_CELLS).collect();
                    painter.text(
                        cell.left_top(),
                        egui::Align2::LEFT_TOP,
                        text,
                        font.clone(),
                        ui.visuals().weak_text_color(),
                    );
                    if !annotation.hover.is_empty() {
                        let hit = ui
                            .interact(cell, focus_id.with(("annotation", line)), Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(&annotation.hover);
                        if hit.clicked() {
                            annotation_clicked = Some(line);
                        }
                    }
                }
                if options.show_line_numbers {
                    let number_color = mark.map_or_else(
                        || ui.visuals().weak_text_color(),
                        |mark| match mark.severity {
                            crate::lsp::Severity::Error => ui.visuals().error_fg_color,
                            crate::lsp::Severity::Warning => ui.visuals().warn_fg_color,
                            _ => ui.visuals().weak_text_color(),
                        },
                    );
                    painter.text(
                        Pos2::new(origin.x + gutter - advance, y),
                        egui::Align2::RIGHT_TOP,
                        format!("{}", line.saturating_add(1)),
                        font.clone(),
                        number_color,
                    );
                }
            }
            let text = &visual.text;
            if visual.start == 0 {
                let lead: String = text
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect();
                if lead.chars().count() < text.chars().count() {
                    guide_cells = display_x(&lead, lead.chars().count());
                }
                // One guide per indent level the line sits inside.
                for cell in (0..guide_cells).step_by(crate::rows::TAB_WIDTH) {
                    let x = origin.x + gutter + cell as f32 * advance + 1.0;
                    painter.vline(x, y..=y + row_height, egui::Stroke::new(1.0, guide_color));
                }
            }
            // Selection background for this row.
            let span = match block {
                Some((top, bottom, left, right)) if line >= top && line <= bottom => {
                    let len = visual.line_len;
                    (left < right.min(len)).then(|| (left, right.min(len)))
                }
                _ => selection.and_then(|(a, b)| {
                    selection_span_for_line(SelectionSpanForLineInput { a, b, line, line_len: visual.line_len, inclusive: options.vim })
                }),
            };
            if let Some((start_col, end_col)) = span
                && let (start_col, end_col) = (start_col.max(visual.start), end_col.min(visual.end))
                && start_col < end_col
            {
                // Char columns to display cells: wide chars shift every
                // cell after them.
                let start_cell = geometry.char_x(start_col.saturating_sub(visual.start));
                let cells = geometry
                    .caret_x(end_col.saturating_sub(visual.start))
                    .saturating_sub(start_cell);
                let start_x = origin.x + gutter + start_cell as f32 * advance;
                let span_w = cells as f32 * advance;
                painter.rect_filled(
                    Rect::from_min_size(Pos2::new(start_x, y), Vec2::new(span_w, row_height)),
                    0.0,
                    ui.visuals().selection.bg_fill,
                );
            }
            for caret in &extra_carets {
                if let Some((a, b)) = caret.selection()
                    && let Some((start_col, end_col)) =
                        selection_span_for_line(SelectionSpanForLineInput { a, b, line, line_len: visual.line_len, inclusive: false })
                    && let (start_col, end_col) =
                        (start_col.max(visual.start), end_col.min(visual.end))
                    && start_col < end_col
                {
                    let start_cell = geometry.char_x(start_col.saturating_sub(visual.start));
                    let cells = geometry
                        .caret_x(end_col.saturating_sub(visual.start))
                        .saturating_sub(start_cell);
                    let rect = Rect::from_min_size(
                        Pos2::new(origin.x + gutter + start_cell as f32 * advance, y),
                        Vec2::new(cells as f32 * advance, row_height),
                    );
                    painter.rect_filled(rect, 0.0, ui.visuals().selection.bg_fill);
                }
            }
            let cell_rect = |col: usize, cells: usize| {
                let local = col.saturating_sub(visual.start);
                let start = geometry.char_x(local);
                let end = geometry.caret_x(local.saturating_add(cells));
                Rect::from_min_size(
                    Pos2::new(origin.x + gutter + start as f32 * advance, y),
                    Vec2::new(
                        end.saturating_sub(start).max(1) as f32 * advance,
                        row_height,
                    ),
                )
            };
            if let Some(word) = caret_word.as_deref() {
                for (start, len) in word_occurrences(text, word) {
                    let rect = cell_rect(visual.start.saturating_add(start), len);
                    painter.rect_filled(rect, 2.0, occurrence_fill);
                }
            }
            // Find-bar matches: all softly, the selected one strongly.
            let from = options
                .find_matches
                .partition_point(|(match_line, _, _)| *match_line < line);
            for (index, (_, start, end)) in options
                .find_matches
                .iter()
                .enumerate()
                .skip(from)
                .take_while(|(_, (match_line, _, _))| *match_line == line)
            {
                let (start, end) = ((*start).max(visual.start), (*end).min(visual.end));
                if start < end {
                    let alpha = if options.find_current == Some(index) {
                        0.9
                    } else {
                        0.4
                    };
                    painter.rect_filled(
                        cell_rect(start, end.saturating_sub(start)),
                        2.0,
                        ui.visuals().warn_fg_color.gamma_multiply(alpha),
                    );
                }
            }
            if let Some((open, close)) = brackets {
                for end in [open, close] {
                    if end.line == line && end.col >= visual.start && end.col < visual.end {
                        painter.rect_stroke(
                            cell_rect(end.col, 1),
                            2.0,
                            bracket_stroke,
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            }
            let cursor_here = cursor_visible && visual.owns_cursor(cursor.line, cursor.col);
            let local_col = cursor.col.saturating_sub(visual.start);
            let cell = cursor_here.then(|| text.chars().nth(local_col)).flatten();
            // A bar caret sits before a hint; a block covers the char after it.
            let bar = engine.mode() == Mode::Insert || !options.vim;
            let cursor_cells = cursor_here.then(|| {
                if bar {
                    geometry.caret_x(local_col)
                } else {
                    geometry.char_x(local_col)
                }
            });
            let spans: Vec<Span> = highlight
                .line(line)
                .iter()
                .filter_map(|span| {
                    let start = span.start.max(visual.start);
                    let end = span.end.min(visual.end);
                    (start < end).then_some(Span {
                        start: start.saturating_sub(visual.start),
                        end: end.saturating_sub(visual.start),
                        color: span.color,
                        style: span.style,
                    })
                })
                .collect();
            RowPaint {
                painter: &painter,
                text,
                hints: &hints,
                spans: &spans,
                x: origin.x + gutter,
                y,
                advance,
                font: &font,
                italic_font: &italic_font,
                bold_font: &bold_font,
                plain: ui.visuals().text_color(),
            }
            .paint();
            if visual.folded {
                // `⋯` placeholder after a folded region's first line.
                let at = geometry.char_x(text.chars().count()).saturating_add(1);
                let rect = Rect::from_min_size(
                    Pos2::new(origin.x + gutter + at as f32 * advance, y + 1.0),
                    Vec2::new(advance * 3.0, row_height - 2.0),
                );
                painter.rect_filled(rect, 3.0, ui.visuals().faint_bg_color);
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "⋯",
                    font.clone(),
                    ui.visuals().weak_text_color(),
                );
                let hit = ui.interact(
                    rect,
                    focus_id.with(("fold-placeholder", line)),
                    Sense::click(),
                );
                if hit.on_hover_text("Click to unfold").clicked() {
                    fold_toggles.push(line);
                }
            }
            // Diagnostic underlines on this row, with their message on hover.
            let first = options
                .squiggles
                .partition_point(|squiggle| squiggle.line < line);
            for (index, squiggle) in options
                .squiggles
                .iter()
                .enumerate()
                .skip(first)
                .take_while(|(_, squiggle)| squiggle.line == line)
            {
                let (start, end) = (
                    squiggle.start.max(visual.start),
                    squiggle
                        .end
                        .min(visual.end.max(visual.start.saturating_add(1))),
                );
                if start >= end {
                    continue;
                }
                let rect = cell_rect(start, end.saturating_sub(start));
                let color = match squiggle.severity {
                    crate::lsp::Severity::Error => ui.visuals().error_fg_color,
                    crate::lsp::Severity::Warning => ui.visuals().warn_fg_color,
                    _ => ui.visuals().weak_text_color(),
                };
                paint_squiggle(&painter, rect, color);
                ui.interact(rect, focus_id.with(("squiggle", index)), Sense::hover())
                    .on_hover_text(&squiggle.message);
            }
            for caret in &extra_carets {
                if cursor_visible && visual.owns_cursor(caret.cursor.line, caret.cursor.col) {
                    let cells = geometry.caret_x(caret.cursor.col.saturating_sub(visual.start));
                    let x = origin.x + gutter + cells as f32 * advance;
                    painter.rect_filled(
                        Rect::from_min_size(Pos2::new(x, y), Vec2::new(2.0, row_height)),
                        0.0,
                        ui.visuals().text_color(),
                    );
                }
            }
            if let Some(cells) = cursor_cells {
                // A tab's block covers the cells up to its stop.
                let cell_cells = if cell.is_some() {
                    geometry
                        .caret_x(local_col.saturating_add(1))
                        .saturating_sub(cells)
                } else {
                    1
                };
                let x = origin.x + gutter + cells as f32 * advance;
                caret_rect = Some(Rect::from_min_size(
                    Pos2::new(x, y),
                    Vec2::new(advance, row_height),
                ));
                // Modeless carets are always a bar: they sit between chars.
                if bar {
                    painter.rect_filled(
                        Rect::from_min_size(Pos2::new(x, y), Vec2::new(2.0, row_height)),
                        0.0,
                        ui.visuals().text_color(),
                    );
                } else {
                    let cursor_w = cell_cells as f32 * advance;
                    // Opaque caret in the text color (IntelliJ's block
                    // caret), not the muted selection blue: a #214283
                    // block on the #1E1F22 surface is nearly invisible.
                    painter.rect_filled(
                        Rect::from_min_size(
                            Pos2::new(x, y),
                            Vec2::new(cursor_w.max(2.0), row_height),
                        ),
                        0.0,
                        ui.visuals().text_color(),
                    );
                    // Keep the covered cell legible on the block cursor.
                    if let Some(cell) = cell {
                        painter.text(
                            Pos2::new(x, y),
                            egui::Align2::LEFT_TOP,
                            cell.to_string(),
                            font.clone(),
                            ui.visuals().window_fill,
                        );
                    }
                }
            }
        }

        // Sticky lines: headers of the blocks enclosing the top visible
        // row stay pinned while their first line is scrolled away.
        if let Some(top) = visual_rows.get(first_row).filter(|_| first_row > 0) {
            let headers = folds.enclosing(top.line);
            let headers: Vec<(usize, usize)> =
                headers.iter().rev().take(3).rev().copied().collect();
            for (index, (first, _)) in headers.iter().enumerate() {
                let y = clip.top() + index as f32 * row_height;
                let band = Rect::from_min_max(
                    Pos2::new(origin.x, y),
                    Pos2::new(clip.right(), y + row_height),
                );
                painter.rect_filled(band, 0.0, ui.visuals().extreme_bg_color);
                if options.show_line_numbers {
                    painter.text(
                        Pos2::new(origin.x + gutter - advance, y),
                        egui::Align2::RIGHT_TOP,
                        format!("{}", first.saturating_add(1)),
                        font.clone(),
                        ui.visuals().weak_text_color(),
                    );
                }
                let header = doc.line_text(*first);
                RowPaint {
                    painter: &painter,
                    text: &header,
                    hints: &[],
                    spans: highlight.line(*first),
                    x: origin.x + gutter,
                    y,
                    advance,
                    font: &font,
                    italic_font: &italic_font,
                    bold_font: &bold_font,
                    plain: ui.visuals().text_color(),
                }
                .paint();
                let hit = ui
                    .interact(band, focus_id.with(("sticky-line", *first)), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if hit.clicked() {
                    let col = doc
                        .line_text(*first)
                        .chars()
                        .take_while(|c| c.is_whitespace())
                        .count();
                    effects.extend(engine.place_cursor(doc, *first, col));
                    engine.request_scroll();
                }
            }
            if !headers.is_empty() {
                let bottom = clip.top() + headers.len() as f32 * row_height;
                painter.hline(
                    origin.x..=clip.right(),
                    bottom,
                    egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
                );
            }
        }
        if focused && !options.read_only && !options.vim {
            // Multiple carets (`IntelliJ`): Add / Unselect / Select All
            // Occurrences (Ctrl+G, Ctrl+Shift+G, Ctrl+Cmd+G on macOS;
            // Alt+J, Alt+Shift+J, Ctrl+Alt+Shift+J elsewhere), and Escape
            // back to one caret.
            let mac = cfg!(target_os = "macos");
            let mut occurrence: Vec<(egui::Key, egui::Modifiers, u8)> = Vec::new();
            ui.input(|input| {
                for event in &input.events {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } = event
                    {
                        let chord = if mac {
                            (*key == egui::Key::G && modifiers.ctrl && !modifiers.alt)
                                .then_some(match (modifiers.shift, modifiers.mac_cmd) {
                                    (false, false) => Some(0),
                                    (true, false) => Some(1),
                                    (false, true) => Some(2),
                                    _ => None,
                                })
                                .flatten()
                        } else {
                            (*key == egui::Key::J && modifiers.alt)
                                .then_some(match (modifiers.shift, modifiers.ctrl) {
                                    (false, false) => Some(0),
                                    (true, false) => Some(1),
                                    (true, true) => Some(2),
                                    _ => None,
                                })
                                .flatten()
                        };
                        if let Some(kind) = chord {
                            occurrence.push((*key, *modifiers, kind));
                        } else if *key == egui::Key::Escape
                            && !engine.extra_carets().is_empty()
                            && !(options.popup_keys && popup_key(*key))
                        {
                            occurrence.push((*key, *modifiers, 3));
                        }
                    }
                }
            });
            for (key, modifiers, kind) in occurrence {
                match kind {
                    0 => {
                        engine.add_next_occurrence(doc);
                    }
                    1 => engine.remove_last_occurrence(),
                    2 => engine.add_all_occurrences(doc),
                    _ => engine.clear_extra_carets(),
                }
                ui.input_mut(|input| {
                    input.consume_key(modifiers, key);
                    // An Alt chord may also arrive as typed text.
                    input.events.retain(|event| {
                        !matches!(event, egui::Event::Text(text) if text.chars().count() == 1 && key != egui::Key::Escape && modifiers.alt)
                    });
                });
            }
        }
        if focused && !options.read_only {
            // IDE shortcuts first: copy/cut/paste/select/undo work in
            // every mode (IntelliJ-style), then plain vim keys. Ctrl-V
            // and Ctrl-H detour to vim where it owns them.
            let mut actions: Vec<(egui::Key, egui::Modifiers, IdeAction)> = Vec::new();
            let mut vim_ctrls: Vec<(egui::Key, egui::Modifiers, VKey)> = Vec::new();
            let mut pastes: Vec<String> = Vec::new();
            ui.input(|input| {
                for event in &input.events {
                    match event {
                        egui::Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } => {
                            // Control is independent of Command on macOS.
                            // Ctrl-Q is Vim's block-selection alias for backends
                            // that reserve Ctrl-V for clipboard paste.
                            if matches!(key, egui::Key::V | egui::Key::Q)
                                && modifiers.ctrl
                                && !modifiers.mac_cmd
                                && !modifiers.alt
                                && !modifiers.shift
                                && ctrl_v_is_block(options.vim, engine.mode())
                            {
                                vim_ctrls.push((*key, *modifiers, VKey::CtrlV));
                            } else if let Some(action) = ide_shortcut(*key, *modifiers) {
                                actions.push((*key, *modifiers, action));
                            }
                        }
                        egui::Event::Paste(text) => pastes.push(text.clone()),
                        _ => {}
                    }
                }
            });
            // Vim-owned Ctrl chords become engine keys before IDE
            // actions run, so the mode they observe is current.
            actions.retain(|(key, modifiers, action)| {
                let mode = engine.mode();
                match action {
                    IdeAction::Replace
                        if *key == egui::Key::H && ctrl_h_is_motion(options.vim, mode) =>
                    {
                        vim_ctrls.push((*key, *modifiers, VKey::Backspace));
                        false
                    }
                    _ => true,
                }
            });
            for (key, modifiers, action) in actions {
                match action {
                    IdeAction::Copy => {
                        let text = if engine.extra_carets().is_empty() {
                            engine.copy_text(doc)
                        } else {
                            engine.caret_texts(doc).join("\n")
                        };
                        if !text.is_empty() {
                            ui.ctx().copy_text(text);
                        }
                    }
                    IdeAction::Cut => {
                        let text = if engine.extra_carets().is_empty() {
                            engine.cut(doc)
                        } else {
                            let texts = engine.caret_texts(doc).join("\n");
                            engine.for_each_caret(doc, |engine, doc| {
                                engine.ide_delete_selection(doc);
                                Vec::new()
                            });
                            texts
                        };
                        if !text.is_empty() {
                            ui.ctx().copy_text(text);
                        }
                    }
                    IdeAction::SelectAll => {
                        engine.clear_extra_carets();
                        engine.select_all(doc);
                    }
                    IdeAction::Undo => {
                        engine.clear_extra_carets();
                        engine.undo(doc);
                    }
                    IdeAction::Redo => {
                        engine.clear_extra_carets();
                        engine.redo(doc);
                    }
                    IdeAction::PasteKey => {}
                    IdeAction::Save => effects.push(Effect::Save),
                    IdeAction::Find if !options.vim => {
                        effects.push(Effect::FindBar { replace: false });
                    }
                    IdeAction::Replace if !options.vim => {
                        effects.push(Effect::FindBar { replace: true });
                    }
                    IdeAction::Find => engine.begin_search(),
                    IdeAction::Replace => engine.begin_ex("%s/"),
                    IdeAction::Hover => effects.push(Effect::Hover),
                }
                ui.input_mut(|input| input.consume_key(modifiers, key));
            }
            // IDE navigation and line edits (`IntelliJ` keymap): Shift
            // selects, word/line/document motions, duplicate, move,
            // comment. Vim keeps its own plain and word keys.
            if engine.mode() != Mode::Command {
                #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "Viewport height in rows.")]
                let page_rows = (ui.clip_rect().height() / row_height).floor().max(1.0) as usize;
                let mut edits: Vec<(egui::Key, egui::Modifiers, IdeEdit)> = Vec::new();
                ui.input(|input| {
                    for event in &input.events {
                        if let egui::Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } = event
                            && !(options.popup_keys && popup_key(*key))
                            && let Some(edit) =
                                ide_edit_key(*key, *modifiers, cfg!(target_os = "macos"), page_rows)
                            && (!options.vim || ide_edit_in_vim(edit, *modifiers))
                        {
                            edits.push((*key, *modifiers, edit));
                        }
                    }
                });
                for (key, modifiers, edit) in edits {
                    if matches!(edit, IdeEdit::Fold { .. } | IdeEdit::Unfold { .. }) {
                        fold_actions.push(edit);
                        ui.input_mut(|input| input.consume_key(modifiers, key));
                        continue;
                    }
                    // Every caret runs the edit (one undo step for all).
                    engine.for_each_caret(doc, |engine, doc| {
                    match edit {
                        IdeEdit::DeleteWordBack => engine.ide_delete_word_back(doc),
                        IdeEdit::DeleteLines => engine.ide_delete_lines(doc),
                        IdeEdit::Duplicate => engine.ide_duplicate(doc),
                        IdeEdit::MoveLines { up } => engine.ide_move_lines(doc, up),
                        IdeEdit::ToggleComment => {
                            if let Some(token) = comment_token(language) {
                                engine.ide_toggle_comment(doc, token);
                            }
                        }
                        IdeEdit::JoinLines => engine.ide_join_lines(doc),
                        IdeEdit::Fold { .. } | IdeEdit::Unfold { .. } => {}
                        IdeEdit::Move(
                            motion @ (crate::vim::IdeMotion::Up
                            | crate::vim::IdeMotion::Down
                            | crate::vim::IdeMotion::PageUp(_)
                            | crate::vim::IdeMotion::PageDown(_)),
                            select,
                        ) => {
                            vertical_move = Some(matches!(
                                motion,
                                crate::vim::IdeMotion::Down | crate::vim::IdeMotion::PageDown(_)
                            ));
                            engine.ide_move(doc, motion, select);
                        }
                        IdeEdit::Move(motion, select) => engine.ide_move(doc, motion, select),
                        IdeEdit::ExtendSelection => engine.ide_extend_selection(doc),
                        IdeEdit::ShrinkSelection => engine.ide_shrink_selection(doc),
                    }
                    Vec::new()
                    });
                    ui.input_mut(|input| input.consume_key(modifiers, key));
                }
            }
            // Some backends send both the key and a clipboard event.
            // A block-selection chord must not also insert clipboard text.
            let selecting_block = vim_ctrls.iter().any(|(_, _, key)| *key == VKey::CtrlV);
            for (key, modifiers, vkey) in vim_ctrls {
                effects.extend(engine.press_key(doc, vkey));
                ui.input_mut(|input| input.consume_key(modifiers, key));
            }
            if !selecting_block {
                for text in pastes {
                    // With one clipboard line per caret, each caret gets
                    // its own line (`IntelliJ`); otherwise all get it all.
                    let carets = engine.extra_carets().len().saturating_add(1);
                    let lines: Vec<&str> = text.lines().collect();
                    let per_caret = carets > 1 && lines.len() == carets;
                    let mut order = engine.caret_order(doc).into_iter();
                    engine.for_each_caret(doc, |engine, doc| {
                        // Modeless paste replaces the selection, IDE-style.
                        if !options.vim {
                            engine.ide_delete_selection(doc);
                        }
                        let piece = if per_caret {
                            order
                                .next()
                                .and_then(|rank| lines.get(rank))
                                .copied()
                                .unwrap_or_default()
                        } else {
                            text.as_str()
                        };
                        engine.paste_text(doc, piece);
                        Vec::new()
                    });
                }
            }
            // Paste is editor-scoped while focused; nothing downstream
            // should see it a second time.
            ui.input_mut(|input| {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Paste(_)));
            });
            // Discrete keys (edge-triggered pressed events).
            let mut discrete: Vec<(egui::Key, VKey)> = Vec::new();
            let mut consumed: Vec<egui::Key> = Vec::new();
            ui.input(|input| {
                for event in &input.events {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } = event
                    {
                        let server = crate::lsp::server_language(language).is_some();
                        if options.popup_keys && popup_key(*key) {
                            // The host popup reads this key.
                        } else if *key == egui::Key::Enter
                            && modifiers.alt
                            && !modifiers.command
                            && server
                        {
                            // Alt+Enter: show intention actions (`IntelliJ`).
                            effects.push(Effect::CodeActions);
                            consumed.push(*key);
                        } else if *key == egui::Key::L
                            && modifiers.alt
                            && modifiers.command
                            && server
                            && !options.read_only
                        {
                            // Cmd/Ctrl+Alt+L: reformat code.
                            effects.push(Effect::Format);
                            consumed.push(*key);
                        } else if *key == egui::Key::Space
                            && modifiers.ctrl
                            && !modifiers.alt
                            && crate::lsp::server_language(language).is_some()
                        {
                            // Ctrl+Space: basic completion (`IntelliJ`).
                            effects.push(Effect::Complete);
                            consumed.push(*key);
                        }
                        // F12 jumps to the definition under the cursor in
                        // every mode (VS convention; F-keys are never
                        // vim input and `egui_key` leaves them unmapped).
                        else if *key == egui::Key::F12
                            && !modifiers.command
                            && !modifiers.ctrl
                            && !modifiers.alt
                        {
                            effects.push(Effect::GotoDefinition);
                            consumed.push(*key);
                        } else if *key == egui::Key::C
                            && modifiers.alt
                            && modifiers.shift
                            && modifiers.command
                            && let Some(reference) = &options.reference
                        {
                            // Cmd/Ctrl+Alt+Shift+C: Copy Reference.
                            ui.ctx().copy_text(caret_reference(reference, engine));
                            consumed.push(*key);
                        } else if *key == egui::Key::H
                            && modifiers.ctrl
                            && modifiers.alt
                            && !modifiers.shift
                            && !modifiers.mac_cmd
                            && crate::lsp::server_language(language).is_some()
                        {
                            // Ctrl+Alt+H: Call Hierarchy (`IntelliJ`).
                            effects.push(Effect::Hierarchy { types: false });
                            consumed.push(*key);
                        } else if *key == egui::Key::F7
                            && modifiers.alt
                            && !modifiers.command
                            && crate::lsp::server_language(language).is_some()
                        {
                            // Alt+F7: Find Usages (`IntelliJ`).
                            effects.push(Effect::FindReferences);
                            consumed.push(*key);
                        } else if *key == egui::Key::F8 && !modifiers.command && !modifiers.alt {
                            // F8 / Shift+F8: next / previous problem.
                            let here = (engine.cursor().line, engine.cursor().col);
                            let target = if modifiers.shift {
                                options
                                    .squiggles
                                    .iter()
                                    .rev()
                                    .find(|s| (s.line, s.start) < here)
                                    .or_else(|| options.squiggles.last())
                            } else {
                                options
                                    .squiggles
                                    .iter()
                                    .find(|s| (s.line, s.start) > here)
                                    .or_else(|| options.squiggles.first())
                            };
                            if let Some(target) = target {
                                effects.extend(engine.place_cursor(doc, target.line, target.start));
                            }
                            consumed.push(*key);
                        } else if *key == egui::Key::F2
                            && crate::lsp::server_language(language).is_some()
                            && !modifiers.command
                            && !modifiers.ctrl
                            && !modifiers.alt
                        {
                            effects.push(Effect::RenameSymbol);
                            consumed.push(*key);
                        } else if let Some(mapped) = egui_key(*key, *modifiers) {
                            discrete.push((*key, mapped));
                        }
                    }
                }
            });
            // Printable chars handled below as discrete vim keys; their
            // Text echo must not type a second time (the `i` in `i`).
            let mut handled_chars: Vec<char> = Vec::new();
            for (key, vkey) in discrete {
                // Single printable chars in Insert mode arrive as Text
                // events below; the key event only carries control keys.
                if engine.mode() == Mode::Insert && matches!(vkey, VKey::Char(_)) {
                    continue;
                }
                if let VKey::Char(c) = vkey {
                    handled_chars.push(c);
                }
                // Selection indentation also applies with Vim disabled;
                // keep the selected lines instead of opening Insert mode.
                if engine.mode().is_visual() && matches!(vkey, VKey::Tab | VKey::BackTab) {
                    effects.extend(
                        engine.for_each_caret(doc, |engine, doc| engine.press_key(doc, vkey)),
                    );
                    consumed.push(key);
                    continue;
                }
                // Modeless: with vim off, no key is ever a vim command.
                // Printable keys type (replacing a selection), editing
                // keys edit, and arrows move the IDE caret.
                if !options.vim && engine.mode() != Mode::Command {
                    effects.extend(
                        engine.for_each_caret(doc, |engine, doc| engine.modeless_key(doc, vkey)),
                    );
                    consumed.push(key);
                    continue;
                }
                effects.extend(engine.press_key(doc, vkey));
                consumed.push(key);
            }
            // Handled keys must not leak to egui focus/menu handling:
            // arrows stay in the file, Escape keeps editor focus.
            ui.input_mut(|input| {
                for key in consumed {
                    input.consume_key(egui::Modifiers::NONE, key);
                }
            });
            if engine.mode() == Mode::Insert {
                let mut typed = String::new();
                ui.input_mut(|input| {
                    input.events.retain(|event| {
                        if let egui::Event::Text(text) = event {
                            if !echo_of_handled_key(text, &mut handled_chars) {
                                typed.push_str(text);
                            }
                            false
                        } else {
                            true
                        }
                    });
                });
                if !typed.is_empty() {
                    effects.extend(
                        engine.for_each_caret(doc, |engine, doc| engine.type_text(doc, &typed)),
                    );
                }
            } else {
                // Discard text echoes outside Insert mode so stale input
                // (the `:` and `q` of `:q`) can never leak into a later
                // Insert session.
                ui.input_mut(|input| {
                    input
                        .events
                        .retain(|event| !matches!(event, egui::Event::Text(_)));
                });
            }
            // Keep repainting while focused so the caret stays live. The
            // caret does not blink, so a slow tick is enough: a per-frame
            // repaint keeps the whole GUI awake for nothing.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }

        // Yank sync (`clipboard=unnamed`): every unnamed-register write
        // lands on the system clipboard; terminal contexts can wrap it
        // in `clipboard::copy_sequence` instead.
        if let Some(text) = engine.take_clipboard()
            && !text.is_empty()
        {
            ui.ctx().copy_text(text);
        }

        // Search and long jumps ask for the cursor row to be revealed;
        // user scrolling never triggers this, so it cannot fight input.
        let landed = engine.cursor();
        let caret_moved = landed != caret_before;
        if engine.take_scroll_request() {
            let visual_line = visual_rows
                .iter()
                .position(|row| row.owns_cursor(landed.line, landed.col))
                .unwrap_or(landed.line);
            let row_y = origin.y + visual_line as f32 * row_height;
            ui.scroll_to_rect(
                Rect::from_min_size(Pos2::new(origin.x, row_y), Vec2::new(width, row_height)),
                Some(egui::Align::Center),
            );
        } else if caret_moved {
            // Keyboard and drag moves keep the caret in sight with the
            // least scrolling (`IntelliJ`); a visible caret does not move
            // the viewport. Rows are this frame's layout, so an edit that
            // added lines lands on its logical line instead.
            let (visual_line, cells) = visual_rows
                .iter()
                .enumerate()
                .find(|(_, row)| row.owns_cursor(landed.line, landed.col))
                .map_or((landed.line, 0), |(index, row)| {
                    let hints = row_hints(&options.inlay_hints, row);
                    let geometry = RowCells {
                        text: &row.text,
                        hints: &hints,
                    };
                    (
                        index,
                        geometry.caret_x(landed.col.saturating_sub(row.start)),
                    )
                });
            let caret = Rect::from_min_size(
                Pos2::new(
                    origin.x + gutter + cells as f32 * advance,
                    origin.y + visual_line as f32 * row_height,
                ),
                Vec2::new(advance, row_height),
            );
            ui.scroll_to_rect(caret.expand2(Vec2::new(advance * 2.0, 0.0)), None);
        }
    });
    // Fold changes, now that the row layout is free; a caret left inside a
    // folded region opens it (`IntelliJ` unfolds where the caret goes).
    let caret_line = engine.cursor().line;
    let folding = !fold_toggles.is_empty() || !fold_actions.is_empty();
    let folds = highlight.folds_mut();
    for line in fold_toggles {
        folds.toggle(line);
    }
    for action in fold_actions {
        match action {
            IdeEdit::Fold { all: true } => folds.fold_all(),
            IdeEdit::Fold { all: false } => folds.fold_at(caret_line),
            IdeEdit::Unfold { all: true } => folds.unfold_all(),
            _ => folds.unfold_at(caret_line),
        }
    }
    if let Some(start) = folds.hiding(caret_line) {
        let col = engine.cursor().col;
        if folding || vertical_move == Some(false) {
            // Folding the caret's block, or moving up into one, parks the
            // caret on its first line.
            let col = col.min(doc.line_text(start).chars().count());
            effects.extend(engine.place_cursor(doc, start, col));
        } else if let (Some(true), Some((_, last))) = (vertical_move, folds.region_at(start)) {
            // Moving down hops past the folded block.
            let line = last
                .saturating_add(1)
                .min(doc.line_count().saturating_sub(1));
            let col = col.min(doc.line_text(line).chars().count());
            effects.extend(engine.place_cursor(doc, line, col));
        } else {
            folds.unfold_at(caret_line);
        }
    }
    // Error stripe: every problem as a tick on the right edge, placed by
    // its line's share of the file (`IntelliJ`'s scrollbar marks).
    if !options.squiggles.is_empty() {
        let rail = scroll_output.inner_rect;
        let lines = doc.line_count().max(1);
        let painter = ui.painter().with_clip_rect(rail);
        let mut last_y = f32::NEG_INFINITY;
        for squiggle in &options.squiggles {
            let y = rail.top() + rail.height() * squiggle.line as f32 / lines as f32;
            if y - last_y < 2.0 {
                continue;
            }
            last_y = y;
            let color = match squiggle.severity {
                crate::lsp::Severity::Error => ui.visuals().error_fg_color,
                crate::lsp::Severity::Warning => ui.visuals().warn_fg_color,
                _ => continue,
            };
            painter.rect_filled(
                Rect::from_min_size(Pos2::new(rail.right() - 4.0, y), Vec2::new(4.0, 2.0)),
                0.0,
                color,
            );
        }
    }
    let scroll_range = (scroll_output.content_size.y - scroll_output.inner_rect.height()).max(0.0);
    let scroll_fraction = if scroll_range > 0.0 {
        (scroll_output.state.unclamped_offset().y / scroll_range).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Reserve the footer on every frame so entering or leaving command
    // mode does not move the source viewport. The command prompt owns the
    // left half; the cursor position always sits at the right edge, so
    // code editing gets the same Ln/Col readout as the Markdown preview.
    let footer_rect = if options.show_status {
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), footer_height),
            Sense::hover(),
        );
        if let Some(prompt) = command_prompt(engine) {
            ui.painter().text(
                Pos2::new(rect.left(), rect.center().y),
                egui::Align2::LEFT_CENTER,
                prompt,
                font.clone(),
                ui.visuals().text_color(),
            );
        } else {
            let cursor = engine.cursor();
            ui.painter().text(
                Pos2::new(rect.right(), rect.center().y),
                egui::Align2::RIGHT_CENTER,
                format!(
                    "Ln {}, Col {}",
                    cursor.line.saturating_add(1),
                    cursor.col.saturating_add(1)
                ),
                font.clone(),
                ui.visuals().weak_text_color(),
            );
        }
        Some(rect)
    } else {
        None
    };
    SourceOutcome {
        effects,
        content_rect,
        activated,
        context_menu_opened,
        rename_action_rect,
        hover_action_rect,
        footer_rect,
        scroll_fraction,
        visual_rows: row_count,
        caret_rect,
        change_clicked,
        annotation_clicked,
        toggle_blame,
    }
}

fn line_col_ord(c: Cursor) -> (usize, usize) {
    (c.line, c.col)
}

/// Paint one row: plain in a single shape, or one shape per highlight
/// span with a plain tail for anything the spans do not cover. Piece
/// positions use display cells, so wide chars keep every color aligned.
struct RowPaint<'a> {
    painter: &'a egui::Painter,
    text: &'a str,
    /// Inlay hints: painted as weak boxed text, pushing the chars after
    /// them right (see [`RowCells`]).
    hints: &'a [RowHint<'a>],
    spans: &'a [Span],
    x: f32,
    y: f32,
    advance: f32,
    font: &'a FontId,
    italic_font: &'a FontId,
    bold_font: &'a FontId,
    plain: Color32,
}

impl RowPaint<'_> {
    /// Font for one span: theme emphasis picks the italic/bold family.
    fn font_for(&self, style: SpanStyle) -> FontId {
        if style.italic {
            self.italic_font.clone()
        } else if style.bold {
            self.bold_font.clone()
        } else {
            self.font.clone()
        }
    }

    /// Paint `piece` (row text from char `col`) in one style, split at
    /// every hint inside it so the hint gaps open between chars.
    fn piece(&self, input: PieceInput<'_>) {
        let PieceInput {
            col,
            piece,
            font,
            color,
        } = input;
        let geometry = RowCells {
            text: self.text,
            hints: self.hints,
        };
        let len = piece.chars().count();
        let end = col.saturating_add(len);
        let mut breaks: Vec<usize> = self
            .hints
            .iter()
            .map(|hint| hint.col)
            .filter(|at| *at > col && *at < end)
            .collect();
        breaks.dedup();
        breaks.push(end);
        let mut chars = piece.chars();
        let mut from = col;
        for to in breaks {
            let part: String = chars.by_ref().take(to.saturating_sub(from)).collect();
            if !part.is_empty() {
                let x = self.x + geometry.char_x(from) as f32 * self.advance;
                self.painter.text(
                    Pos2::new(x, self.y),
                    egui::Align2::LEFT_TOP,
                    expand_tabs(&part, display_x(self.text, from)),
                    font.clone(),
                    color,
                );
            }
            from = to;
        }
    }

    fn paint(&self) {
        self.paint_hints();
        if self.spans.is_empty() {
            self.piece(PieceInput {
                col: 0,
                piece: self.text,
                font: self.font,
                color: self.plain,
            });
            return;
        }
        let mut chars = self.text.chars();
        let mut col = 0usize;
        for span in self.spans {
            // Unstyled gaps between spans paint plain: spans need not
            // partition the row (tree-sitter emits styled ranges only).
            let gap: String = chars
                .by_ref()
                .take(span.start.saturating_sub(col))
                .collect();
            if !gap.is_empty() {
                self.piece(PieceInput {
                    col,
                    piece: &gap,
                    font: self.font,
                    color: self.plain,
                });
                col = span.start;
            }
            let piece: String = chars.by_ref().take(span.end.saturating_sub(col)).collect();
            if !piece.is_empty() {
                let [red, green, blue] = span.color;
                self.piece(PieceInput {
                    col,
                    piece: &piece,
                    font: &self.font_for(span.style),
                    color: Color32::from_rgb(red, green, blue),
                });
            }
            col = span.end;
        }
        let tail: String = chars.collect();
        if !tail.is_empty() {
            self.piece(PieceInput {
                col,
                piece: &tail,
                font: self.font,
                color: self.plain,
            });
        }
    }

    /// Hint labels in weak text on a faint box, in the gap before their
    /// char.
    fn paint_hints(&self) {
        let geometry = RowCells {
            text: self.text,
            hints: self.hints,
        };
        let mut seen_at: Option<(usize, usize)> = None;
        for hint in self.hints {
            // Several hints at one column sit side by side.
            let offset = match seen_at {
                Some((at, cells)) if at == hint.col => cells,
                _ => 0,
            };
            seen_at = Some((hint.col, offset.saturating_add(hint.cells)));
            let cell = geometry.caret_x(hint.col).saturating_add(offset);
            let rect = Rect::from_min_size(
                Pos2::new(self.x + cell as f32 * self.advance, self.y + 1.0),
                Vec2::new(hint.cells as f32 * self.advance, self.font.size + 2.0),
            );
            let label = hint.label.trim();
            let lead = hint
                .label
                .len()
                .saturating_sub(hint.label.trim_start().len());
            let box_rect = Rect::from_min_size(
                Pos2::new(rect.left() + lead as f32 * self.advance, rect.top()),
                Vec2::new(
                    label
                        .chars()
                        .map(crate::rows::char_display_width)
                        .sum::<usize>() as f32
                        * self.advance,
                    rect.height(),
                ),
            );
            self.painter
                .rect_filled(box_rect, 3.0, self.plain.gamma_multiply(0.08));
            self.painter.text(
                box_rect.left_top(),
                egui::Align2::LEFT_TOP,
                label,
                self.font.clone(),
                self.plain.gamma_multiply(0.5),
            );
        }
    }
}

/// `reference:line` for the caret (1-based line), or `reference:first-last`
/// for a selection spanning several lines.
pub(super) fn caret_reference(reference: &str, engine: &VimEngine) -> String {
    let caret = engine.cursor().line.saturating_add(1);
    match engine.selection() {
        Some((start, end)) if start.line != end.line => format!(
            "{reference}:{}-{}",
            start.line.saturating_add(1),
            end.line.saturating_add(1)
        ),
        _ => format!("{reference}:{caret}"),
    }
}

/// Char span of the selection on one visual line, end-exclusive.
fn selection_span_for_line(input: SelectionSpanForLineInput) -> Option<(usize, usize)> {
    let SelectionSpanForLineInput {
        a,
        b,
        line,
        line_len,
        inclusive,
    } = input;
    if line < a.line || line > b.line {
        return None;
    }
    let start = if line == a.line { a.col } else { 0 };
    let end = if line == b.line && !inclusive {
        b.col.min(line_len)
    } else if line == b.line {
        b.col
            .checked_add(1)
            .map_or(line_len, |end| end.min(line_len))
    } else {
        line_len
    };
    if start >= end {
        return None;
    }
    Some((start, end))
}

/// A wavy underline along the bottom of `rect`.
fn paint_squiggle(painter: &egui::Painter, rect: Rect, color: Color32) {
    const STEP: f32 = 2.0;
    let base = rect.bottom() - 2.0;
    let mut points = Vec::new();
    let mut x = rect.left();
    let mut up = false;
    while x <= rect.right() {
        points.push(Pos2::new(x, if up { base - 1.5 } else { base }));
        up = !up;
        x += STEP;
    }
    if points.len() > 1 {
        painter.add(egui::Shape::line(points, egui::Stroke::new(1.0, color)));
    }
}

/// Whole-word occurrences of `word` in `text`, as (char start, char len).
fn word_occurrences(text: &str, word: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let needle: Vec<char> = word.chars().collect();
    let is_word = |c: Option<&char>| c.is_some_and(|c| c.is_alphanumeric() || *c == '_');
    let mut out = Vec::new();
    if needle.is_empty() || needle.len() > chars.len() {
        return out;
    }
    let mut i = 0usize;
    while i.saturating_add(needle.len()) <= chars.len() {
        let end = i.saturating_add(needle.len());
        if chars.get(i..end) == Some(needle.as_slice())
            && !is_word(i.checked_sub(1).and_then(|p| chars.get(p)))
            && !is_word(chars.get(end))
        {
            out.push((i, needle.len()));
            i = end;
        } else {
            i = i.saturating_add(1);
        }
    }
    out
}

/// `-- NORMAL --` style status plus 1-based cursor position.
pub fn status_line(engine: &impl ModalEngine) -> String {
    let cursor = engine.cursor();
    format!(
        "-- {} -- {}:{}",
        mode_name(engine.mode()),
        cursor.line.saturating_add(1),
        cursor.col.saturating_add(1)
    )
}

#[cfg(test)]
mod wrap_tests {
    use super::{ShowSourceInput, SourceOptions, show_source};
    use crate::doc::{Buffer, Doc};
    use crate::highlight::{HighlightCache, Language};
    use crate::vim::VimEngine;

    #[test]
    fn markdown_source_uses_wrapped_viewport_height() {
        let ctx = egui::Context::default();
        let mut doc = Doc::new(format!("{}\n", "long source words ".repeat(25)));
        let mut engine = VimEngine::new();
        let mut highlight = HighlightCache::default();
        let mut visual_count = 0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(200.0, 220.0),
                )),
                ..Default::default()
            },
            |ui| {
                ui.set_width(200.0);
                let outcome = show_source(
                    ui,
                    &mut doc,
                    &mut engine,
                    ShowSourceInput {
                        options: &SourceOptions {
                            soft_wrap: true,
                            ..Default::default()
                        },
                        highlight: &mut highlight,
                        language: Language::Markdown,
                        id: "wrapped.md",
                    },
                );
                visual_count = outcome.visual_rows;
            },
        );
        output.textures_delta.clear();
        assert!(visual_count > doc.line_count());
    }
}
