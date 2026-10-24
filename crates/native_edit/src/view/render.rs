use super::super::doc::Buffer;
use super::super::highlight::{HighlightCache, Language, Span, SpanStyle};
use super::super::vim::{Cursor, Effect, Key as VKey, ModalEngine, Mode, VimEngine};
use super::keys::{
    IdeAction, command_prompt, ctrl_h_is_motion, ctrl_v_is_block, echo_of_handled_key, egui_key,
    ide_shortcut, mode_name, source_focus_id,
};
use super::{BOLD_FAMILY, ITALIC_FAMILY};
use egui;
use egui::{Color32, FontId, Pos2, Rect, Sense, Vec2};
/// Worst severity on one buffer row, for the diagnostics gutter.
pub struct DiagnosticMark {
    /// Zero-based buffer row.
    pub line: usize,
    pub severity: crate::lsp::Severity,
}

pub type MenuItemRenderer = fn(&mut egui::Ui, &str, &str, &str) -> egui::Response;

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
}

struct VisualRow {
    line: usize,
    start: usize,
    end: usize,
    line_len: usize,
    text: String,
}

impl VisualRow {
    fn owns_cursor(&self, line: usize, col: usize) -> bool {
        self.line == line
            && col >= self.start
            && (col < self.end || (col == self.end && self.end == self.line_len))
    }
}

