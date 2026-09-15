pub mod settings;

use crate::types::Size;
use alacritty_terminal::event::{Event, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{
    Selection, SelectionRange, SelectionType as AlacrittySelectionType,
};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};
use alacritty_terminal::term::{
    self, cell::Cell, test::TermSize, viewport_to_point, Term, TermMode,
};
use alacritty_terminal::{tty, Grid};
use egui::Modifiers;
use settings::BackendSettings;
use std::borrow::Cow;
use std::cmp::min;
use std::io::Result;
use std::ops::{Index, RangeInclusive};
use std::sync::mpsc::Sender;
use std::sync::{mpsc, Arc};

pub type TerminalMode = TermMode;
pub type PtyEvent = Event;
pub type SelectionType = AlacrittySelectionType;

#[derive(Debug, Clone)]
pub enum BackendCommand {
    Write(Vec<u8>),
    Scroll(i32),
    /// Scroll retained history without translating wheel movement into input.
    ScrollLocal(i32),
    Resize(Size, Size),
    SelectStart(SelectionType, f32, f32),
    SelectUpdate(f32, f32),
    ProcessLink(LinkAction, Point),
    MouseReport(MouseButton, Modifiers, Point, bool),
}

#[derive(Debug, Clone)]
pub enum MouseMode {
    Sgr,
    Normal(bool),
}

impl From<TermMode> for MouseMode {
    fn from(term_mode: TermMode) -> Self {
        if term_mode.contains(TermMode::SGR_MOUSE) {
            MouseMode::Sgr
        } else if term_mode.contains(TermMode::UTF8_MOUSE) {
            MouseMode::Normal(true)
        } else {
            MouseMode::Normal(false)
        }
    }
}

#[derive(Debug, Clone)]
pub enum MouseButton {
    LeftButton = 0,
    MiddleButton = 1,
    RightButton = 2,
    LeftMove = 32,
    MiddleMove = 33,
    RightMove = 34,
    NoneMove = 35,
    ScrollUp = 64,
    ScrollDown = 65,
    Other = 99,
}

#[derive(Debug, Clone)]
pub enum LinkAction {
    Clear,
    Hover,
    Open,
}

#[derive(Clone, Copy, Debug)]
pub struct TerminalSize {
    pub cell_width: u16,
    pub cell_height: u16,
    num_cols: u16,
    num_lines: u16,
    layout_size: Size,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            cell_width: 1,
            cell_height: 1,
            num_cols: 80,
            num_lines: 50,
            layout_size: Size::default(),
        }
    }
}

impl Dimensions for TerminalSize {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        self.num_lines as usize
    }

    fn columns(&self) -> usize {
        self.num_cols as usize
    }

    fn last_column(&self) -> Column {
        Column(self.num_cols as usize - 1)
    }

    fn bottommost_line(&self) -> Line {
        Line(self.num_lines as i32 - 1)
    }
}

impl From<TerminalSize> for WindowSize {
    fn from(size: TerminalSize) -> Self {
        Self {
            num_lines: size.num_lines,
            num_cols: size.num_cols,
            cell_width: size.cell_width,
            cell_height: size.cell_height,
        }
    }
}

pub struct TerminalBackend {
    id: u64,
    pty_id: u32,
    url_regex: RegexSearch,
    term: Arc<FairMutex<Term<EventProxy>>>,
    size: TerminalSize,
    notifier: Notifier,
    last_content: RenderableContent,
}

