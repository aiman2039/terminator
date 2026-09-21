use alacritty_terminal::index::Line;
use alacritty_terminal::index::Point as TerminalGridPoint;
use alacritty_terminal::term::cell;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use egui::epaint::RectShape;
use egui::text::{LayoutJob, TextFormat};
use egui::Modifiers;
use egui::MouseWheelUnit;
use egui::Shape;
use egui::Widget;
use egui::{Align, Color32, FontFamily, FontId, Painter, Pos2, Rect, Response, Stroke, Vec2};
use egui::{CornerRadius, Key};
use egui::{Id, PointerButton};

use crate::backend::BackendCommand;
use crate::backend::FoundMatch;
use crate::backend::TerminalBackend;
use crate::backend::{LinkAction, MouseButton, SelectionType};
use crate::bindings::Binding;
use crate::bindings::{BindingAction, BindingsLayout, InputKind};
use crate::font::TerminalFont;
use crate::theme::TerminalTheme;
use crate::types::Size;

const EGUI_TERM_WIDGET_ID_PREFIX: &str = "egui_term::instance::";

#[derive(Debug, Clone)]
enum InputAction {
    BackendCall(BackendCommand),
    WriteToClipboard(String),
    Ignore,
}

#[derive(Clone, Default)]
pub struct TerminalViewState {
    is_dragged: bool,
    scroll_pixels: f32,
    scroll_lines: f32,
    scroll_time: Option<f64>,
    current_mouse_position_on_grid: TerminalGridPoint,
}

/// Match highlight overlay for [`TerminalView`]. Ranges use live grid
/// coordinates (see [`crate::backend::FoundMatch`]).
#[derive(Clone, Debug, Default)]
pub struct FindPaint {
    pub matches: Vec<FoundMatch>,
    pub current: usize,
}

pub struct TerminalView<'a> {
    widget_id: Id,
    has_focus: bool,
    size: Vec2,
    backend: &'a mut TerminalBackend,
    font: TerminalFont,
    theme: TerminalTheme,
    external_links: bool,
    bindings_layout: BindingsLayout,
    find: Option<FindPaint>,
}

impl Widget for TerminalView<'_> {
    fn ui(self, ui: &mut egui::Ui) -> Response {
        let (layout, painter) = ui.allocate_painter(self.size, egui::Sense::click_and_drag());

        let widget_id = self.widget_id;
        let mut state = ui.memory(|m| {
            m.data
                .get_temp::<TerminalViewState>(widget_id)
                .unwrap_or_default()
        });

        self.focus(&layout)
            .resize(&layout)
            .process_input(&layout, &mut state)
            .show(&mut state, &layout, &painter);

        ui.memory_mut(|m| m.data.insert_temp(widget_id, state));
        layout
    }
}

impl<'a> TerminalView<'a> {
    pub fn new(ui: &mut egui::Ui, backend: &'a mut TerminalBackend) -> Self {
        let widget_id =
            ui.make_persistent_id(format!("{}{}", EGUI_TERM_WIDGET_ID_PREFIX, backend.id()));

        Self {
            widget_id,
            has_focus: false,
            size: ui.available_size(),
            backend,
            font: TerminalFont::default(),
            theme: TerminalTheme::default(),
            external_links: false,
            bindings_layout: BindingsLayout::new(),
            find: None,
        }
    }

    /// Overlay literal-find match highlights. The current match paints in the
    /// selection color, other matches dimmed. Coordinates are live grid points.
    pub fn find_highlight(mut self, find: Option<FindPaint>) -> Self {
        self.find = find;
        self
    }

    #[inline]
    pub fn set_theme(mut self, theme: TerminalTheme) -> Self {
        self.theme = theme;
        self
    }

    #[inline]
    pub fn set_font(mut self, font: TerminalFont) -> Self {
        self.font = font;
        self
    }

    /// Delegate modified-click link actions to the embedding application.
    pub fn external_links(mut self, enabled: bool) -> Self {
        self.external_links = enabled;
        self
    }
    #[inline]
    pub fn set_focus(mut self, has_focus: bool) -> Self {
        self.has_focus = has_focus;
        self
    }

    #[inline]
    pub fn set_size(mut self, size: Vec2) -> Self {
        self.size = size;
        self
    }

    #[inline]
    pub fn add_bindings(mut self, bindings: Vec<(Binding<InputKind>, BindingAction)>) -> Self {
        self.bindings_layout.add_bindings(bindings);
        self
    }

    fn focus(self, layout: &Response) -> Self {
        if self.has_focus {
            focus_terminal(layout);
        } else {
            layout.surrender_focus();
        }

        self
    }

    fn resize(self, layout: &Response) -> Self {
        self.backend.process_command(BackendCommand::Resize(
            Size::from(layout.rect.size()),
            self.font.font_measure(&layout.ctx),
        ));

        self
    }