fn visual_rows<B: Buffer>(doc: &B, wrap_columns: Option<usize>) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    for line in 0..doc.line_count() {
        let text = doc.line_text(line);
        let chars: Vec<char> = text.chars().collect();
        let line_len = chars.len();
        if line_len == 0 || wrap_columns.is_none() {
            rows.push(VisualRow {
                line,
                start: 0,
                end: line_len,
                line_len,
                text,
            });
            continue;
        }
        let limit = wrap_columns.unwrap_or(usize::MAX).max(1);
        let mut start = 0;
        while start < line_len {
            let mut end = start;
            let mut cells = 0usize;
            let mut last_space = None;
            while end < line_len {
                let Some(ch) = chars.get(end).copied() else {
                    break;
                };
                let width = char_display_width(ch);
                if end > start && cells.saturating_add(width) > limit {
                    break;
                }
                cells = cells.saturating_add(width);
                end = end.saturating_add(1);
                if ch.is_whitespace() {
                    last_space = Some(end);
                }
            }
            if end < line_len
                && let Some(space) = last_space.filter(|space| *space > start)
            {
                end = space;
            }
            rows.push(VisualRow {
                line,
                start,
                end,
                line_len,
                text: chars.get(start..end).unwrap_or(&[]).iter().collect(),
            });
            start = end;
        }
    }
    rows
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
    let gutter = if options.show_line_numbers {
        #[allow(clippy::cast_precision_loss)] // UI coordinate
        let digits = doc.line_count().max(1).to_string().len() as f32;
        (digits + 2.0) * advance
    } else {
        0.0
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // Pane geometry becomes a bounded number of monospace cells.
    let wrap_columns = options.soft_wrap.then(|| {
        ((ui.available_width() - gutter - 16.0).max(advance) / advance)
            .floor()
            .max(1.0) as usize
    });
    let visual_rows = visual_rows(doc, wrap_columns);
    #[allow(clippy::cast_precision_loss)] // UI coordinate
    let content_height = visual_rows.len().max(1) as f32 * row_height;
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

    // Keep one footer row outside the source viewport. The viewport must
    // fill its allotted height even when the file has only a few lines;
    // otherwise the command/search prompt follows the end of the file.
    let footer_height = if options.show_status { row_height } else { 0.0 };
    let max_scroll =
        (ui.available_height() - footer_height - ui.spacing().item_spacing.y).max(row_height);
    let mut content_rect = egui::Rect::NOTHING;
    let mut context_menu_opened = false;
    let mut rename_action_rect = None;
    let mut hover_action_rect = None;
    let mut activated = false;
    let mut scroll = egui::ScrollArea::vertical()
        .id_salt(("native-source", id))
        .max_height(max_scroll)
        .auto_shrink([true, false]);
    if let Some(fraction) = options.scroll_fraction {
        scroll = scroll.vertical_scroll_offset(
            (content_height - max_scroll).max(0.0) * fraction.clamp(0.0, 1.0),
        );
    }
    let scroll_output = scroll.show(ui, |ui| {
        let width = ui.available_width().max(gutter + advance);
        let (response, painter) = ui.allocate_painter(
            Vec2::new(width, content_height.max(max_scroll)),
            Sense::click(),
        );
        // I-beam over file text; the default Sense::click pointer reads
        // as "clickable chrome" instead of an editor.
        let response = response.on_hover_cursor(egui::CursorIcon::Text);
        content_rect = response.rect;
        activated = response.clicked() || response.secondary_clicked();
        let focus_id = source_focus_id(id);
        ui.ctx().data_mut(|data| {
            data.insert_temp(focus_id.with("native-source"), true);
        });
        // Register the stable keyboard target each frame. Otherwise egui
        // drops it as a disappeared widget after the focus request.
        let focus = ui.interact(response.rect, focus_id, Sense::focusable_noninteractive());
        focus.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::TextInput, ui.is_enabled(), "Source editor")
        });
        if response.clicked() || (response.secondary_clicked() && engine.selection().is_none()) {
            ui.memory_mut(|memory| memory.request_focus(focus_id));
            if let Some(pos) = response.interact_pointer_pos() {
                let click_origin = response.rect.min;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                // UI coordinate
                let visual_line = ((pos.y - click_origin.y) / row_height).floor().max(0.0) as usize;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                // UI coordinate
                let cells = ((pos.x - click_origin.x - gutter) / advance)
                    .round()
                    .max(0.0) as usize;
                // Wide chars occupy two cells: resolve the display cell
                // back to a char column on the clicked row.
                if let Some(row) = visual_rows.get(visual_line).or_else(|| visual_rows.last()) {
                    let col = row.start.saturating_add(col_at_display_x(&row.text, cells));
                    effects.extend(engine.place_cursor(doc, row.line, col));
                }
            }
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

        let cursor = engine.cursor();
        let cursor_visible = !matches!(engine.mode(), Mode::Command);
        for (visual_line, visual) in visual_rows.iter().enumerate() {
            let line = visual.line;
            #[allow(clippy::cast_precision_loss)] // UI coordinate
            let y = origin.y + visual_line as f32 * row_height;
            if y + row_height < painter.clip_rect().top() || y > painter.clip_rect().bottom() {
                continue;
            }
            if options.show_line_numbers && visual.start == 0 {
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
            let text = &visual.text;
            // Selection background for this row.
            let span = match block {
                Some((top, bottom, left, right)) if line >= top && line <= bottom => {
                    let len = visual.line_len;
                    (left < right.min(len)).then(|| (left, right.min(len)))
                }
                _ => selection
                    .and_then(|(a, b)| selection_span_for_line(a, b, line, visual.line_len)),
            };
            if let Some((start_col, end_col)) = span
                && let (start_col, end_col) = (start_col.max(visual.start), end_col.min(visual.end))
                && start_col < end_col
            {
                // Char columns to display cells: wide chars shift every
                // cell after them.
                let start_cell = display_x(text, start_col.saturating_sub(visual.start));
                let cells = display_x(text, end_col.saturating_sub(visual.start))
                    .saturating_sub(start_cell);
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
            let cursor_here = cursor_visible && visual.owns_cursor(cursor.line, cursor.col);
            let local_col = cursor.col.saturating_sub(visual.start);
            let cell = cursor_here.then(|| text.chars().nth(local_col)).flatten();
            let cursor_cells = cursor_here.then(|| display_x(text, local_col));
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
            // Some backends send both the key and a clipboard event.
            // A block-selection chord must not also insert clipboard text.
            let selecting_block = vim_ctrls.iter().any(|(_, _, key)| *key == VKey::CtrlV);
            for (key, modifiers, vkey) in vim_ctrls {
                effects.extend(engine.press_key(doc, vkey));
                ui.input_mut(|input| input.consume_key(modifiers, key));
            }
            if !selecting_block {
                for text in pastes {
                    engine.paste_text(doc, &text);
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
                    effects.extend(engine.press_key(doc, vkey));
                    consumed.push(key);
                    continue;
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
            let visual_line = visual_rows
                .iter()
                .position(|row| row.owns_cursor(landed.line, landed.col))
                .unwrap_or(landed.line);
            #[allow(clippy::cast_precision_loss)] // UI coordinate
            let row_y = origin.y + visual_line as f32 * row_height;
            ui.scroll_to_rect(
                Rect::from_min_size(Pos2::new(origin.x, row_y), Vec2::new(width, row_height)),
                Some(egui::Align::Center),
            );
        }
    });
    let scroll_range = (scroll_output.content_size.y - scroll_output.inner_rect.height()).max(0.0);
    let scroll_fraction = if scroll_range > 0.0 {
        (scroll_output.state.unclamped_offset().y / scroll_range).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // Reserve the footer on every frame so entering or leaving command
    // mode does not move the source viewport.
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
                font,
                ui.visuals().text_color(),
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
        visual_rows: visual_rows.len(),
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
                    self.font_for(span.style),
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

#[cfg(test)]
mod wrap_tests {
    use super::{SourceOptions, display_x, show_source, visual_rows};
    use crate::doc::{Buffer, Doc};
    use crate::highlight::{HighlightCache, Language};
    use crate::vim::VimEngine;

    #[test]
    fn wrapped_rows_preserve_source_and_cursor_columns() {
        let doc = Doc::new("# Heading\n- Long words remain in one logical line\n");
        let rows = visual_rows(&doc, Some(10));
        for line in 0..doc.line_count() {
            let joined: String = rows
                .iter()
                .filter(|row| row.line == line)
                .map(|row| row.text.as_str())
                .collect();
            assert_eq!(joined, doc.line_text(line));
        }
        assert!(rows.iter().filter(|row| row.line == 1).count() > 1);
        for col in 0..=doc.line_text(1).chars().count() {
            assert_eq!(
                rows.iter().filter(|row| row.owns_cursor(1, col)).count(),
                1,
                "cursor column {col} must have one visual row"
            );
        }
        assert!(
            rows.iter()
                .all(|row| display_x(&row.text, row.text.chars().count()) <= 10)
        );
    }

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
                    &SourceOptions {
                        soft_wrap: true,
                        ..Default::default()
                    },
                    &mut highlight,
                    Language::Markdown,
                    "wrapped.md",
                );
                visual_count = outcome.visual_rows;
            },
        );
        output.textures_delta.clear();
        assert!(visual_count > doc.line_count());
    }
}
