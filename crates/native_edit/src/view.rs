//! Embeddable native source view over [`crate::doc::Buffer`].
//!
//! The caller owns the [`Doc`](crate::doc::Doc) and [`VimEngine`](super::vim::VimEngine)
//! (a future session layer will); [`show_source`] renders lines with cursor
//! and selection plus a vim status line, and feeds egui input into the
//! engine. Monospace metrics assume every glyph advances like `m` (CJK text
//! misaligns columns; recorded limit, same class as the terminal widget's
//! wide-cell handling).

use super::doc::Buffer;
use super::vim::{Cursor, Effect, Key as VKey, ModalEngine, Mode, VimEngine};
use egui;
use egui::{Color32, FontId, Pos2, Rect, Sense, Vec2};

/// Map an egui key to the engine key. Returns `None` for keys the caller
/// should leave alone (notably anything with Command/Ctrl held, so global
/// shortcuts keep working).
pub fn egui_key(key: egui::Key, modifiers: egui::Modifiers) -> Option<VKey> {
    if modifiers.command || modifiers.ctrl {
        return None;
    }
    Some(match key {
        egui::Key::ArrowLeft => VKey::Left,
        egui::Key::ArrowRight => VKey::Right,
        egui::Key::ArrowUp => VKey::Up,
        egui::Key::ArrowDown => VKey::Down,
        egui::Key::Home => VKey::Home,
        egui::Key::End => VKey::End,
        egui::Key::Enter => VKey::Enter,
        egui::Key::Backspace => VKey::Backspace,
        egui::Key::Delete => VKey::Delete,
        egui::Key::Escape => VKey::Escape,
        egui::Key::Tab => VKey::Tab,
        // Punctuation arrives as dedicated physical keys; pass the character
        // through so current and future engine commands stay reachable.
        egui::Key::Colon => VKey::Char(':'),
        egui::Key::Semicolon => VKey::Char(';'),
        egui::Key::Slash => VKey::Char('/'),
        egui::Key::Period => VKey::Char('.'),
        egui::Key::Comma => VKey::Char(','),
        egui::Key::Minus => VKey::Char('-'),
        egui::Key::Equals => VKey::Char('='),
        egui::Key::Space => VKey::Char(' '),
        egui::Key::Quote => VKey::Char('\''),
        egui::Key::OpenBracket => VKey::Char('['),
        egui::Key::CloseBracket => VKey::Char(']'),
        egui::Key::Backslash => VKey::Char('\\'),
        egui::Key::Backtick => VKey::Char('`'),
        _ => return key_name_char(key, modifiers),
    })
}

fn key_name_char(key: egui::Key, modifiers: egui::Modifiers) -> Option<VKey> {
    let name = key.name();
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(VKey::Char(if modifiers.shift {
            c.to_ascii_uppercase()
        } else {
            c.to_ascii_lowercase()
        })),
        // Shifted digits are their US symbols (`1`→`!`, …, `0`→`)`); a bare
        // digit is a count. Positional like the rest of this table.
        (Some(c), None) if c.is_ascii_digit() => Some(VKey::Char(if modifiers.shift {
            const SYMBOLS: &[u8; 10] = b")!@#$%^&*(";
            SYMBOLS[(c as u8 - b'0') as usize] as char
        } else {
            c
        })),
        _ => {
            // Backends that report the symbol itself (`!` instead of shifted
            // `1`) expose it via `symbol_or_name`; pass printable punctuation
            // through so `:q!` stays typeable everywhere.
            let symbol = key.symbol_or_name();
            let mut chars = symbol.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_punctuation() => Some(VKey::Char(c)),
                _ => None,
            }
        }
    }
}

/// IntelliJ-style shortcut the file view owns in every mode (vim included):
/// the editor keeps copy/cut/paste/select/undo keys instead of yielding them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdeAction {
    Copy,
    Cut,
    SelectAll,
    Undo,
    Redo,
    /// Consume only; the pasted text arrives as a separate Paste event.
    PasteKey,
}

