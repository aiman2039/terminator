//! Hints and copy mode in the app: start and leave, the keys, the overlay,
//! and opening a picked hint with the hover file menu.
use super::super::super::*;
use super::key_modes::{HintStep, Hints, KeyMode, ModeKey, PendingHint, take_mode_keys};
use egui_term::{CopyCommand, SelectionType, ViMotion};

impl App {
    /// Hints mode on the active terminal, or off when it is on.
    pub(crate) fn toggle_hints_mode(&mut self) {
        let Some(sid) = self.active_session.clone() else {
            return;
        };
        if matches!(self.key_modes.on(&sid), Some(KeyMode::Hints(_))) {
            self.leave_key_mode();
            return;
        }
        self.leave_key_mode();
        let Some(backend) = self.backends.get(&sid) else {
            return;
        };
        let hints = Hints::new(backend.visible_targets());
        if hints.items.is_empty() {
            self.info = Some("No links, paths or hashes on screen".into());
            return;
        }
        self.key_modes.active = Some((sid, KeyMode::Hints(hints)));
    }

    /// Copy mode on the active terminal, or off when it is on.
    pub(crate) fn toggle_copy_mode(&mut self) {
        let Some(sid) = self.active_session.clone() else {
            return;
        };
        if matches!(self.key_modes.on(&sid), Some(KeyMode::Copy { .. })) {
            self.leave_key_mode();
            return;
        }
        self.leave_key_mode();
        let Some(backend) = self.backends.get_mut(&sid) else {
            return;
        };
        backend.set_copy_mode(true);
        self.key_modes.active = Some((sid, KeyMode::Copy { pending_g: false }));
    }

    pub(crate) fn leave_key_mode(&mut self) {
        if let Some((sid, KeyMode::Copy { .. })) = self.key_modes.active.take()
            && let Some(backend) = self.backends.get_mut(&sid)
        {
            backend.set_copy_mode(false);
        }
    }

    /// Take this frame's keys for the mode on `sid`, before the terminal
    /// widget reads input. A mode whose terminal lost focus ends.
    pub(crate) fn key_mode_input(&mut self, ctx: &egui::Context, sid: &str) {
        if self.key_modes.on(sid).is_none() {
            return;
        }
        if self.active_session.as_deref() != Some(sid) || !self.backends.contains_key(sid) {
            self.leave_key_mode();
            return;
        }
        if ctx.text_edit_focused() {
            return;
        }
        let keys = ctx.input_mut(|input| take_mode_keys(&mut input.events));
        for key in keys {
            if self.key_modes.on(sid).is_none() {
                break;
            }
            self.key_mode_key(ctx, sid, key);
        }
    }

    fn key_mode_key(&mut self, ctx: &egui::Context, sid: &str, key: ModeKey) {
        let Some((_, mode)) = self.key_modes.active.as_mut() else {
            return;
        };
        match mode {
            KeyMode::Hints(hints) => match hints.key(key) {
                HintStep::Wait => {}
                HintStep::Leave => self.leave_key_mode(),
                HintStep::Pick { index, copy } => {
                    let item = hints.items.get(index).map(|(_, t)| t.clone());
                    self.leave_key_mode();
                    match item {
                        Some(target) if copy => self.copy_with_note(ctx, target.text, "Text"),
                        Some(target) => self.open_hint(sid, target),
                        None => {}
                    }
                }
            },
            KeyMode::Copy { .. } => self.copy_mode_key(ctx, sid, key),
        }
    }

    /// Resolve a picked hint like a hovered link; [`Self::finish_pending_hint`]
    /// shows the result.
    fn open_hint(&mut self, sid: &str, target: egui_term::LinkTarget) {
        let Some(session) = self.state.sessions.iter().find(|s| s.id == sid) else {
            return;
        };
        let key = format!("target:{}:{}:{}", sid, session.cwd.display(), target.text);
        if !self.targets.contains_key(&key) && self.loading.insert(key.clone()) {
            let _ = self.jobs.send(Job::ResolveTarget(
                key.clone(),
                target.text.clone(),
                session.cwd.clone(),
            ));
        }
        self.key_modes.pending = Some(PendingHint {
            session: sid.to_owned(),
            key,
            token: target.text,
            rect: target.rects.first().copied().unwrap_or(egui::Rect::ZERO),
        });
    }