impl TerminalBackend {
    pub fn new(
        id: u64,
        app_context: egui::Context,
        pty_event_proxy_sender: Sender<(u64, PtyEvent)>,
        settings: BackendSettings,
    ) -> Result<Self> {
        let pty_config = tty::Options {
            shell: Some(tty::Shell::new(settings.shell, settings.args)),
            working_directory: settings.working_directory,
            ..tty::Options::default()
        };
        let config = term::Config::default();
        let terminal_size = TerminalSize::default();
        let pty = tty::new(&pty_config, terminal_size.into(), id)?;
        #[cfg(not(windows))]
        let pty_id = pty.child().id();
        #[cfg(windows)]
        let pty_id = pty
            .child_watcher()
            .pid()
            .ok_or(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Failed to get child process ID",
            ))?
            .into();
        let (event_sender, event_receiver) = mpsc::channel();
        let event_proxy = EventProxy(event_sender);
        let mut term = Term::new(config, &terminal_size, event_proxy.clone());
        let initial_content = RenderableContent {
            grid: term.grid().clone(),
            selectable_range: None,
            terminal_mode: *term.mode(),
            terminal_size,
            cursor: term.grid_mut().cursor_cell().clone(),
            hovered_hyperlink: None,
        };
        let term = Arc::new(FairMutex::new(term));
        let pty_event_loop = EventLoop::new(term.clone(), event_proxy, pty, false, false)?;
        let notifier = Notifier(pty_event_loop.channel());

        let url_regex = RegexSearch::new(r#"(ipfs:|ipns:|magnet:|mailto:|gemini://|gopher://|https://|http://|news:|file://|git://|ssh:|ftp://)[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>"\s{-}\^⟨⟩`]+"#).unwrap();
        let _pty_event_loop_thread = pty_event_loop.spawn();
        let _pty_event_subscription = std::thread::Builder::new()
            .name(format!("pty_event_subscription_{}", id))
            .spawn(move || loop {
                if let Ok(event) = event_receiver.recv() {
                    if pty_event_proxy_sender.send((id, event.clone())).is_err() {
                        break;
                    }
                    app_context.clone().request_repaint();
                    match event {
                        Event::Exit => break,
                        // The owning daemon answers terminal queries, including when detached.
                        Event::PtyWrite(_) => {}
                        _ => {}
                    }
                } else {
                    break;
                }
            })?;

        Ok(Self {
            id,
            pty_id,
            url_regex,
            term: term.clone(),
            size: terminal_size,
            notifier,
            last_content: initial_content,
        })
    }

    pub fn process_command(&mut self, cmd: BackendCommand) {
        let term = self.term.clone();
        let mut term = term.lock();
        match cmd {
            BackendCommand::Write(input) => {
                self.write(input);
                term.scroll_display(Scroll::Bottom);
            }
            BackendCommand::ScrollLocal(delta) => {
                term.scroll_display(Scroll::Delta(delta));
            }
            BackendCommand::Scroll(delta) => {
                self.scroll(&mut term, delta);
            }
            BackendCommand::Resize(layout_size, font_size) => {
                self.resize(&mut term, layout_size, font_size);
            }
            BackendCommand::SelectStart(selection_type, x, y) => {
                self.start_selection(&mut term, selection_type, x, y);
            }
            BackendCommand::SelectUpdate(x, y) => {
                self.update_selection(&mut term, x, y);
            }
            BackendCommand::ProcessLink(link_action, point) => {
                self.process_link_action(&term, link_action, point);
            }
            BackendCommand::MouseReport(button, modifiers, point, pressed) => {
                self.process_mouse_report(button, modifiers, point, pressed);
            }
        };
    }

    pub fn selection_point(
        x: f32,
        y: f32,
        terminal_size: &TerminalSize,
        display_offset: usize,
    ) -> Point {
        let col = (x as usize) / (terminal_size.cell_width as usize);
        let col = min(Column(col), Column(terminal_size.num_cols as usize - 1));

        let line = (y as usize) / (terminal_size.cell_height as usize);
        let line = min(line, terminal_size.num_lines as usize - 1);

        viewport_to_point(display_offset, Point::new(line, col))
    }

    /// Token and cell rectangles under a pointer, across logical wrapped lines.
    /// This does no filesystem access and skips wide-character spacer cells.
    pub fn target_at(&self, x: f32, y: f32) -> Option<LinkTarget> {
        target_at_content(self.last_content(), x, y)
    }