    fn process_input(self, layout: &Response, state: &mut TerminalViewState) -> Self {
        let wheel_target = layout.enabled() && layout.contains_pointer();
        if !wheel_target {
            state.scroll_pixels = 0.0;
            state.scroll_lines = 0.0;
            state.scroll_time = None;
        }
        if wheel_target {
            // Egui can smooth a wheel tick over subsequent frames without another raw event.
            layout.ctx.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
        let modifiers = layout.ctx.input(|i| i.modifiers);
        let events = layout.ctx.input(|i| i.events.clone());
        let ctrl_text = ctrl_text_mask(&events, modifiers);
        for event in events {
            if !input_event_applies(&event, layout, state.is_dragged, wheel_target) {
                continue;
            }
            if self.external_links
                && matches!(&event, egui::Event::PointerButton {button: PointerButton::Primary, modifiers, ..} if if cfg!(target_os = "macos") { modifiers.mac_cmd } else {modifiers.ctrl})
                && !self
                    .backend
                    .last_content()
                    .terminal_mode
                    .intersects(TermMode::MOUSE_MODE)
            {
                continue;
            }
            let mut input_actions = vec![];

            let is_key = matches!(event, egui::Event::Key { .. });
            match event {
                egui::Event::Text(_)
                | egui::Event::Key { .. }
                | egui::Event::Copy
                | egui::Event::Paste(_) => input_actions.push(skip_repeated_ctrl_letter(
                    process_keyboard_event(event, self.backend, &self.bindings_layout, modifiers),
                    is_key,
                    ctrl_text,
                )),
                egui::Event::MouseWheel {
                    unit,
                    delta,
                    phase,
                    modifiers,
                } => {
                    let now = layout.ctx.input(|i| i.time);
                    if matches!(phase, egui::TouchPhase::Start | egui::TouchPhase::Cancel)
                        || state.scroll_time.is_some_and(|last| now - last > 0.5)
                    {
                        state.scroll_pixels = 0.0;
                        state.scroll_lines = 0.0;
                    }
                    state.scroll_time = Some(now);
                    if let Some(pos) = layout.ctx.input(|i| i.pointer.hover_pos()) {
                        let content = self.backend.last_content();
                        state.current_mouse_position_on_grid = TerminalBackend::selection_point(
                            pos.x - layout.rect.min.x,
                            pos.y - layout.rect.min.y,
                            &content.terminal_size,
                            0, // Application mouse reports use viewport coordinates.
                        );
                    }
                    // Consume both representations before an enclosing ScrollArea sees them.
                    layout.ctx.input_mut(|i| {
                        i.events
                            .retain(|event| !matches!(event, egui::Event::MouseWheel { .. }));
                        i.smooth_scroll_delta = Vec2::ZERO;
                    });
                    if phase == egui::TouchPhase::Cancel {
                        continue;
                    }
                    input_actions.extend(process_mouse_wheel(
                        state,
                        self.font.font_measure(&layout.ctx).height,
                        (layout.rect.height() / self.font.font_measure(&layout.ctx).height.max(1.0))
                            as usize,
                        unit,
                        delta,
                        self.backend.last_content().terminal_mode,
                        modifiers,
                    ))
                }
                egui::Event::PointerButton {
                    button,
                    pressed,
                    modifiers,
                    pos,
                    ..
                } => input_actions.push(process_button_click(
                    state,
                    layout,
                    self.backend,
                    &self.bindings_layout,
                    button,
                    pos,
                    &modifiers,
                    pressed,
                )),
                egui::Event::PointerMoved(pos) => {
                    input_actions = process_mouse_move(state, layout, self.backend, pos, &modifiers)
                }
                _ => {}
            };

            for action in input_actions {
                match action {
                    InputAction::BackendCall(cmd) => {
                        self.backend.process_command(cmd);
                    }
                    InputAction::WriteToClipboard(data) => {
                        layout.ctx.copy_text(data);
                    }
                    InputAction::Ignore => {}
                }
            }
        }

        self
    }

    fn show(self, state: &mut TerminalViewState, layout: &Response, painter: &Painter) {
        let content = self.backend.sync();
        painter.extend(paint_terminal(
            &self.theme,
            self.font.font_type(),
            layout,
            painter,
            state,
            content,
            self.find.as_ref(),
        ));
    }
}

fn live_point(point: TerminalGridPoint, display_offset: usize) -> TerminalGridPoint {
    TerminalGridPoint::new(Line(point.line.0 - display_offset as i32), point.column)
}

struct TextRun {
    origin: Pos2,
    end_x: f32,
    y: f32,
    fg: Color32,
    font: FontId,
    text: String,
    tracking: f32,
}

fn paint_terminal(
    theme: &TerminalTheme,
    regular: FontId,
    layout: &Response,
    painter: &Painter,
    state: &TerminalViewState,
    content: &crate::backend::RenderableContent,
    find: Option<&FindPaint>,
) -> Vec<Shape> {
    let layout_min = layout.rect.min;
    let cell_height = content.terminal_size.cell_height as f32;
    let cell_width = content.terminal_size.cell_width as f32;
    let global_bg = theme.get_color(Color::Named(NamedColor::Background));
    let selection_bg = layout.ctx.global_style().visuals.selection.bg_fill;
    let bold = layout.ctx.fonts_mut(|fonts| {
        fonts
            .families()
            .contains(&FontFamily::Name("Terminal Bold".into()))
    });
    let bold_font = FontId::new(regular.size, FontFamily::Name("Terminal Bold".into()));
    let app_cursor = content.terminal_mode.contains(TermMode::APP_CURSOR);
    let mut shapes = vec![Shape::Rect(RectShape::filled(
        Rect::from_min_max(layout_min, layout.rect.max),
        CornerRadius::ZERO,
        global_bg,
    ))];
    let tracking = cell_width
        - layout
            .ctx
            .fonts_mut(|fonts| fonts.glyph_width(&regular, 'm'));
    let mut run = TextRun {
        origin: Pos2::ZERO,
        end_x: f32::NAN,
        y: f32::NAN,
        fg: Color32::TRANSPARENT,
        font: regular.clone(),
        text: String::new(),
        tracking,
    };
    // Group match ranges by grid line once per frame so per-cell lookup is cheap.
    let find_map: std::collections::HashMap<i32, Vec<(usize, usize, bool)>> = match find {
        Some(paint) => {
            let mut map = std::collections::HashMap::new();
            for (index, m) in paint.matches.iter().enumerate() {
                map.entry(m.line).or_insert_with(Vec::new).push((
                    m.start_col,
                    m.end_col,
                    index == paint.current,
                ));
            }
            map
        }
        None => std::collections::HashMap::new(),
    };

    for indexed in content.grid.display_iter() {
        if indexed.cell.flags.contains(cell::Flags::WIDE_CHAR_SPACER) {
            flush_text_run(painter, &mut shapes, &mut run);
            continue;
        }
        let live = live_point(indexed.point, content.display_offset);
        let selected = content
            .selectable_range
            .is_some_and(|range| range.contains(live));
        let hovered = content.hovered_hyperlink.as_ref().is_some_and(|range| {
            range.contains(&live) && range.contains(&state.current_mouse_position_on_grid)
        });
        let x = layout_min.x + (cell_width * indexed.point.column.0 as f32);
        let y = layout_min.y
            + (cell_height * (indexed.point.line.0 + content.grid.display_offset() as i32) as f32);
        let wide = indexed.cell.flags.contains(cell::Flags::WIDE_CHAR);
        let width = if wide { cell_width * 2.0 } else { cell_width };
        let mut fg = theme.get_color(indexed.fg);
        let mut bg = theme.get_color(indexed.bg);
        if indexed
            .cell
            .flags
            .intersects(cell::Flags::DIM | cell::Flags::DIM_BOLD)
        {
            fg = fg.linear_multiply(0.7);
        }
        if indexed.cell.flags.contains(cell::Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if selected {
            bg = selection_bg;
        }
        if let Some(ranges) = find_map.get(&live.line.0) {
            let col = live.column.0;
            if ranges
                .iter()
                .any(|(s, e, cur)| *cur && col >= *s && col <= *e)
            {
                bg = selection_bg;
            } else if ranges.iter().any(|(s, e, _)| col >= *s && col <= *e) {
                bg = selection_bg.gamma_multiply(0.45);
            }
        }
        if global_bg != bg {
            shapes.push(Shape::Rect(RectShape::filled(
                Rect::from_min_size(Pos2::new(x, y), Vec2::new(width + 1., cell_height + 1.)),
                CornerRadius::ZERO,
                bg,
            )));
        }
        if hovered {
            shapes.push(Shape::LineSegment {
                points: [
                    Pos2::new(x, y + cell_height),
                    Pos2::new(x + width, y + cell_height),
                ],
                stroke: Stroke::new(cell_height * 0.15, fg),
            });
        }
        let on_cursor = content.grid.cursor.point == indexed.point;
        if on_cursor {
            shapes.push(Shape::Rect(RectShape::filled(
                Rect::from_min_size(Pos2::new(x, y), Vec2::new(width, cell_height)),
                CornerRadius::default(),
                theme.get_color(content.cursor.fg),
            )));
        }
        if indexed.c == ' ' || indexed.c == '\t' {
            flush_text_run(painter, &mut shapes, &mut run);
            continue;
        }
        if on_cursor && app_cursor {
            fg = bg;
        }
        let font = if indexed.cell.flags.contains(cell::Flags::BOLD) && bold {
            bold_font.clone()
        } else {
            regular.clone()
        };
        let invert_cursor = on_cursor && app_cursor;
        if invert_cursor || wide || !text_run_continues(&run, x, y, fg, &font) {
            flush_text_run(painter, &mut shapes, &mut run);
            run.origin = Pos2::new(x, y);
            run.end_x = x;
            run.y = y;
            run.fg = fg;
            run.font = font;
        }
        run.text.push(indexed.c);
        run.end_x += width;
        if invert_cursor || wide {
            flush_text_run(painter, &mut shapes, &mut run);
        }
    }
    flush_text_run(painter, &mut shapes, &mut run);
    shapes
}

fn text_run_continues(run: &TextRun, x: f32, y: f32, fg: Color32, font: &FontId) -> bool {
    !run.text.is_empty() && run.y == y && run.end_x == x && run.fg == fg && run.font == *font
}

fn flush_text_run(painter: &Painter, shapes: &mut Vec<Shape>, run: &mut TextRun) {
    if run.text.is_empty() {
        return;
    }
    let font = run.font.clone();
    let text = std::mem::take(&mut run.text);
    let origin = run.origin;
    let fg = run.fg;
    let tracking = run.tracking;
    let mut job = LayoutJob {
        break_on_newline: false,
        ..Default::default()
    };
    job.append(
        &text,
        0.0,
        TextFormat {
            font_id: font,
            color: fg,
            extra_letter_spacing: tracking,
            valign: Align::TOP,
            ..Default::default()
        },
    );
    let galley = painter.fonts_mut(|fonts| fonts.layout_job(job));
    shapes.push(Shape::galley(origin, galley, fg));
}

fn focus_terminal(layout: &Response) {
    layout.request_focus();
    // These keys belong to the shell/editor, not egui's widget navigation.
    // Otherwise Tab can briefly focus and highlight a dock separator.
    layout.ctx.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            layout.id,
            egui::EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: true,
            },
        );
    });
}