    /// A picked hint resolved: open the file menu on it, or copy a token
    /// that is no file or URL (a hash, for example).
    pub(crate) fn finish_pending_hint(&mut self, ctx: &egui::Context, sid: &str, pane: egui::Rect) {
        let Some(pending) = self.key_modes.pending.clone().filter(|p| p.session == sid) else {
            return;
        };
        let Some(resolved) = self.targets.get(&pending.key).cloned() else {
            ctx.request_repaint_after(Duration::from_millis(50));
            return;
        };
        self.key_modes.pending = None;
        match resolved {
            Some(target) => {
                self.hover_popup = Some(HoverPopup {
                    session: sid.to_owned(),
                    key: pending.key,
                    target,
                    rect: pending.rect.translate(pane.min.to_vec2()),
                });
            }
            None => self.copy_with_note(ctx, pending.token, "Text"),
        }
    }

    fn copy_mode_key(&mut self, ctx: &egui::Context, sid: &str, key: ModeKey) {
        let gg = match self.key_modes.active.as_mut() {
            Some((_, KeyMode::Copy { pending_g })) => std::mem::take(pending_g),
            _ => false,
        };
        let Some(backend) = self.backends.get_mut(sid) else {
            return;
        };
        let rows = i32::from(backend.last_content().terminal_size.rows()).max(2);
        let command = match key {
            ModeKey::Char('g') if gg => Some(CopyCommand::Top),
            ModeKey::Char('g') => {
                if let Some((_, KeyMode::Copy { pending_g })) = self.key_modes.active.as_mut() {
                    *pending_g = true;
                }
                None
            }
            ModeKey::Char('y') | ModeKey::Named(egui::Key::Enter) => {
                let text = backend.selectable_content();
                self.leave_key_mode();
                if !text.is_empty() {
                    self.copy_with_note(ctx, text, "Selection");
                }
                return;
            }
            ModeKey::Char('q') => {
                self.leave_key_mode();
                return;
            }
            ModeKey::Named(egui::Key::Escape) => {
                if !backend.has_selection() {
                    self.leave_key_mode();
                    return;
                }
                Some(CopyCommand::ClearSelection)
            }
            ModeKey::Char('[' | ']') => {
                backend.jump_to_prompt(key == ModeKey::Char(']'));
                None
            }
            other => copy_command(other, rows),
        };
        if let Some(command) = command
            && let Some(backend) = self.backends.get_mut(sid)
        {
            backend.copy_mode_command(command);
        }
    }

    /// Mode badge, hint labels and the copy-mode cursor over the pane.
    pub(crate) fn paint_key_mode(&self, ui: &egui::Ui, sid: &str, pane: egui::Rect) {
        let Some(mode) = self.key_modes.on(sid) else {
            return;
        };
        let accent = appearance::color(&self.theme.accent);
        let style = TagStyle {
            fill: accent,
            ink: appearance::color(&self.theme.terminal_background),
            font: egui::FontId::monospace(appearance::FONT_CAPTION),
        };
        let painter = ui.painter().with_clip_rect(pane);
        let badge = match mode {
            KeyMode::Hints(hints) => {
                for (label, target) in &hints.items {
                    if !label.starts_with(&hints.typed) {
                        continue;
                    }
                    let at = target
                        .rects
                        .first()
                        .map_or(pane.min, |r| r.translate(pane.min.to_vec2()).min);
                    paint_tag(&painter, (at, false), label, &style);
                }
                "HINTS"
            }
            KeyMode::Copy { .. } => {
                if let Some(cell) = self.copy_cursor_rect(sid, pane) {
                    painter.rect_stroke(
                        cell,
                        egui::CornerRadius::ZERO,
                        egui::Stroke::new(1.5, accent),
                        egui::StrokeKind::Inside,
                    );
                }
                "COPY"
            }
        };
        let corner = egui::pos2(pane.right() - 4.0, pane.top() + 4.0);
        paint_tag(&painter, (corner, true), badge, &style);
    }