    pub fn word_at(&self, x: f32, y: f32) -> String {
        self.target_at(x, y).map(|t| t.text).unwrap_or_default()
    }

    /// Select the terminal grid including retained scrollback, without sending input.
    pub fn select_all(&mut self) {
        let mut term = self.term.lock();
        let grid = term.grid();
        let mut selection = Selection::new(
            AlacrittySelectionType::Simple,
            Point::new(grid.topmost_line(), Column(0)),
            Side::Left,
        );
        selection.update(
            Point::new(grid.bottommost_line(), grid.last_column()),
            Side::Right,
        );
        term.selection = Some(selection);
    }
    pub fn selectable_content(&self) -> String {
        selected_text(self.last_content())
    }

    pub fn sync(&mut self) -> &RenderableContent {
        let term = self.term.clone();
        let mut terminal = term.lock();
        let selectable_range = match &terminal.selection {
            Some(s) => s.to_range(&terminal),
            None => None,
        };

        let cursor = terminal.grid_mut().cursor_cell().clone();
        self.last_content.grid = terminal.grid().clone();
        self.last_content.selectable_range = selectable_range;
        self.last_content.cursor = cursor.clone();
        self.last_content.terminal_mode = *terminal.mode();
        self.last_content.terminal_size = self.size;
        self.last_content()
    }

