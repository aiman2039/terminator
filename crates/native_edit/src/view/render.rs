use super::super::doc::Buffer;
use super::super::highlight::{HighlightCache, Language, Span};
use super::super::vim::{Cursor, Effect, Key as VKey, ModalEngine, Mode, VimEngine};
use super::keys::{
    IdeAction, command_prompt, ctrl_h_is_motion, ctrl_v_is_block, echo_of_handled_key, egui_key,
    ide_shortcut, mode_name, source_focus_id,
};
use egui;
use egui::{Color32, FontId, Pos2, Rect, Sense, Vec2};
/// Worst severity on one buffer row, for the diagnostics gutter.
pub struct DiagnosticMark {
    /// Zero-based buffer row.
    pub line: usize,
    pub severity: crate::lsp::Severity,
}

pub struct SourceOptions {
    pub font_size: f32,
    pub show_line_numbers: bool,
    pub show_status: bool,
    /// Gutter marks, sorted by row; binary-searched per painted row, so
    /// an empty vec costs nothing on large files.
    pub diagnostics: Vec<DiagnosticMark>,
    /// Take keyboard focus when nothing else holds it, so an opened file is
    /// immediately typeable (`IntelliJ` focuses the editor on open).
    pub autofocus: bool,
    /// Vim mode: Ctrl-V/Ctrl-H belong to vim in its modes, and printable
    /// keys are vim commands in Normal mode. Off is fully modeless:
    /// printable keys (and Enter/Backspace/…) always edit, and Ctrl-V/H
    /// always paste/prompt.
    pub vim: bool,
}

impl Default for SourceOptions {
    fn default() -> Self {
        Self {
            font_size: 13.0,
            show_line_numbers: true,
            show_status: true,
            autofocus: true,
            vim: true,
            diagnostics: Vec::new(),
        }
    }
}

pub struct SourceOutcome {
    pub effects: Vec<Effect>,
    /// Painted file area, for fixture targeting and reveal requests.
    pub content_rect: egui::Rect,
}