#[cfg(test)]
mod focus_tests {
    use super::*;

    #[test]
    fn terminal_navigation_keys_do_not_focus_the_separator() {
        for (key, modifiers) in [
            (Key::Tab, Modifiers::NONE),
            (Key::Tab, Modifiers::SHIFT),
            (Key::ArrowLeft, Modifiers::NONE),
            (Key::ArrowRight, Modifiers::NONE),
            (Key::ArrowUp, Modifiers::NONE),
            (Key::ArrowDown, Modifiers::NONE),
            (Key::Escape, Modifiers::NONE),
        ] {
            let ctx = egui::Context::default();
            for frame in 0..4 {
                let mut input = egui::RawInput::default();
                if frame >= 2 {
                    input.events.push(egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: frame > 2,
                        modifiers,
                    });
                }
                let mut output = ctx.run_ui(input, |ui| {
                    ui.scope(|ui| {
                        let terminal =
                            ui.allocate_response(Vec2::splat(100.0), egui::Sense::click());
                        if frame >= 2 {
                            assert!(terminal.has_focus(), "terminal lost focus on {key:?}");
                            assert!(ui.input(|i| i.key_pressed(key)));
                        }
                        focus_terminal(&terminal);
                        let separator = ui.allocate_response(
                            Vec2::new(4.0, 100.0),
                            egui::Sense::click_and_drag(),
                        );
                        assert!(!separator.has_focus(), "separator focused on {key:?}");
                    });
                });
                output.textures_delta.clear();
            }
        }
    }
}