    pub fn last_content(&self) -> &RenderableContent {
        &self.last_content
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn pty_id(&self) -> u32 {
        self.pty_id
    }

    fn process_link_action(
        &mut self,
        terminal: &Term<EventProxy>,
        link_action: LinkAction,
        point: Point,
    ) {
        match link_action {
            LinkAction::Hover => {
                self.last_content.hovered_hyperlink =
                    self.regex_match_at(terminal, point, &mut self.url_regex.clone());
            }
            LinkAction::Clear => {
                self.last_content.hovered_hyperlink = None;
            }
            LinkAction::Open => {
                self.open_link();
            }
        };
    }

    fn open_link(&self) {
        if let Some(range) = &self.last_content.hovered_hyperlink {
            let start = range.start();
            let end = range.end();

            let mut url = String::from(self.last_content.grid.index(*start).c);
            for indexed in self.last_content.grid.iter_from(*start) {
                url.push(indexed.c);
                if indexed.point == *end {
                    break;
                }
            }

            open::that(url).unwrap_or_else(|_| {
                panic!("link opening is failed");
            })
        }
    }

    fn process_mouse_report(
        &self,
        button: MouseButton,
        modifiers: Modifiers,
        point: Point,
        pressed: bool,
    ) {
        let mut mods = 0;
        if modifiers.contains(Modifiers::SHIFT) {
            mods += 4;
        }
        if modifiers.contains(Modifiers::ALT) {
            mods += 8;
        }
        if modifiers.contains(Modifiers::COMMAND) {
            mods += 16;
        }

        match MouseMode::from(self.last_content().terminal_mode) {
            MouseMode::Sgr => self.sgr_mouse_report(point, button as u8 + mods, pressed),
            MouseMode::Normal(is_utf8) => {
                if pressed {
                    self.normal_mouse_report(point, button as u8 + mods, is_utf8)
                } else {
                    self.normal_mouse_report(point, 3 + mods, is_utf8)
                }
            }
        }
    }

    fn sgr_mouse_report(&self, point: Point, button: u8, pressed: bool) {
        let c = if pressed { 'M' } else { 'm' };

        let msg = format!(
            "\x1b[<{};{};{}{}",
            button,
            point.column + 1,
            point.line + 1,
            c
        );

        self.notifier.notify(msg.as_bytes().to_vec());
    }

    fn normal_mouse_report(&self, point: Point, button: u8, is_utf8: bool) {
        let Point { line, column } = point;
        let max_point = if is_utf8 { 2015 } else { 223 };

        if line >= max_point || column >= max_point {
            return;
        }

        let mut msg = vec![b'\x1b', b'[', b'M', 32 + button];

        let mouse_pos_encode = |pos: usize| -> Vec<u8> {
            let pos = 32 + 1 + pos;
            let first = 0xC0 + pos / 64;
            let second = 0x80 + (pos & 63);
            vec![first as u8, second as u8]
        };

        if is_utf8 && column >= Column(95) {
            msg.append(&mut mouse_pos_encode(column.0));
        } else {
            msg.push(32 + 1 + column.0 as u8);
        }

        if is_utf8 && line >= 95 {
            msg.append(&mut mouse_pos_encode(line.0 as usize));
        } else {
            msg.push(32 + 1 + line.0 as u8);
        }

        self.notifier.notify(msg);
    }

    fn start_selection(
        &mut self,
        terminal: &mut Term<EventProxy>,
        selection_type: SelectionType,
        x: f32,
        y: f32,
    ) {
        let location = Self::selection_point(x, y, &self.size, terminal.grid().display_offset());
        terminal.selection = Some(Selection::new(
            selection_type,
            location,
            self.selection_side(x),
        ));
    }

    fn update_selection(&mut self, terminal: &mut Term<EventProxy>, x: f32, y: f32) {
        let display_offset = terminal.grid().display_offset();
        if let Some(ref mut selection) = terminal.selection {
            let location = Self::selection_point(x, y, &self.size, display_offset);
            selection.update(location, self.selection_side(x));
        }
    }

    fn selection_side(&self, x: f32) -> Side {
        let cell_x = x as usize % self.size.cell_width as usize;
        let half_cell_width = (self.size.cell_width as f32 / 2.0) as usize;

        if cell_x > half_cell_width {
            Side::Right
        } else {
            Side::Left
        }
    }

    fn resize(&mut self, terminal: &mut Term<EventProxy>, layout_size: Size, font_size: Size) {
        if layout_size == self.size.layout_size
            && font_size.width as u16 == self.size.cell_width
            && font_size.height as u16 == self.size.cell_height
        {
            return;
        }

        let lines = (layout_size.height / font_size.height.floor()) as u16;
        let cols = (layout_size.width / font_size.width.floor()) as u16;
        if lines > 0 && cols > 0 {
            self.size = TerminalSize {
                layout_size,
                cell_height: font_size.height as u16,
                cell_width: font_size.width as u16,
                num_lines: lines,
                num_cols: cols,
            };

            self.notifier.on_resize(self.size.into());
            terminal.resize(TermSize::new(
                self.size.num_cols as usize,
                self.size.num_lines as usize,
            ));
        }
    }

    fn write<I: Into<Cow<'static, [u8]>>>(&self, input: I) {
        self.notifier.notify(input);
    }

    fn scroll(&mut self, terminal: &mut Term<EventProxy>, delta_value: i32) {
        if delta_value != 0 {
            let scroll = Scroll::Delta(delta_value);
            if terminal
                .mode()
                .contains(TermMode::ALTERNATE_SCROLL | TermMode::ALT_SCREEN)
            {
                let line_cmd = if delta_value > 0 { b'A' } else { b'B' };
                let mut content = vec![];

                for _ in 0..delta_value.abs() {
                    content.push(0x1b);
                    content.push(b'O');
                    content.push(line_cmd);
                }

                self.notifier.notify(content);
            } else {
                terminal.grid_mut().scroll_display(scroll);
            }
        }
    }

    /// Based on alacritty/src/display/hint.rs > regex_match_at
    /// Retrieve the match, if the specified point is inside the content matching the regex.
    fn regex_match_at(
        &self,
        terminal: &Term<EventProxy>,
        point: Point,
        regex: &mut RegexSearch,
    ) -> Option<Match> {
        let x = visible_regex_match_iter(terminal, regex).find(|rm| rm.contains(&point));
        x
    }
}