/// Render an editable source view. Returns engine effects (`Save`/`Quit`/…)
/// for the session layer to act on. Highlighting derives from the buffer
/// revision into the caller's [`HighlightCache`]: frames without edits
/// repaint from the cache, edits re-derive once.
pub fn show_source<B: Buffer>(
    ui: &mut egui::Ui,
    doc: &mut B,
    engine: &mut VimEngine,
    options: &SourceOptions,
    highlight: &mut HighlightCache,
    language: Language,
    id: &str,
) -> SourceOutcome {
    let font = FontId::monospace(options.font_size);
    let (advance, row_height) = ui.fonts_mut(|fonts| {
        (
            fonts.glyph_width(&font, 'm').max(1.0),
            fonts.row_height(&font).max(1.0),
        )
    });
    let gutter = if options.show_line_numbers {
        #[allow(clippy::cast_precision_loss)] // UI coordinate
        let digits = doc.line_count().max(1).to_string().len() as f32;
        (digits + 2.0) * advance
    } else {
        0.0
    };
    #[allow(clippy::cast_precision_loss)] // UI coordinate
    let content_height = doc.line_count().max(1) as f32 * row_height;
    let mut effects = Vec::new();

    // Highlight derive, once per buffer revision: unchanged frames repaint
    // straight from the cache without rebuilding the text.
    let dark = ui.visuals().dark_mode;
    if !highlight.is_current(language, dark, doc.revision()) {
        let mut text = String::new();
        for line in 0..doc.line_count() {
            text.push_str(&doc.line_text(line));
            text.push('\n');
        }
        highlight.ensure(language, dark, doc.revision(), &text);
    }

    // Reserve the vim footer: an unconstrained scroll area fills the pane
    // and pushes the status/command line out of sight.
    let max_scroll = (ui.available_height() - row_height - 8.0).max(row_height * 3.0);
    let mut content_rect = egui::Rect::NOTHING;
    egui::ScrollArea::vertical()
        .id_salt(("native-source", id))
        .max_height(max_scroll)
        .show(ui, |ui| {
            let width = ui.available_width().max(gutter + advance);
            let (response, painter) =
                ui.allocate_painter(Vec2::new(width, content_height), Sense::click());
            // I-beam over file text; the default Sense::click pointer reads
            // as "clickable chrome" instead of an editor.
            let response = response.on_hover_cursor(egui::CursorIcon::Text);
            content_rect = response.rect;
            let focus_id = source_focus_id(id);
            if response.clicked() {
                ui.memory_mut(|memory| memory.request_focus(focus_id));
                if let Some(pos) = response.interact_pointer_pos() {
                    let click_origin = response.rect.min;
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    // UI coordinate
                    let line = ((pos.y - click_origin.y) / row_height).floor().max(0.0) as usize;
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    // UI coordinate
                    let cells = ((pos.x - click_origin.x - gutter) / advance)
                        .round()
                        .max(0.0) as usize;
                    // Wide chars occupy two cells: resolve the display cell
                    // back to a char column on the clicked row.
                    let row = doc.line_text(line.min(doc.line_count().saturating_sub(1)));
                    let col = col_at_display_x(&row, cells);
                    effects.extend(engine.place_cursor(doc, line, col));
                }
            }
            if options.autofocus && ui.memory(|memory| memory.focused().is_none()) {
                ui.memory_mut(|memory| memory.request_focus(focus_id));
            }
            let focused = ui.memory(|memory| memory.has_focus(focus_id));
            let origin = response.rect.min;

            // Selection in line/col space, ordered. A visual block
            // paints its rectangle instead: (top, bottom, left, right+1).
            let selection = engine.selection().map(|(a, b)| {
                let (a_idx, b_idx) = (line_col_ord(a), line_col_ord(b));
                if a_idx <= b_idx { (a, b) } else { (b, a) }
            });
            let block = engine.block_span();

            let cursor = engine.cursor();
            let cursor_visible = !matches!(engine.mode(), Mode::Command);
            for line in 0..doc.line_count() {
                #[allow(clippy::cast_precision_loss)] // UI coordinate
                let y = origin.y + line as f32 * row_height;
                if options.show_line_numbers {
                    // A marked row tints its line number: errors shout,
                    // warnings caution, lower severities stay quiet.
                    let number_color = options
                        .diagnostics
                        .binary_search_by(|mark| mark.line.cmp(&line))
                        .ok()
                        .and_then(|at| options.diagnostics.get(at))
                        .map_or_else(
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
                let text = doc.line_text(line);
                // Selection background for this row.
                let span = match block {
                    Some((top, bottom, left, right)) if line >= top && line <= bottom => {
                        let len = text.chars().count();
                        (left < right.min(len)).then(|| (left, right.min(len)))
                    }
                    _ => selection.and_then(|(a, b)| {
                        selection_span_for_line(a, b, line, text.chars().count())
                    }),
                };
                if let Some((start_col, end_col)) = span {
                    // Char columns to display cells: wide chars shift every
                    // cell after them.
                    let start_cell = display_x(&text, start_col);
                    let cells = display_x(&text, end_col).saturating_sub(start_cell);
                    #[allow(clippy::cast_precision_loss)] // UI coordinate
                    let start_x = origin.x + gutter + start_cell as f32 * advance;
                    #[allow(clippy::cast_precision_loss)] // UI coordinate
                    let span_w = cells as f32 * advance;
                    painter.rect_filled(
                        Rect::from_min_size(Pos2::new(start_x, y), Vec2::new(span_w, row_height)),
                        0.0,
                        ui.visuals().selection.bg_fill,
                    );
                }
                // Owned before `text` moves into the paint calls below.
                let cell = (cursor_visible && cursor.line == line)
                    .then(|| text.chars().nth(cursor.col))
                    .flatten();
                let cursor_cells =
                    (cursor_visible && cursor.line == line).then(|| display_x(&text, cursor.col));
                RowPaint {
                    painter: &painter,
                    text: &text,
                    spans: highlight.line(line),
                    x: origin.x + gutter,
                    y,
                    advance,
                    font: &font,
                    plain: ui.visuals().text_color(),
                }
                .paint();
                if let Some(cells) = cursor_cells {
                    let cell_cells = cell.map_or(1, char_display_width);
                    #[allow(clippy::cast_precision_loss)] // UI coordinate
                    let x = origin.x + gutter + cells as f32 * advance;
                    if engine.mode() == Mode::Insert {
                        painter.rect_filled(
                            Rect::from_min_size(Pos2::new(x, y), Vec2::new(2.0, row_height)),
                            0.0,
                            ui.visuals().text_color(),
                        );
                    } else {
                        #[allow(clippy::cast_precision_loss)] // UI coordinate
                        let cursor_w = cell_cells as f32 * advance;
                        painter.rect_filled(
                            Rect::from_min_size(
                                Pos2::new(x, y),
                                Vec2::new(cursor_w.max(2.0), row_height),
                            ),
                            0.0,
                            ui.visuals().selection.bg_fill,
                        );
                        // Keep the covered cell legible on the block cursor.
                        if let Some(cell) = cell {
                            painter.text(
                                Pos2::new(x, y),
                                egui::Align2::LEFT_TOP,
                                cell.to_string(),
                                font.clone(),
                                ui.visuals().selection.stroke.color,
                            );
                        }
                    }
                }
            }

            if focused {
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
                                if let Some(action) = ide_shortcut(*key, *modifiers) {
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
                        IdeAction::PasteKey
                            if *key == egui::Key::V && ctrl_v_is_block(options.vim, mode) =>
                        {
                            vim_ctrls.push((*key, *modifiers, VKey::CtrlV));
                            false
                        }
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
                            let text = engine.copy_text(doc);
                            if !text.is_empty() {
                                ui.ctx().copy_text(text);
                            }
                        }
                        IdeAction::Cut => {
                            let text = engine.cut(doc);
                            if !text.is_empty() {
                                ui.ctx().copy_text(text);
                            }
                        }
                        IdeAction::SelectAll => engine.select_all(doc),
                        IdeAction::Undo => engine.undo(doc),
                        IdeAction::Redo => engine.redo(doc),
                        IdeAction::PasteKey => {}
                        IdeAction::Save => effects.push(Effect::Save),
                        IdeAction::Find => engine.begin_search(),
                        IdeAction::Replace => engine.begin_ex("%s/"),
                        IdeAction::Hover => effects.push(Effect::Hover),
                    }
                    ui.input_mut(|input| input.consume_key(modifiers, key));
                }
                for (key, modifiers, vkey) in vim_ctrls {
                    effects.extend(engine.press_key(doc, vkey));
                    ui.input_mut(|input| input.consume_key(modifiers, key));
                }
                for text in pastes {
                    engine.paste_text(doc, &text);
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
                            // F12 jumps to the definition under the cursor in
                            // every mode (VS convention; F-keys are never
                            // vim input and `egui_key` leaves them unmapped).
                            if *key == egui::Key::F12
                                && !modifiers.command
                                && !modifiers.ctrl
                                && !modifiers.alt
                            {
                                effects.push(Effect::GotoDefinition);
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
                    // Modeless: with vim off, Normal mode never holds keys
                    // for commands. Printable keys open an insert session
                    // and type; editing keys (Enter/Backspace/…) open one
                    // and apply there; typing over a selection replaces it.
                    if !options.vim {
                        let mode = engine.mode();
                        if mode == Mode::Normal || mode.is_visual() {
                            match vkey {
                                VKey::Char(c) if c == ' ' || c.is_ascii_graphic() => {
                                    if mode.is_visual() {
                                        let _ = engine.cut(doc);
                                    }
                                    effects.extend(engine.press_key(doc, VKey::Char('i')));
                                    effects.extend(engine.type_text(doc, &c.to_string()));
                                    consumed.push(key);
                                    continue;
                                }
                                VKey::Enter
                                | VKey::Backspace
                                | VKey::Delete
                                | VKey::Tab
                                | VKey::Home
                                | VKey::End => {
                                    effects.extend(engine.press_key(doc, VKey::Char('i')));
                                    effects.extend(engine.press_key(doc, vkey));
                                    consumed.push(key);
                                    continue;
                                }
                                _ => {}
                            }
                        }
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
                        effects.extend(engine.type_text(doc, &typed));
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
                // Keep repainting while focused so the cursor stays live.
                ui.ctx().request_repaint();
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
            if engine.take_scroll_request() {
                #[allow(clippy::cast_precision_loss)] // UI coordinate
                let row_y = origin.y + landed.line as f32 * row_height;
                ui.scroll_to_rect(
                    Rect::from_min_size(Pos2::new(origin.x, row_y), Vec2::new(width, row_height)),
                    Some(egui::Align::Center),
                );
            }
        });

    // Command footer only: mode and cursor live in the pane header. The
    // scroll reservation above keeps this visible while typing `:ex` or
    // `/search`.
    if options.show_status
        && let Some(prompt) = command_prompt(engine)
    {
        ui.horizontal(|ui| {
            ui.monospace(prompt);
        });
    }
    SourceOutcome {
        effects,
        content_rect,
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
    spans: &'a [Span],
    x: f32,
    y: f32,
    advance: f32,
    font: &'a FontId,
    plain: Color32,
}

impl RowPaint<'_> {
    fn paint(&self) {
        if self.spans.is_empty() {
            self.painter.text(
                Pos2::new(self.x, self.y),
                egui::Align2::LEFT_TOP,
                self.text,
                self.font.clone(),
                self.plain,
            );
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
                #[allow(clippy::cast_precision_loss)] // UI coordinate
                let gap_x = self.x + display_x(self.text, col) as f32 * self.advance;
                self.painter.text(
                    Pos2::new(gap_x, self.y),
                    egui::Align2::LEFT_TOP,
                    gap,
                    self.font.clone(),
                    self.plain,
                );
                col = span.start;
            }
            let piece: String = chars.by_ref().take(span.end.saturating_sub(col)).collect();
            if !piece.is_empty() {
                let [red, green, blue] = span.color;
                #[allow(clippy::cast_precision_loss)] // UI coordinate
                let piece_x = self.x + display_x(self.text, col) as f32 * self.advance;
                self.painter.text(
                    Pos2::new(piece_x, self.y),
                    egui::Align2::LEFT_TOP,
                    piece,
                    self.font.clone(),
                    Color32::from_rgb(red, green, blue),
                );
            }
            col = span.end;
        }
        let tail: String = chars.collect();
        if !tail.is_empty() {
            #[allow(clippy::cast_precision_loss)] // UI coordinate
            let tail_x = self.x + display_x(self.text, col) as f32 * self.advance;
            self.painter.text(
                Pos2::new(tail_x, self.y),
                egui::Align2::LEFT_TOP,
                tail,
                self.font.clone(),
                self.plain,
            );
        }
    }
}

/// Char span of the selection on one visual line, end-exclusive.
fn selection_span_for_line(
    a: Cursor,
    b: Cursor,
    line: usize,
    line_len: usize,
) -> Option<(usize, usize)> {
    if line < a.line || line > b.line {
        return None;
    }
    let start = if line == a.line { a.col } else { 0 };
    let end = if line == b.line {
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

/// Display cells of one char: East Asian wide chars take two columns,
/// everything else (including tabs, kept single-cell like the engine)
/// takes one. Control chars fall back to one cell.
#[must_use]
pub fn char_display_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c)
        .unwrap_or(1)
        .max(1)
}

/// Display-column x (in cells) of char column `col`.
#[must_use]
pub fn display_x(text: &str, col: usize) -> usize {
    text.chars()
        .take(col)
        .map(char_display_width)
        .fold(0usize, |acc, w| acc.saturating_add(w))
}

/// Char column at display cell `x` (floor: a click on the second cell of
/// a wide char lands on that char).
#[must_use]
pub fn col_at_display_x(text: &str, x: usize) -> usize {
    let mut acc = 0usize;
    let mut col = 0usize;
    for c in text.chars() {
        let next = acc.saturating_add(char_display_width(c));
        if next > x {
            break;
        }
        acc = next;
        col = col.saturating_add(1);
    }
    col
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