fn process_keyboard_event(
    event: egui::Event,
    backend: &TerminalBackend,
    bindings_layout: &BindingsLayout,
    modifiers: Modifiers,
) -> InputAction {
    match event {
        egui::Event::Text(text) => process_text_event(&text, modifiers, backend, bindings_layout),
        egui::Event::Paste(text) => InputAction::BackendCall(
            #[cfg(not(any(target_os = "ios", target_os = "macos")))]
            if modifiers.contains(Modifiers::COMMAND | Modifiers::SHIFT) {
                BackendCommand::Write(text.as_bytes().to_vec())
            } else {
                // Hotfix - Send ^V when there's not selection on view.
                BackendCommand::Write([0x16].to_vec())
            },
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            {
                BackendCommand::Write(text.as_bytes().to_vec())
            },
        ),
        egui::Event::Copy => copy_input_action(backend.selectable_content(), modifiers),
        egui::Event::Key {
            key,
            pressed,
            modifiers,
            ..
        } => process_keyboard_key(backend, bindings_layout, key, modifiers, pressed),
        _ => InputAction::Ignore,
    }
}

fn process_text_event(
    text: &str,
    modifiers: Modifiers,
    backend: &TerminalBackend,
    bindings_layout: &BindingsLayout,
) -> InputAction {
    // A Ctrl+letter Text event is the control character. Ignoring it because a
    // binding exists drops Ctrl+E when the Key event was stamped without Control.
    if let Some(bytes) = ctrl_letter_bytes(text, modifiers) {
        return InputAction::BackendCall(BackendCommand::Write(bytes));
    }
    if let Some(key) = Key::from_name(text) {
        if bindings_layout.get_action(
            InputKind::KeyCode(key),
            modifiers,
            backend.last_content().terminal_mode,
        ) == BindingAction::Ignore
        {
            InputAction::BackendCall(BackendCommand::Write(text.as_bytes().to_vec()))
        } else {
            InputAction::Ignore
        }
    } else {
        InputAction::BackendCall(BackendCommand::Write(text.as_bytes().to_vec()))
    }
}

fn process_keyboard_key(
    backend: &TerminalBackend,
    bindings_layout: &BindingsLayout,
    key: Key,
    modifiers: Modifiers,
    pressed: bool,
) -> InputAction {
    if !pressed {
        return InputAction::Ignore;
    }
    match crate::keyboard::bytes_for_pressed_key(
        bindings_layout,
        key,
        modifiers,
        backend.last_content().terminal_mode,
    ) {
        Some(bytes) => InputAction::BackendCall(BackendCommand::Write(bytes)),
        None => InputAction::Ignore,
    }
}