/// Copied from alacritty/src/display/hint.rs:
/// Iterate over all visible regex matches.
fn visible_regex_match_iter<'a>(
    term: &'a Term<EventProxy>,
    regex: &'a mut RegexSearch,
) -> impl Iterator<Item = Match> + 'a {
    let viewport_start = Line(-(term.grid().display_offset() as i32));
    let viewport_end = viewport_start + term.bottommost_line();
    let mut start = term.line_search_left(Point::new(viewport_start, Column(0)));
    let mut end = term.line_search_right(Point::new(viewport_end, Column(0)));
    start.line = start.line.max(viewport_start - 100);
    end.line = end.line.min(viewport_end + 100);

    RegexIter::new(start, end, Direction::Right, term, regex)
        .skip_while(move |rm| rm.end().line < viewport_start)
        .take_while(move |rm| rm.start().line <= viewport_end)
}

pub struct RenderableContent {
    pub grid: Grid<Cell>,
    pub hovered_hyperlink: Option<RangeInclusive<Point>>,
    pub selectable_range: Option<SelectionRange>,
    pub cursor: Cell,
    pub terminal_mode: TermMode,
    pub terminal_size: TerminalSize,
}

impl Default for RenderableContent {
    fn default() -> Self {
        Self {
            grid: Grid::new(0, 0, 0),
            hovered_hyperlink: None,
            selectable_range: None,
            cursor: Cell::default(),
            terminal_mode: TermMode::empty(),
            terminal_size: TerminalSize::default(),
        }
    }
}

impl Drop for TerminalBackend {
    fn drop(&mut self) {
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}

#[derive(Clone)]
pub struct EventProxy(mpsc::Sender<Event>);

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let _ = self.0.send(event.clone());
    }
}