/// Resolve a Command/Ctrl-held key to an IDE action. Plain keys and the Vim
/// engine's own keys return `None` and keep their existing routing.
/// (`command` mirrors Ctrl on Linux/Windows, so only Alt is excluded.)
pub fn ide_shortcut(key: egui::Key, modifiers: egui::Modifiers) -> Option<IdeAction> {
    if !(modifiers.command && !modifiers.alt) {
        return None;
    }
    Some(match key {
        egui::Key::C => IdeAction::Copy,
        egui::Key::X => IdeAction::Cut,
        egui::Key::A => IdeAction::SelectAll,
        egui::Key::Z if modifiers.shift => IdeAction::Redo,
        egui::Key::Z => IdeAction::Undo,
        egui::Key::V => IdeAction::PasteKey,
        _ => return None,
    })
}

/// Stable keyboard-focus identity for one source view. The painter response
/// id is layout-derived, so panes use this to return focus to the file after
/// header-button clicks instead of stranding keys on Save/Reload.
pub fn source_focus_id(id: &str) -> egui::Id {
    egui::Id::new(("native-source-focus", id))
}

/// True when a Text event only echoes a keypress already handled as a
/// discrete vim key — e.g. the `i` that opened Insert mode must not type
/// itself. Each single-char echo consumes one recorded key, in order.
pub fn echo_of_handled_key(text: &str, handled: &mut Vec<char>) -> bool {
    if text.chars().count() != 1 {
        return false;
    }
    if let Some(pos) = handled.iter().position(|c| text.starts_with(*c)) {
        handled.remove(pos);
        return true;
    }
    false
}

/// Vim footer prompt: `:ex` commands and `/search` share the command line.
pub fn command_prompt(engine: &impl ModalEngine) -> Option<String> {
    if engine.mode() != Mode::Command {
        return None;
    }
    let line = engine.command_line();
    if line.starts_with('/') {
        Some(line.to_string())
    } else {
        Some(format!(":{line}"))
    }
}

/// Header badge text for the current vim mode.
pub fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "NORMAL",
        Mode::Insert => "INSERT",
        Mode::Visual => "VISUAL",
        Mode::VisualLine => "V-LINE",
        Mode::Command => "COMMAND",
    }
}

/// Header badge color for the current vim mode (readable on dark/light).
pub fn mode_color(mode: Mode) -> Color32 {
    match mode {
        Mode::Normal => Color32::LIGHT_BLUE,
        Mode::Insert => Color32::LIGHT_GREEN,
        Mode::Visual | Mode::VisualLine => Color32::GOLD,
        Mode::Command => Color32::ORANGE,
    }
}

pub struct SourceOptions {
    pub font_size: f32,
    pub show_line_numbers: bool,
    pub show_status: bool,
    /// Take keyboard focus when nothing else holds it, so an opened file is
    /// immediately typeable (IntelliJ focuses the editor on open).
    pub autofocus: bool,
}

impl Default for SourceOptions {
    fn default() -> Self {
        Self {
            font_size: 13.0,
            show_line_numbers: true,
            show_status: true,
            autofocus: true,
        }
    }
}

pub struct SourceOutcome {
    pub effects: Vec<Effect>,
    /// Painted file area, for fixture targeting and reveal requests.
    pub content_rect: egui::Rect,
}