fn process_mouse_wheel(
    state: &mut TerminalViewState,
    font_size: f32,
    page_lines: usize,
    unit: MouseWheelUnit,
    delta: Vec2,
    terminal_mode: TermMode,
    modifiers: Modifiers,
) -> Vec<InputAction> {
    if !delta.is_finite() {
        return vec![];
    }
    let font_size = font_size.max(1.0);
    let lines = match unit {
        MouseWheelUnit::Line | MouseWheelUnit::Page => {
            state.scroll_lines += delta.y
                * if unit == MouseWheelUnit::Page {
                    page_lines as f32
                } else {
                    1.0
                };
            let lines = state.scroll_lines.trunc() as i32;
            state.scroll_lines -= lines as f32;
            lines
        }
        MouseWheelUnit::Point => {
            state.scroll_pixels += delta.y;
            let lines = (state.scroll_pixels / font_size).trunc() as i32;
            state.scroll_pixels %= font_size;
            lines
        }
    };
    let lines = lines.clamp(-1000, 1000);
    if lines == 0 {
        return vec![];
    }
    // Full-screen agents own their history and expect wheel reports, not
    // alternate-screen arrow keys. Shift retains local terminal scrolling.
    if terminal_mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift {
        (0..lines.unsigned_abs())
            .map(|_| {
                InputAction::BackendCall(BackendCommand::MouseReport(
                    if lines > 0 {
                        MouseButton::ScrollUp
                    } else {
                        MouseButton::ScrollDown
                    },
                    modifiers,
                    state.current_mouse_position_on_grid,
                    true,
                ))
            })
            .collect()
    } else {
        vec![InputAction::BackendCall(if modifiers.shift {
            BackendCommand::ScrollLocal(lines)
        } else {
            BackendCommand::Scroll(lines)
        })]
    }
}

fn process_button_click(
    state: &mut TerminalViewState,
    layout: &Response,
    backend: &TerminalBackend,
    bindings_layout: &BindingsLayout,
    button: PointerButton,
    position: Pos2,
    modifiers: &Modifiers,
    pressed: bool,
) -> InputAction {
    match button {
        PointerButton::Primary => process_left_button(
            state,
            layout,
            backend,
            bindings_layout,
            position,
            modifiers,
            pressed,
        ),
        _ => InputAction::Ignore,
    }
}

fn process_left_button(
    state: &mut TerminalViewState,
    layout: &Response,
    backend: &TerminalBackend,
    bindings_layout: &BindingsLayout,
    position: Pos2,
    modifiers: &Modifiers,
    pressed: bool,
) -> InputAction {
    refresh_pointer_cell(state, layout, backend, position);
    state.is_dragged = pressed;
    if pressed {
        process_left_button_pressed(state, layout, position)
    } else {
        process_left_button_released(state, layout, backend, bindings_layout, position, modifiers)
    }
}

fn process_left_button_pressed(
    state: &mut TerminalViewState,
    layout: &Response,
    position: Pos2,
) -> InputAction {
    state.is_dragged = true;
    InputAction::BackendCall(build_start_select_command(layout, position))
}

fn process_left_button_released(
    state: &mut TerminalViewState,
    layout: &Response,
    backend: &TerminalBackend,
    bindings_layout: &BindingsLayout,
    position: Pos2,
    modifiers: &Modifiers,
) -> InputAction {
    state.is_dragged = false;
    if layout.double_clicked() || layout.triple_clicked() {
        InputAction::BackendCall(build_start_select_command(layout, position))
    } else {
        let terminal_content = backend.last_content();
        let binding_action = bindings_layout.get_action(
            InputKind::Mouse(PointerButton::Primary),
            *modifiers,
            terminal_content.terminal_mode,
        );

        if binding_action == BindingAction::LinkOpen {
            InputAction::BackendCall(BackendCommand::ProcessLink(
                LinkAction::Open,
                state.current_mouse_position_on_grid,
            ))
        } else {
            InputAction::Ignore
        }
    }
}

fn build_start_select_command(layout: &Response, cursor_position: Pos2) -> BackendCommand {
    let selection_type = if layout.double_clicked() {
        SelectionType::Semantic
    } else if layout.triple_clicked() {
        SelectionType::Lines
    } else {
        SelectionType::Simple
    };

    BackendCommand::SelectStart(
        selection_type,
        cursor_position.x - layout.rect.min.x,
        cursor_position.y - layout.rect.min.y,
    )
}

fn process_mouse_move(
    state: &mut TerminalViewState,
    layout: &Response,
    backend: &TerminalBackend,
    position: Pos2,
    modifiers: &Modifiers,
) -> Vec<InputAction> {
    let terminal_content = backend.last_content();
    let cursor_x = position.x - layout.rect.min.x;
    let cursor_y = position.y - layout.rect.min.y;
    state.current_mouse_position_on_grid = TerminalBackend::selection_point(
        cursor_x,
        cursor_y,
        &terminal_content.terminal_size,
        terminal_content.display_offset,
    );

    let mut actions = Vec::new();
    if state.is_dragged {
        actions.extend(drag_actions(cursor_x, cursor_y, layout.rect.height()));
    }

    if modifiers.command_only() {
        actions.push(InputAction::BackendCall(BackendCommand::ProcessLink(
            LinkAction::Hover,
            state.current_mouse_position_on_grid,
        )));
    }

    actions
}