fn selected_text(content: &RenderableContent) -> String {
    let Some(range) = content.selectable_range else {
        return String::new();
    };
    let mut output = String::new();
    for line in range.start.line.0..=range.end.line.0 {
        let mut row = String::new();
        for col in 0..content.grid.columns() {
            let point = Point::new(Line(line), Column(col));
            if !range.contains(point) {
                continue;
            }
            let cell = &content.grid[point];
            if cell.flags.intersects(
                alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER
                    | alacritty_terminal::term::cell::Flags::LEADING_WIDE_CHAR_SPACER,
            ) {
                continue;
            }
            row.push(cell.c);
            if let Some(extra) = cell.zerowidth() {
                row.extend(extra);
            }
        }
        output.push_str(row.trim_end());
        if line < range.end.line.0
            && !content.grid[Point::new(Line(line), content.grid.last_column())]
                .flags
                .contains(alacritty_terminal::term::cell::Flags::WRAPLINE)
        {
            output.push('\n');
        }
    }
    output
}
fn target_at_content(content: &RenderableContent, x: f32, y: f32) -> Option<LinkTarget> {
    use alacritty_terminal::term::cell::Flags;
    let size = &content.terminal_size;
    if size.cell_width == 0 || size.cell_height == 0 || x < 0.0 || y < 0.0 {
        return None;
    }
    let grid = &content.grid;
    let point = TerminalBackend::selection_point(x, y, size, grid.display_offset());
    let mut first = point.line;
    let mut last = point.line;
    let column = grid.last_column();
    while first > grid.topmost_line()
        && point.line.0 - first.0 < 32
        && grid[Point::new(first - 1, column)]
            .flags
            .contains(Flags::WRAPLINE)
    {
        first -= 1;
    }
    while last < grid.bottommost_line()
        && last.0 - first.0 < 32
        && grid[Point::new(last, column)]
            .flags
            .contains(Flags::WRAPLINE)
    {
        last += 1;
    }
    let mut chars = Vec::new();
    let mut points = Vec::new();
    let mut hit = None;
    for line in first.0..=last.0 {
        for col in 0..=column.0 {
            let p = Point::new(Line(line), Column(col));
            let cell = &grid[p];
            if p == point {
                hit = Some(if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    chars.len().saturating_sub(1)
                } else {
                    chars.len()
                });
            }
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            chars.push(cell.c);
            points.push(p);
            if let Some(extra) = cell.zerowidth() {
                for c in extra {
                    chars.push(*c);
                    points.push(p);
                }
            }
        }
    }
    let range = token_range(&chars, hit?)?;
    let mut rects = Vec::<egui::Rect>::new();
    for p in &points[range.clone()] {
        let row = p.line.0 + grid.display_offset() as i32;
        if row < 0 || row >= size.num_lines as i32 {
            continue;
        }
        let width = if grid[*p].flags.contains(Flags::WIDE_CHAR) {
            2.0
        } else {
            1.0
        };
        let rect = egui::Rect::from_min_size(
            egui::pos2(
                p.column.0 as f32 * size.cell_width as f32,
                row as f32 * size.cell_height as f32,
            ),
            egui::vec2(width * size.cell_width as f32, size.cell_height as f32),
        );
        if rects
            .last()
            .is_some_and(|previous| previous.top() == rect.top())
        {
            let previous = rects.last_mut().unwrap();
            *previous = previous.union(rect);
        } else {
            rects.push(rect);
        }
    }
    Some(LinkTarget {
        text: chars[range].iter().collect(),
        rects,
    })
}
/// Renderer-neutral target text with local-coordinate underline rectangles.
#[derive(Clone, Debug)]
pub struct LinkTarget {
    pub text: String,
    pub rects: Vec<egui::Rect>,
}
fn token_range(chars: &[char], hit: usize) -> Option<std::ops::Range<usize>> {
    let mut start = 0;
    while start < chars.len() {
        if chars[start].is_whitespace() {
            start += 1;
            continue;
        }
        let quote_start = (start..chars.len())
            .find(|i| !matches!(chars[*i], '(' | '[' | '{' | '<'))
            .unwrap_or(start);
        let quote = matches!(chars[quote_start], '\'' | '"' | '`').then_some(chars[quote_start]);
        let mut end = if quote.is_some() {
            quote_start + 1
        } else {
            start + 1
        };
        while end < chars.len()
            && if let Some(q) = quote {
                chars[end] != q
            } else {
                !chars[end].is_whitespace()
            }
        {
            end += 1;
        }
        if quote.is_some() && end < chars.len() {
            end += 1;
            while end < chars.len() && (chars[end].is_ascii_digit() || chars[end] == ':') {
                end += 1;
            }
        }
        if (start..end).contains(&hit) {
            let mut left = start;
            let mut right = end;
            while left < right && matches!(chars[left], '(' | '[' | '{' | '<' | '\'' | '"' | '`') {
                left += 1;
            }
            while right > left {
                let last = chars[right - 1];
                let matching = match last {
                    ')' => Some('('),
                    ']' => Some('['),
                    '}' => Some('{'),
                    '>' => Some('<'),
                    _ => None,
                };
                let trim = if let Some(open) = matching {
                    chars[left..right].iter().filter(|c| **c == last).count()
                        > chars[left..right].iter().filter(|c| **c == open).count()
                } else {
                    matches!(last, ',' | ';' | '.' | ':' | '\'' | '"' | '`')
                };
                if !trim {
                    break;
                }
                right -= 1;
            }
            return (left < right && (left..right).contains(&hit)).then_some(left..right);
        }
        start = end;
    }
    None
}
#[cfg(test)]
mod target_tests {
    use super::*;
    #[test]
    fn new_output_keeps_retained_history_anchored_until_scrolling_down() {
        let mut grid = Grid::<Cell>::new(3, 10, 100);
        grid.scroll_up(&(Line(0)..Line(3)), 8);
        grid.scroll_display(Scroll::Delta(3));
        let offset = grid.display_offset();
        assert_eq!(offset, 3);
        grid.scroll_up(&(Line(0)..Line(3)), 1);
        assert_eq!(grid.display_offset(), offset + 1);
        grid.scroll_display(Scroll::Delta(-100));
        assert_eq!(grid.display_offset(), 0);
    }