    /// The copy-mode cursor cell in screen coordinates, when visible.
    fn copy_cursor_rect(&self, sid: &str, pane: egui::Rect) -> Option<egui::Rect> {
        let backend = self.backends.get(sid)?;
        let (row, column) = backend.copy_mode_cursor()?;
        let size = backend.last_content().terminal_size;
        let (width, height) = (f32::from(size.cell_width), f32::from(size.cell_height));
        let min = egui::pos2(
            pane.left() + column as f32 * width,
            pane.top() + row as f32 * height,
        );
        Some(egui::Rect::from_min_size(min, egui::vec2(width, height)))
    }
}

/// Colors and font of a hint label or mode badge.
struct TagStyle {
    fill: egui::Color32,
    ink: egui::Color32,
    font: egui::FontId,
}

/// A small filled tag with `text`, its top-left (or, with `right`, its
/// top-right) corner at `at`.
fn paint_tag(painter: &egui::Painter, at: (egui::Pos2, bool), text: &str, style: &TagStyle) {
    let (at, right) = at;
    let galley = painter.layout_no_wrap(text.to_owned(), style.font.clone(), style.ink);
    let size = galley.size();
    let width = size.x + 6.0;
    let left = if right { at.x - width } else { at.x };
    let rect = egui::Rect::from_min_size(egui::pos2(left, at.y), egui::vec2(width, size.y));
    painter.rect_filled(rect, appearance::RADIUS_XSMALL, style.fill);
    painter.galley(egui::pos2(rect.left() + 3.0, rect.top()), galley, style.ink);
}

/// The vi motion or scroll for a copy-mode key.
pub(super) fn copy_command(key: ModeKey, rows: i32) -> Option<CopyCommand> {
    use egui::Key;
    let half = rows / 2;
    // Positive scrolls into history (up).
    let (down_half, down_page) = (half.saturating_neg(), rows.saturating_neg());
    let motion = match key {
        ModeKey::Char('h') | ModeKey::Named(Key::ArrowLeft) => ViMotion::Left,
        ModeKey::Char('j') | ModeKey::Named(Key::ArrowDown) => ViMotion::Down,
        ModeKey::Char('k') | ModeKey::Named(Key::ArrowUp) => ViMotion::Up,
        ModeKey::Char('l') | ModeKey::Named(Key::ArrowRight) => ViMotion::Right,
        ModeKey::Char('w') => ViMotion::SemanticRight,
        ModeKey::Char('b') => ViMotion::SemanticLeft,
        ModeKey::Char('e') => ViMotion::SemanticRightEnd,
        ModeKey::Char('W') => ViMotion::WordRight,
        ModeKey::Char('B') => ViMotion::WordLeft,
        ModeKey::Char('E') => ViMotion::WordRightEnd,
        ModeKey::Char('0') | ModeKey::Named(Key::Home) => ViMotion::First,
        ModeKey::Char('^') => ViMotion::FirstOccupied,
        ModeKey::Char('$') | ModeKey::Named(Key::End) => ViMotion::Last,
        ModeKey::Char('H') => ViMotion::High,
        ModeKey::Char('M') => ViMotion::Middle,
        ModeKey::Char('L') => ViMotion::Low,
        ModeKey::Char('%') => ViMotion::Bracket,
        ModeKey::Char('{') => ViMotion::ParagraphUp,
        ModeKey::Char('}') => ViMotion::ParagraphDown,
        ModeKey::Char('G') => return Some(CopyCommand::Bottom),
        ModeKey::Char('v') => return Some(CopyCommand::Select(SelectionType::Simple)),
        ModeKey::Char('V') => return Some(CopyCommand::Select(SelectionType::Lines)),
        ModeKey::Ctrl('v') => return Some(CopyCommand::Select(SelectionType::Block)),
        ModeKey::Ctrl('u') => return Some(CopyCommand::Scroll(half)),
        ModeKey::Ctrl('d') => return Some(CopyCommand::Scroll(down_half)),
        ModeKey::Ctrl('b') | ModeKey::Named(Key::PageUp) => return Some(CopyCommand::Scroll(rows)),
        ModeKey::Ctrl('f') | ModeKey::Named(Key::PageDown) => {
            return Some(CopyCommand::Scroll(down_page));
        }
        _ => return None,
    };
    Some(CopyCommand::Motion(motion))
}

#[cfg(test)]
#[path = "key_mode_tests.rs"]
mod tests;