/// Render an editable source view. Returns engine effects (`Save`/`Quit`/…)
/// for the session layer to act on.
pub fn show_source<B: Buffer>(
    ui: &mut egui::Ui,
    doc: &mut B,
    engine: &mut VimEngine,
    options: &SourceOptions,
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
        (doc.line_count().max(1).to_string().len() as f32 + 2.0) * advance
    } else {
        0.0
    };
    let content_height = doc.line_count().max(1) as f32 * row_height;
    let mut effects = Vec::new();

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
                    let line = ((pos.y - click_origin.y) / row_height).floor().max(0.0) as usize;
                    let col = ((pos.x - click_origin.x - gutter) / advance)
                        .round()
                        .max(0.0) as usize;
                    effects.extend(engine.place_cursor(doc, line, col));
                }
            }
            if options.autofocus && ui.memory(|memory| memory.focused().is_none()) {
                ui.memory_mut(|memory| memory.request_focus(focus_id));
            }
            let focused = ui.memory(|memory| memory.has_focus(focus_id));
            let origin = response.rect.min;

            // Selection in line/col space, ordered.
            let selection = engine.selection().map(|(a, b)| {
                let (a_idx, b_idx) = (line_col_ord(a), line_col_ord(b));
                if a_idx <= b_idx { (a, b) } else { (b, a) }
            });

            let cursor = engine.cursor();
            let cursor_visible = !matches!(engine.mode(), Mode::Command);
            for line in 0..doc.line_count() {
                let y = origin.y + line as f32 * row_height;
                if options.show_line_numbers {
                    painter.text(
                        Pos2::new(origin.x + gutter - advance, y),
                        egui::Align2::RIGHT_TOP,
                        format!("{}", line + 1),
                        font.clone(),
                        ui.visuals().weak_text_color(),
                    );
                }
                let text = doc.line_text(line);
                // Selection background for this row.
                if let Some((a, b)) = selection {
                    let span = selection_span_for_line(a, b, line, text.chars().count());
                    if let Some((start_col, end_col)) = span {
                        painter.rect_filled(
                            Rect::from_min_size(
                                Pos2::new(origin.x + gutter + start_col as f32 * advance, y),
                                Vec2::new((end_col - start_col) as f32 * advance, row_height),
                            ),
                            0.0,
                            ui.visuals().selection.bg_fill,
                        );
                    }
                }
                // Owned before `text` moves into the paint call below.
                let cell = (cursor_visible && cursor.line == line)
                    .then(|| text.chars().nth(cursor.col))
                    .flatten();
                painter.text(
                    Pos2::new(origin.x + gutter, y),
                    egui::Align2::LEFT_TOP,
                    text,
                    font.clone(),
                    ui.visuals().text_color(),
                );
                if cursor_visible && cursor.line == line {
                    let x = origin.x + gutter + cursor.col as f32 * advance;
                    if engine.mode() == Mode::Insert {
                        painter.rect_filled(
                            Rect::from_min_size(Pos2::new(x, y), Vec2::new(2.0, row_height)),
                            0.0,
                            ui.visuals().text_color(),
                        );
                    } else {
                        painter.rect_filled(
                            Rect::from_min_size(
                                Pos2::new(x, y),
                                Vec2::new(advance.max(2.0), row_height),
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
                // every mode (IntelliJ-style), then plain vim keys.
                let mut actions: Vec<(egui::Key, egui::Modifiers, IdeAction)> = Vec::new();
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
                    }
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
                ui.input(|input| {
                    for event in &input.events {
                        if let egui::Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } = event
                            && let Some(mapped) = egui_key(*key, *modifiers)
                        {
                            discrete.push((*key, mapped));
                        }
                    }
                });
                let mut consumed: Vec<egui::Key> = Vec::new();
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

            // Search and long jumps ask for the cursor row to be revealed;
            // user scrolling never triggers this, so it cannot fight input.
            let landed = engine.cursor();
            if engine.take_scroll_request() {
                ui.scroll_to_rect(
                    Rect::from_min_size(
                        Pos2::new(origin.x, origin.y + landed.line as f32 * row_height),
                        Vec2::new(width, row_height),
                    ),
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
        (b.col + 1).min(line_len)
    } else {
        line_len
    };
    if start >= end {
        return None;
    }
    Some((start, end))
}

/// `-- NORMAL --` style status plus 1-based cursor position.
pub fn status_line(engine: &impl ModalEngine) -> String {
    let cursor = engine.cursor();
    format!(
        "-- {} -- {}:{}",
        mode_name(engine.mode()),
        cursor.line + 1,
        cursor.col + 1
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(key: egui::Key) -> Option<VKey> {
        egui_key(key, egui::Modifiers::NONE)
    }

    #[test]
    fn command_key_reaches_colon() {
        // `:` must stay reachable: ex commands live behind it.
        assert_eq!(plain(egui::Key::Colon), Some(VKey::Char(':')));
    }

    #[test]
    fn letters_follow_shift_case() {
        assert_eq!(plain(egui::Key::A), Some(VKey::Char('a')));
        assert_eq!(
            egui_key(egui::Key::A, egui::Modifiers::SHIFT),
            Some(VKey::Char('A'))
        );
    }

    #[test]
    fn shifted_digits_are_symbols_not_counts() {
        // `$` is a motion; it must not arrive as the count digit `4`.
        assert_eq!(
            egui_key(egui::Key::Num4, egui::Modifiers::SHIFT),
            Some(VKey::Char('$'))
        );
        assert_eq!(plain(egui::Key::Num4), Some(VKey::Char('4')));
    }

    #[test]
    fn command_held_keys_stay_with_the_app() {
        assert_eq!(egui_key(egui::Key::F, egui::Modifiers::COMMAND), None);
    }

    #[test]
    fn symbol_keys_pass_through() {
        // Backends may report `!` directly instead of shifted `1`.
        assert_eq!(plain(egui::Key::Exclamationmark), Some(VKey::Char('!')));
    }

    #[test]
    fn arrows_and_escape_map() {
        assert_eq!(plain(egui::Key::ArrowLeft), Some(VKey::Left));
        assert_eq!(plain(egui::Key::Escape), Some(VKey::Escape));
        assert_eq!(plain(egui::Key::F5), None);
    }

    #[test]
    fn ide_shortcuts_need_command_held() {
        assert_eq!(
            ide_shortcut(egui::Key::C, egui::Modifiers::COMMAND),
            Some(IdeAction::Copy)
        );
        assert_eq!(
            ide_shortcut(egui::Key::X, egui::Modifiers::COMMAND),
            Some(IdeAction::Cut)
        );
        assert_eq!(
            ide_shortcut(egui::Key::A, egui::Modifiers::COMMAND),
            Some(IdeAction::SelectAll)
        );
        assert_eq!(
            ide_shortcut(egui::Key::V, egui::Modifiers::COMMAND),
            Some(IdeAction::PasteKey)
        );
        assert_eq!(
            ide_shortcut(egui::Key::Z, egui::Modifiers::COMMAND),
            Some(IdeAction::Undo)
        );
        let shift_command = egui::Modifiers {
            shift: true,
            ..egui::Modifiers::COMMAND
        };
        assert_eq!(
            ide_shortcut(egui::Key::Z, shift_command),
            Some(IdeAction::Redo)
        );
        // Plain keys stay with the vim engine; bare Ctrl (macOS) and Alt
        // chords stay with the terminal/app layer. Ctrl+C on Linux arrives
        // with `command` set, so it still resolves.
        assert_eq!(ide_shortcut(egui::Key::C, egui::Modifiers::NONE), None);
        assert_eq!(
            ide_shortcut(
                egui::Key::C,
                egui::Modifiers {
                    ctrl: true,
                    ..egui::Modifiers::NONE
                }
            ),
            None
        );
        assert_eq!(
            ide_shortcut(
                egui::Key::C,
                egui::Modifiers {
                    ctrl: true,
                    command: true,
                    ..egui::Modifiers::NONE
                }
            ),
            Some(IdeAction::Copy)
        );
        assert_eq!(
            ide_shortcut(
                egui::Key::C,
                egui::Modifiers {
                    command: true,
                    alt: true,
                    ..egui::Modifiers::NONE
                }
            ),
            None
        );
        assert_eq!(ide_shortcut(egui::Key::F, egui::Modifiers::COMMAND), None);
    }

    #[test]
    fn focus_id_is_stable_per_file() {
        assert_eq!(source_focus_id("a"), source_focus_id("a"));
        assert_ne!(source_focus_id("a"), source_focus_id("b"));
    }

    #[test]
    fn handled_key_echo_is_dropped_once() {
        // The `i` that opened Insert mode must not type itself.
        let mut handled = vec!['i'];
        assert!(echo_of_handled_key("i", &mut handled));
        assert!(handled.is_empty());
        assert!(!echo_of_handled_key("i", &mut handled));
        // Multi-char input (IME/paste) is never an echo.
        assert!(!echo_of_handled_key("xy", &mut vec!['x']));
    }

    #[test]
    fn command_prompt_shows_ex_and_search() {
        use crate::doc::Doc;
        let mut doc = Doc::new("hi");
        let mut engine = VimEngine::new();
        assert_eq!(command_prompt(&engine), None);
        engine.press_key(&mut doc, VKey::Char(':'));
        engine.press_key(&mut doc, VKey::Char('w'));
        assert_eq!(command_prompt(&engine), Some(":w".to_string()));
        engine.press_key(&mut doc, VKey::Escape);
        engine.press_key(&mut doc, VKey::Char('/'));
        engine.press_key(&mut doc, VKey::Char('h'));
        assert_eq!(command_prompt(&engine), Some("/h".to_string()));
    }

    /// Drive the real widget headless: one synthetic keypress per frame.
    fn drive_keys(
        ctx: &egui::Context,
        doc: &mut crate::doc::Doc,
        engine: &mut VimEngine,
        keys: &[egui::Key],
    ) -> Vec<Effect> {
        fn key_event(key: egui::Key) -> egui::Event {
            egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }
        }
        let mut effects = Vec::new();
        for key in keys {
            let mut raw = egui::RawInput::default();
            raw.events.push(key_event(*key));
            // A real backend also delivers the printable echo as Text.
            if let Some(c) =
                super::egui_key(*key, egui::Modifiers::NONE).and_then(|mapped| match mapped {
                    VKey::Char(c) => Some(c),
                    _ => None,
                })
            {
                raw.events.push(egui::Event::Text(c.to_string()));
            }
            let mut output = ctx.run_ui(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let outcome = super::show_source(
                        ui,
                        doc,
                        engine,
                        &super::SourceOptions::default(),
                        "headless",
                    );
                    effects = outcome.effects;
                });
            });
            // No renderer headless: acknowledge font texture uploads.
            output.textures_delta.clear();
        }
        effects
    }

    #[test]
    fn colon_q_emits_quit_through_view() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("hi");
        let mut engine = VimEngine::new();
        let effects = drive_keys(
            &ctx,
            &mut doc,
            &mut engine,
            &[egui::Key::Colon, egui::Key::Q, egui::Key::Enter],
        );
        assert!(effects.contains(&Effect::Quit));
        assert_eq!(engine.mode(), Mode::Normal);
    }

    #[test]
    fn insert_opener_echo_does_not_type_through_view() {
        use crate::doc::Doc;
        let ctx = egui::Context::default();
        let mut doc = Doc::new("hi");
        let mut engine = VimEngine::new();
        drive_keys(&ctx, &mut doc, &mut engine, &[egui::Key::I]);
        assert_eq!(engine.mode(), Mode::Insert);
        assert_eq!(doc.text(), "hi");
    }

    #[test]
    fn mode_names_cover_every_mode() {
        assert_eq!(mode_name(Mode::Normal), "NORMAL");
        assert_eq!(mode_name(Mode::Insert), "INSERT");
        assert_eq!(mode_name(Mode::Visual), "VISUAL");
        assert_eq!(mode_name(Mode::VisualLine), "V-LINE");
        assert_eq!(mode_name(Mode::Command), "COMMAND");
    }
}