fn refresh_pointer_cell(
    state: &mut TerminalViewState,
    layout: &Response,
    backend: &TerminalBackend,
    position: Pos2,
) {
    let content = backend.last_content();
    state.current_mouse_position_on_grid = TerminalBackend::selection_point(
        position.x - layout.rect.min.x,
        position.y - layout.rect.min.y,
        &content.terminal_size,
        content.display_offset,
    );
}

fn ctrl_letters_apply(modifiers: Modifiers) -> bool {
    modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd
}

/// Bit `0` is Ctrl+A, bit `4` is Ctrl+E. Letters and the C0 bytes both count.
fn control_bit(text: &str) -> Option<u8> {
    let mut chars = text.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if c.is_ascii_alphabetic() {
        return Some(c.to_ascii_lowercase() as u8 - b'a');
    }
    let code = u32::from(c);
    if (1..=26).contains(&code) {
        Some(code as u8 - 1)
    } else {
        None
    }
}

fn ctrl_letter_bytes(text: &str, modifiers: Modifiers) -> Option<Vec<u8>> {
    if !ctrl_letters_apply(modifiers) {
        return None;
    }
    let c = text.chars().next()?;
    if !c.is_ascii_alphabetic() {
        return None;
    }
    control_bit(text).map(|bit| vec![bit + 1])
}

fn ctrl_text_mask(events: &[egui::Event], modifiers: Modifiers) -> u32 {
    if !ctrl_letters_apply(modifiers) {
        return 0;
    }
    let mut mask = 0u32;
    for event in events {
        if let egui::Event::Text(text) = event {
            if let Some(bit) = control_bit(text) {
                mask |= 1 << bit;
            }
        }
    }
    mask
}

fn skip_repeated_ctrl_letter(action: InputAction, is_key: bool, ctrl_text: u32) -> InputAction {
    if !is_key {
        return action;
    }
    let repeated = match &action {
        InputAction::BackendCall(BackendCommand::Write(bytes)) => match bytes.as_slice() {
            [byte] if (1..=26).contains(byte) => ctrl_text & (1 << (byte - 1)) != 0,
            _ => false,
        },
        _ => false,
    };
    if repeated {
        InputAction::Ignore
    } else {
        action
    }
}

#[cfg(test)]
mod ctrl_letter_tests {
    use super::{ctrl_letter_bytes, ctrl_text_mask, skip_repeated_ctrl_letter, InputAction};
    use crate::backend::BackendCommand;
    use egui::Modifiers;

    #[test]
    fn control_e_text_is_enq() {
        assert_eq!(ctrl_letter_bytes("e", Modifiers::CTRL), Some(vec![0x05]));
        assert_eq!(ctrl_letter_bytes("E", Modifiers::CTRL), Some(vec![0x05]));
        assert_eq!(ctrl_letter_bytes("a", Modifiers::CTRL), Some(vec![0x01]));
        assert_eq!(ctrl_letter_bytes("e", Modifiers::NONE), None);
        assert_eq!(ctrl_letter_bytes("\u{5}", Modifiers::CTRL), None);
    }

    #[test]
    fn key_enq_is_not_repeated_when_text_already_carries_e() {
        let events = [egui::Event::Text("e".into())];
        let mask = ctrl_text_mask(&events, Modifiers::CTRL);
        let action = InputAction::BackendCall(BackendCommand::Write(vec![0x05]));
        assert!(matches!(
            skip_repeated_ctrl_letter(action, true, mask),
            InputAction::Ignore
        ));
        let only_key = InputAction::BackendCall(BackendCommand::Write(vec![0x05]));
        assert!(matches!(
            skip_repeated_ctrl_letter(only_key, true, 0),
            InputAction::BackendCall(BackendCommand::Write(bytes)) if bytes.as_slice() == [0x05]
        ));
    }
}

fn input_event_applies(
    event: &egui::Event,
    layout: &Response,
    is_dragged: bool,
    wheel_target: bool,
) -> bool {
    if !layout.enabled() {
        return false;
    }
    match event {
        egui::Event::MouseWheel { .. } => wheel_target,
        egui::Event::PointerButton { pressed, .. } => {
            layout.contains_pointer() || (!pressed && is_dragged)
        }
        egui::Event::PointerMoved(_) => layout.contains_pointer() || is_dragged,
        _ => layout.has_focus(),
    }
}

fn copy_input_action(content: String, modifiers: Modifiers) -> InputAction {
    if content.is_empty() {
        interrupt_if_unshifted_copy(modifiers)
    } else {
        InputAction::WriteToClipboard(content)
    }
}

#[cfg(not(any(target_os = "ios", target_os = "macos")))]
fn interrupt_if_unshifted_copy(modifiers: Modifiers) -> InputAction {
    if modifiers.shift {
        InputAction::Ignore
    } else {
        InputAction::BackendCall(BackendCommand::Write([0x3].to_vec()))
    }
}

#[cfg(any(target_os = "ios", target_os = "macos"))]
fn interrupt_if_unshifted_copy(_modifiers: Modifiers) -> InputAction {
    InputAction::Ignore
}

fn drag_actions(cursor_x: f32, cursor_y: f32, layout_height: f32) -> Vec<InputAction> {
    let mut actions = Vec::new();
    if cursor_y < 0.0 {
        actions.push(InputAction::BackendCall(BackendCommand::ScrollLocal(1)));
    } else if cursor_y > layout_height {
        actions.push(InputAction::BackendCall(BackendCommand::ScrollLocal(-1)));
    }
    actions.push(InputAction::BackendCall(BackendCommand::SelectUpdate(
        cursor_x, cursor_y,
    )));
    actions
}