    #[test]
    fn copying_selection_preserves_newlines_and_offscreen_history() {
        let mut content = RenderableContent {
            grid: Grid::new(2, 6, 2),
            ..Default::default()
        };
        for (line, text) in [(0, "first"), (1, "second")] {
            for (col, c) in text.chars().enumerate() {
                content.grid[Point::new(Line(line), Column(col))].c = c;
            }
        }
        content.grid.scroll_up(&(Line(0)..Line(2)), 1);
        content.selectable_range = Some(SelectionRange::new(
            Point::new(Line(-1), Column(0)),
            Point::new(Line(0), Column(5)),
            false,
        ));
        assert_eq!(selected_text(&content), "first\nsecond");
    }
    #[test]
    fn wrapped_and_scrolled_wide_character_hit_testing() {
        use alacritty_terminal::term::cell::Flags;
        let mut content = RenderableContent {
            grid: Grid::new(3, 10, 10),
            terminal_size: TerminalSize {
                cell_width: 10,
                cell_height: 20,
                num_cols: 10,
                num_lines: 3,
                ..Default::default()
            },
            ..Default::default()
        };
        for (i, c) in "src/long_f".chars().enumerate() {
            content.grid[Point::new(Line(0), Column(i))].c = c;
        }
        content.grid[Point::new(Line(0), Column(9))]
            .flags
            .insert(Flags::WRAPLINE);
        for (i, c) in "ile.rs:12".chars().enumerate() {
            content.grid[Point::new(Line(1), Column(i))].c = c;
        }
        let target = target_at_content(&content, 25.0, 25.0).unwrap();
        assert_eq!(target.text, "src/long_file.rs:12");
        assert_eq!(target.rects.len(), 2);
        content.grid[Point::new(Line(0), Column(0))].c = '界';
        content.grid[Point::new(Line(0), Column(0))]
            .flags
            .insert(Flags::WIDE_CHAR);
        content.grid[Point::new(Line(0), Column(1))]
            .flags
            .insert(Flags::WIDE_CHAR_SPACER);
        let target = target_at_content(&content, 15.0, 5.0).unwrap();
        assert!(target.text.starts_with("界c/"));
        content.grid.scroll_up(&(Line(0)..Line(3)), 1);
        content.grid.scroll_display(Scroll::Delta(1));
        let target = target_at_content(&content, 25.0, 25.0).unwrap();
        assert!(target.text.ends_with("ile.rs:12"));
    }
    #[test]
    fn quoted_spaces_and_punctuation() {
        let chars: Vec<_> = r#"see "src/my file.rs:12:3", next"#.chars().collect();
        let range = token_range(&chars, 12).unwrap();
        assert_eq!(
            chars[range].iter().collect::<String>(),
            "src/my file.rs:12:3"
        );
        let chars: Vec<_> = "(https://example.com/a),".chars().collect();
        let range = token_range(&chars, 8).unwrap();
        assert_eq!(
            chars[range].iter().collect::<String>(),
            "https://example.com/a"
        );
        for (input, expected) in [
            (r#"("src/my file.rs:12:3")"#, "src/my file.rs:12:3"),
            ("src/main.rs:12:3:", "src/main.rs:12:3"),
            (
                "https://example.com/Thing_(language).",
                "https://example.com/Thing_(language)",
            ),
        ] {
            let chars: Vec<_> = input.chars().collect();
            let range = token_range(&chars, 8).unwrap();
            assert_eq!(chars[range].iter().collect::<String>(), expected);
        }
    }
}