#[cfg(test)]
mod scroll_tests {
    use super::*;

    #[test]
    fn agent_mouse_mode_receives_wheel_reports_in_both_directions() {
        let mut state = TerminalViewState::default();
        for (delta, expected) in [(2.0, 64), (-2.0, 65)] {
            let actions = process_mouse_wheel(
                &mut state,
                16.0,
                24,
                MouseWheelUnit::Line,
                Vec2::new(0.0, delta),
                TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE | TermMode::ALT_SCREEN,
                Modifiers::NONE,
            );
            assert_eq!(actions.len(), 2);
            for action in actions {
                match action {
                    InputAction::BackendCall(BackendCommand::MouseReport(
                        button,
                        _,
                        _,
                        pressed,
                    )) => {
                        assert_eq!(button as u8, expected);
                        assert!(pressed);
                    }
                    _ => panic!("expected a wheel report"),
                }
            }
        }
    }

    #[test]
    fn trackpad_accumulates_partial_lines_for_shell_scrollback() {
        let mut state = TerminalViewState::default();
        for _ in 0..3 {
            assert!(process_mouse_wheel(
                &mut state,
                16.0,
                24,
                MouseWheelUnit::Point,
                Vec2::new(0.0, 4.0),
                TermMode::empty(),
                Modifiers::NONE
            )
            .is_empty());
        }
        let actions = process_mouse_wheel(
            &mut state,
            16.0,
            24,
            MouseWheelUnit::Point,
            Vec2::new(0.0, 4.0),
            TermMode::empty(),
            Modifiers::NONE,
        );
        assert!(matches!(
            actions.as_slice(),
            [InputAction::BackendCall(BackendCommand::Scroll(1))]
        ));
    }

    #[test]
    fn fractional_wheel_lines_accumulate_and_pages_use_viewport_rows() {
        let mut state = TerminalViewState::default();
        assert!(process_mouse_wheel(
            &mut state,
            16.0,
            24,
            MouseWheelUnit::Line,
            Vec2::new(0.0, 0.25),
            TermMode::empty(),
            Modifiers::NONE
        )
        .is_empty());
        let actions = process_mouse_wheel(
            &mut state,
            16.0,
            24,
            MouseWheelUnit::Line,
            Vec2::new(0.0, 0.75),
            TermMode::empty(),
            Modifiers::NONE,
        );
        assert!(matches!(
            actions.as_slice(),
            [InputAction::BackendCall(BackendCommand::Scroll(1))]
        ));
        let actions = process_mouse_wheel(
            &mut state,
            16.0,
            24,
            MouseWheelUnit::Page,
            Vec2::new(0.0, -1.0),
            TermMode::empty(),
            Modifiers::SHIFT,
        );
        assert!(matches!(
            actions.as_slice(),
            [InputAction::BackendCall(BackendCommand::ScrollLocal(-24))]
        ));
    }

    #[test]
    fn shift_wheel_bypasses_application_mouse_reporting() {
        let actions = process_mouse_wheel(
            &mut TerminalViewState::default(),
            16.0,
            24,
            MouseWheelUnit::Line,
            Vec2::new(0.0, -1.0),
            TermMode::MOUSE_REPORT_CLICK | TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL,
            Modifiers::SHIFT,
        );
        assert!(matches!(
            actions.as_slice(),
            [InputAction::BackendCall(BackendCommand::ScrollLocal(-1))]
        ));
    }
}

#[cfg(test)]
mod pointer_tests {
    use super::*;

    #[test]
    fn drag_selects_even_when_the_application_wants_mouse_reports() {
        let actions = drag_actions(4.0, 8.0, 100.0);
        assert!(matches!(
            actions.as_slice(),
            [InputAction::BackendCall(BackendCommand::SelectUpdate(
                4.0, 8.0
            ))]
        ));
    }

    #[test]
    fn host_drag_updates_selection_and_autoscrolls() {
        let actions = drag_actions(4.0, -2.0, 100.0);
        assert!(matches!(
            actions.as_slice(),
            [
                InputAction::BackendCall(BackendCommand::ScrollLocal(1)),
                InputAction::BackendCall(BackendCommand::SelectUpdate(_, _))
            ]
        ));
    }

    #[test]
    fn nonempty_copy_writes_clipboard() {
        let action = copy_input_action("hello".into(), Modifiers::NONE);
        assert!(matches!(action, InputAction::WriteToClipboard(text) if text == "hello"));
    }

    #[test]
    fn empty_shifted_copy_does_not_clear_clipboard() {
        let action = copy_input_action(String::new(), Modifiers::SHIFT);
        assert!(matches!(action, InputAction::Ignore));
    }

    #[cfg(not(any(target_os = "ios", target_os = "macos")))]
    #[test]
    fn empty_ctrl_c_interrupts() {
        let action = copy_input_action(String::new(), Modifiers::NONE);
        assert!(matches!(
            action,
            InputAction::BackendCall(BackendCommand::Write(bytes)) if bytes == [0x3]
        ));
    }

    #[cfg(any(target_os = "ios", target_os = "macos"))]
    #[test]
    fn empty_cmd_c_does_not_clear_clipboard() {
        let action = copy_input_action(String::new(), Modifiers::NONE);
        assert!(matches!(action, InputAction::Ignore));
    }
}

#[cfg(test)]
mod paint_tests {
    use super::*;
    use crate::backend::{RenderableContent, TerminalSize};
    use alacritty_terminal::grid::Grid;
    use alacritty_terminal::index::{Column, Point};
    use alacritty_terminal::term::cell::Cell;
    use alacritty_terminal::vte::ansi::{Color, NamedColor};

    fn content_with(text: &str, split_at: Option<usize>) -> RenderableContent {
        let cols = text.chars().count();
        let mut grid = Grid::<Cell>::new(1, cols.max(1), 0);
        for (col, c) in text.chars().enumerate() {
            let cell = &mut grid[Point::new(Line(0), Column(col))];
            cell.c = c;
            if split_at.is_some_and(|at| col >= at) {
                cell.fg = Color::Named(NamedColor::Red);
            }
        }
        let mut terminal_size = TerminalSize::default();
        terminal_size.cell_width = 8;
        terminal_size.cell_height = 16;
        RenderableContent {
            grid,
            terminal_size,
            ..Default::default()
        }
    }

    fn paint(content: &RenderableContent) -> Vec<Shape> {
        let ctx = egui::Context::default();
        let mut collected = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 80.0))),
                ..Default::default()
            },
            |ui| {
                let (response, painter) =
                    ui.allocate_painter(Vec2::new(320.0, 48.0), egui::Sense::hover());
                collected = Some(paint_terminal(
                    &TerminalTheme::default(),
                    FontId::monospace(13.0),
                    &response,
                    &painter,
                    &TerminalViewState::default(),
                    content,
                    None,
                ));
            },
        );
        output.textures_delta.clear();
        collected.expect("paint")
    }

    fn text_galleys(shapes: &[Shape]) -> Vec<String> {
        shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn live_point_maps_viewport_row_to_term_history() {
        assert_eq!(
            live_point(TerminalGridPoint::new(Line(0), Column(3)), 10),
            TerminalGridPoint::new(Line(-10), Column(3))
        );
        assert_eq!(
            live_point(TerminalGridPoint::new(Line(2), Column(0)), 0),
            TerminalGridPoint::new(Line(2), Column(0))
        );
    }

    #[test]
    fn same_style_cells_paint_as_one_text_shape() {
        let galleys = text_galleys(&paint(&content_with("hello", None)));
        assert_eq!(galleys, ["hello"]);
    }

    #[test]
    fn style_change_splits_text_runs() {
        let galleys = text_galleys(&paint(&content_with("hello", Some(2))));
        assert_eq!(galleys, ["he", "llo"]);
    }

    #[test]
    fn spaces_split_text_runs() {
        let galleys = text_galleys(&paint(&content_with("ab cd", None)));
        assert_eq!(galleys, ["ab", "cd"]);
    }

    #[test]
    fn text_run_glyphs_sit_on_the_cell_grid() {
        let mut content = content_with("hello", None);
        content.terminal_size.cell_width = 12;
        let shapes = paint(&content);
        let text = shapes.iter().find_map(|shape| match shape {
            Shape::Text(text) => Some(text),
            _ => None,
        });
        let text = text.expect("text shape");
        let row = text.galley.rows.first().expect("row");
        let xs: Vec<f32> = row.glyphs.iter().map(|glyph| glyph.pos.x).collect();
        assert_eq!(xs.len(), 5);
        for (index, x) in xs.iter().enumerate() {
            assert!(
                (x - index as f32 * 12.0).abs() < 0.51,
                "glyph {index} at {x}, expected {}",
                index as f32 * 12.0
            );
        }
    }

    #[test]
    fn cursor_sits_on_the_last_glyph_of_a_long_line() {
        let line = "~/RustroverProjects/sphalerite-foundry/zinc-monorepo";
        let last = line.chars().count() - 1;
        let mut content = content_with(line, None);
        content.terminal_size.cell_width = 12;
        content.grid.cursor.point = Point::new(Line(0), Column(last));
        let shapes = paint(&content);
        let cell = 12.0;
        let cursor = shapes.iter().find_map(|shape| match shape {
            Shape::Rect(rect) if (rect.rect.width() - cell).abs() < 0.01 => Some(rect.rect.min.x),
            _ => None,
        });
        let text = shapes.iter().rev().find_map(|shape| match shape {
            Shape::Text(text) => Some(text),
            _ => None,
        });
        let cursor_x = cursor.expect("cursor");
        let text = text.expect("text");
        let glyph = text
            .galley
            .rows
            .first()
            .and_then(|row| row.glyphs.last())
            .expect("last glyph");
        let glyph_x = text.pos.x + glyph.pos.x;
        assert_eq!(glyph.chr, 'o');
        assert!(
            (cursor_x - glyph_x).abs() < 0.51,
            "cursor {cursor_x} last glyph {glyph_x}"
        );
        assert!(cursor_x > 400.0, "expected a long line; cursor {cursor_x}");
    }
}
