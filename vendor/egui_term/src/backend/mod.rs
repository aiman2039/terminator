pub mod settings;

use crate::types::Size;
use alacritty_terminal::event::{Event, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point, Side};
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// Upper bound on PTY-driven repaints: a sustained burst stays near 30 fps. An
/// echo arriving after an idle period repaints immediately (leading edge), so
/// input handling itself remains independent of the output repaint budget.
const REPAINT_FRAME: Duration = Duration::from_millis(33);

/// Shared across panes in one GUI context. Budget from actual paints, not from
/// each PTY's independent event stream; a delayed wake must not be followed by
/// a second leading-edge wake immediately after that frame.
#[derive(Default)]
struct RepaintBudget {
    last_paint: Option<Instant>,
    pending: Option<Instant>,
}

impl RepaintBudget {
    fn painted(&mut self, now: Instant) {
        self.last_paint = Some(now);
        self.pending = None;
    }

    fn request(&mut self, now: Instant) -> Option<Duration> {
        let deadline = self
            .last_paint
            .map_or(now, |last| (last + REPAINT_FRAME).max(now));
        if self.pending.is_some_and(|pending| pending <= deadline) {
            return None;
        }
        self.pending = Some(deadline);
        Some(deadline.saturating_duration_since(now))
    }
}

fn repaint_budget(ctx: &egui::Context) -> Arc<Mutex<RepaintBudget>> {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Arc<Mutex<RepaintBudget>>>(egui::Id::new(
            "egui_term::repaint_budget",
        ))
        .clone()
    })
}

pub type TerminalMode = TermMode;
pub type PtyEvent = Event;
pub type SelectionType = AlacrittySelectionType;

#[derive(Debug, Clone)]
pub enum BackendCommand {
    Write(Vec<u8>),
    Scroll(i32),
    /// Scroll retained history without translating wheel movement into input.
    /// `extend_selection` grows the active drag by the display-offset change,
    /// which is zero when the viewport cannot move.
    ScrollLocal {
        lines: i32,
        extend_selection: bool,
    },
    Resize(Size, Size),
    SelectStart(SelectionType, f32, f32),
    SelectUpdate(f32, f32),
    /// Grow the active drag by this many lines. Positive grows into history.
    SelectExtend(i32),
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

impl TerminalSize {
    pub fn from_layout(layout_size: Size, font_size: Size) -> Self {
        let height = font_size.height.floor().max(1.0);
        let width = font_size.width.floor().max(1.0);
        Self {
            layout_size,
            cell_height: font_size.height as u16,
            cell_width: font_size.width as u16,
            num_lines: (layout_size.height / height) as u16,
            num_cols: (layout_size.width / width) as u16,
        }
    }

    pub fn from_pane(pane: egui::Vec2, font_size: Size) -> Self {
        Self::from_layout(Size::from(pane), font_size)
    }

    pub fn is_empty(&self) -> bool {
        self.num_cols == 0 || self.num_lines == 0
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

fn pty_size(settings: &BackendSettings) -> TerminalSize {
    if settings.size.is_empty() {
        TerminalSize::default()
    } else {
        settings.size
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
    grid_dirty: Arc<AtomicBool>,
    painted: Arc<AtomicBool>,
    repaint_budget: Arc<Mutex<RepaintBudget>>,
    /// While a selection drag is active, an application clear must not drop it.
    selection_hold: bool,
    /// Grid point where the current drag started. Survives an application clear.
    drag_anchor: Option<Point>,
}

impl TerminalBackend {
    pub fn new(
        id: u64,
        app_context: egui::Context,
        pty_event_proxy_sender: Sender<(u64, PtyEvent)>,
        settings: BackendSettings,
    ) -> Result<Self> {
        let terminal_size = pty_size(&settings);
        let pty_config = tty::Options {
            shell: Some(tty::Shell::new(settings.shell, settings.args)),
            working_directory: settings.working_directory,
            ..tty::Options::default()
        };
        let config = term::Config::default();
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
            grid: snapshot_viewport(term.grid()),
            display_offset: term.grid().display_offset(),
            selected_text: String::new(),
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
        let grid_dirty = Arc::new(AtomicBool::new(true));
        let grid_dirty_for_events = grid_dirty.clone();
        let painted = Arc::new(AtomicBool::new(false));
        let painted_for_events = painted.clone();
        let repaint_budget = repaint_budget(&app_context);
        let budget_for_events = repaint_budget.clone();
        let _pty_event_loop_thread = pty_event_loop.spawn();
        let _pty_event_subscription = std::thread::Builder::new()
            .name(format!("pty_event_subscription_{}", id))
            .spawn(move || {
                loop {
                    if let Ok(event) = event_receiver.recv() {
                        grid_dirty_for_events.store(true, Ordering::Relaxed);
                        if pty_event_proxy_sender.send((id, event.clone())).is_err() {
                            break;
                        }
                        // Hidden terminals update their grid without waking the GUI.
                        // All visible panes share a budget based on actual paints.
                        if painted_for_events.load(Ordering::Relaxed) {
                            let delay = budget_for_events
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .request(Instant::now());
                            if let Some(delay) = delay {
                                app_context.request_repaint_after(delay);
                            }
                        }
                        match event {
                            Event::Exit => break,
                            // The owning daemon answers terminal queries, including when detached.
                            Event::PtyWrite(_) => {}
                            _ => {}
                        }
                    } else {
                        break;
                    }
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
            grid_dirty,
            painted,
            repaint_budget,
            selection_hold: false,
            drag_anchor: None,
        })
    }

    /// Keep the in-progress drag if the application clears the screen.
    pub fn set_selection_hold(&mut self, hold: bool) {
        if !hold {
            self.drag_anchor = None;
        }
        self.selection_hold = hold;
    }

    pub fn process_command(&mut self, cmd: BackendCommand) {
        if let BackendCommand::Resize(layout_size, font_size) = &cmd {
            if self.resize_is_noop(*layout_size, *font_size) {
                return;
            }
        }
        if Self::command_dirties_grid(&cmd) {
            self.grid_dirty.store(true, Ordering::Relaxed);
        }
        let term = self.term.clone();
        let mut term = term.lock();
        match cmd {
            BackendCommand::Write(input) => {
                self.write(input);
                term.scroll_display(Scroll::Bottom);
            }
            BackendCommand::ScrollLocal {
                lines,
                extend_selection,
            } => {
                let applied = scroll_display_lines(&mut term, lines);
                if extend_selection {
                    let previous = self.last_content.selectable_range;
                    restore_then_extend(&mut term, &mut self.drag_anchor, previous, applied);
                    self.capture_selection(&term);
                }
            }
            BackendCommand::Scroll(delta) => {
                self.scroll(&mut term, delta);
            }
            BackendCommand::Resize(layout_size, font_size) => {
                self.resize(&mut term, layout_size, font_size);
            }
            BackendCommand::SelectStart(selection_type, x, y) => {
                self.start_selection(&mut term, selection_type, x, y);
                self.capture_selection(&term);
            }
            BackendCommand::SelectUpdate(x, y) => {
                self.update_selection(&mut term, x, y);
                self.capture_selection(&term);
            }
            BackendCommand::SelectExtend(delta) => {
                // The application may have cleared the drag. Extending first
                // grows a point at the press and `capture_selection` replaces
                // the cached range with that.
                if self.selection_hold {
                    let previous = self.last_content.selectable_range;
                    restore_then_extend(&mut term, &mut self.drag_anchor, previous, delta);
                } else {
                    extend_drag_selection(&mut term, &mut self.drag_anchor, delta);
                }
                self.capture_selection(&term);
            }
            BackendCommand::ProcessLink(link_action, point) => {
                self.process_link_action(&term, link_action, point);
            }
            BackendCommand::MouseReport(button, modifiers, point, pressed) => {
                self.process_mouse_report(button, modifiers, point, pressed);
            }
        };
    }

    fn resize_is_noop(&self, layout_size: Size, font_size: Size) -> bool {
        layout_size == self.size.layout_size
            && font_size.width as u16 == self.size.cell_width
            && font_size.height as u16 == self.size.cell_height
    }

    fn command_dirties_grid(cmd: &BackendCommand) -> bool {
        matches!(
            cmd,
            BackendCommand::Write(_)
                | BackendCommand::Scroll(_)
                | BackendCommand::ScrollLocal { .. }
                | BackendCommand::Resize(_, _)
        )
    }

    fn capture_selection(&mut self, terminal: &Term<EventProxy>) {
        self.last_content.selectable_range = terminal
            .selection
            .as_ref()
            .and_then(|selection| selection.to_range(terminal));
        self.last_content.selected_text = terminal.selection_to_string().unwrap_or_default();
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
        let term = self.term.clone();
        let mut term = term.lock();
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
        self.capture_selection(&term);
    }
    pub fn selectable_content(&self) -> String {
        self.last_content.selected_text.clone()
    }

    /// One grid row as searchable text. `columns[c]` is the grid column of
    /// the `c`-th char in `text`, so literal matches map back to cells.
    /// Covers the live grid including retained scrollback; wrapped logical
    /// lines stay split across rows (matches cannot span rows in v1).
    pub fn search_rows(&self) -> Vec<SearchRow> {
        use alacritty_terminal::term::cell::Flags;
        let term = self.term.lock();
        let grid = term.grid();
        let mut rows = Vec::new();
        for line in grid.topmost_line().0..=grid.bottommost_line().0 {
            let line = Line(line);
            let mut text = String::new();
            let mut columns = Vec::new();
            for col in 0..grid.columns() {
                let cell = &grid[line][Column(col)];
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                text.push(cell.c);
                columns.push(col);
                if let Some(extra) = cell.zerowidth() {
                    for c in extra {
                        text.push(*c);
                        columns.push(col);
                    }
                }
            }
            let trimmed = text.trim_end().to_string();
            columns.truncate(trimmed.chars().count());
            rows.push(SearchRow {
                line,
                text: trimmed,
                columns,
            });
        }
        rows
    }

    /// Literal search over the live grid plus scrollback. Never sends input.
    pub fn find(&self, query: &str, case_insensitive: bool) -> FindOutcome {
        let rows = self.search_rows();
        let refs: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
        let mut matches = Vec::new();
        for hit in crate::find::find_in_rows(&refs, query, case_insensitive) {
            let row = &rows[hit.row];
            let (Some(&start), Some(&end)) = (
                row.columns.get(hit.chars.start),
                row.columns.get(hit.chars.end.saturating_sub(1)),
            ) else {
                continue;
            };
            matches.push(FoundMatch {
                line: row.line.0,
                start_col: start,
                end_col: end,
            });
        }
        let truncated = matches.len() >= crate::find::MAX_MATCHES;
        FindOutcome { matches, truncated }
    }

    /// Scroll the viewport so grid `line` is visible, with a two-line margin.
    /// Local-only: unlike `scroll()`, this never writes to the PTY, so
    /// full-screen applications are unaffected. No-op in alt-screen mode.
    pub fn reveal_grid_line(&mut self, line: i32) {
        let visible = self.size.num_lines.max(1) as i32;
        let mut term = self.term.lock();
        let current = term.grid().display_offset() as i32;
        let top = -current;
        if line >= top && line < top + visible {
            return;
        }
        let topmost = term.grid().topmost_line().0;
        let max_offset = -topmost;
        let desired = (-(line - 2).max(topmost)).clamp(0, max_offset);
        term.scroll_display(Scroll::Delta(desired - current));
    }

    pub fn sync(&mut self) -> &RenderableContent {
        if !self.grid_dirty.swap(false, Ordering::Relaxed) {
            return self.last_content();
        }
        let terminal = self.term.clone();
        let mut terminal = terminal.lock();
        let previous_range = self.last_content.selectable_range;
        // Output scrolls selection endpoints and leaves `drag_anchor` behind.
        // Reattach it to the selection's fixed end before a later extend or revive.
        refresh_drag_anchor(&terminal, &mut self.drag_anchor);
        if self.selection_hold && terminal.selection.is_none() {
            revive_selection(&mut terminal, self.drag_anchor, previous_range);
        }
        let selectable_range = match &terminal.selection {
            Some(s) => s.to_range(&terminal),
            None => None,
        };

        let cursor = terminal.grid_mut().cursor_cell().clone();
        self.last_content.grid = snapshot_viewport(terminal.grid());
        self.last_content.display_offset = terminal.grid().display_offset();
        self.last_content.selectable_range = selectable_range;
        self.last_content.selected_text = terminal.selection_to_string().unwrap_or_default();
        self.last_content.cursor = cursor;
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

    /// Mark whether this terminal was painted in the current frame. Hidden
    /// terminals still record grid changes but do not wake the UI.
    pub fn set_painted(&self, painted: bool) {
        self.painted.store(painted, Ordering::Relaxed);
        if painted {
            self.repaint_budget
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .painted(Instant::now());
        }
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
        let Some(range) = &self.last_content.hovered_hyperlink else {
            return;
        };
        let start = *range.start();
        let end = *range.end();
        let term = self.term.clone();
        let term = term.lock();
        let grid = term.grid();
        let mut url = String::from(grid.index(start).c);
        for indexed in grid.iter_from(start) {
            url.push(indexed.c);
            if indexed.point == end {
                break;
            }
        }

        open::that(url).unwrap_or_else(|_| {
            panic!("link opening is failed");
        })
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
        self.drag_anchor = Some(location);
        terminal.selection = Some(Selection::new(
            selection_type,
            location,
            self.selection_side(x),
        ));
    }

    fn update_selection(&mut self, terminal: &mut Term<EventProxy>, x: f32, y: f32) {
        refresh_drag_anchor(terminal, &mut self.drag_anchor);
        if self.selection_hold && terminal.selection.is_none() {
            revive_selection(
                terminal,
                self.drag_anchor,
                self.last_content.selectable_range,
            );
        }
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

        let size = TerminalSize::from_layout(layout_size, font_size);
        if size.is_empty() {
            return;
        }
        self.size = size;
        self.notifier.on_resize(self.size.into());
        terminal.resize(TermSize::new(
            self.size.num_cols as usize,
            self.size.num_lines as usize,
        ));
    }

    fn write<I: Into<Cow<'static, [u8]>>>(&self, input: I) {
        self.notifier.notify(input);
    }

    fn scroll(&mut self, terminal: &mut Term<EventProxy>, delta_value: i32) {
        if delta_value == 0 {
            return;
        }
        if terminal
            .mode()
            .contains(TermMode::ALTERNATE_SCROLL | TermMode::ALT_SCREEN)
        {
            self.notifier
                .notify(scroll_key_bytes(delta_value, terminal.mode()));
        } else {
            terminal
                .grid_mut()
                .scroll_display(Scroll::Delta(delta_value));
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

/// Lines the viewport actually moved. Positive reaches into history.
fn scroll_display_lines<T: EventListener>(terminal: &mut Term<T>, lines: i32) -> i32 {
    let before = terminal.grid().display_offset() as i32;
    terminal.scroll_display(Scroll::Delta(lines));
    terminal.grid().display_offset() as i32 - before
}

/// Scroll local history and grow the drag by the offset that changed.
/// Test-only helper: the live drag path inlines this via `scroll_display_lines`.
#[cfg(test)]
fn scroll_local_drag<T: EventListener>(
    terminal: &mut Term<T>,
    anchor: &mut Option<Point>,
    lines: i32,
) -> i32 {
    let applied = scroll_display_lines(terminal, lines);
    extend_drag_selection(terminal, anchor, applied);
    applied
}

/// Put a cleared drag back, then grow it. Extending while the selection is
/// empty replaces the cached span with the press point.
fn restore_then_extend<T>(
    terminal: &mut Term<T>,
    anchor: &mut Option<Point>,
    previous: Option<SelectionRange>,
    delta: i32,
) {
    if terminal.selection.is_none() {
        revive_selection(terminal, *anchor, previous);
    }
    extend_drag_selection(terminal, anchor, delta);
}

/// Grow a drag by `delta` lines. Positive reaches into history.
///
/// Following the stationary pointer instead drops the line that enters at the
/// far edge of the viewport: that line is outside the anchor-to-pointer span.
/// The edge opposite the scroll moves, and the press point stays the anchor
/// unless it is the edge that has to move.
fn extend_drag_selection<T>(terminal: &mut Term<T>, anchor: &mut Option<Point>, delta: i32) {
    if delta == 0 {
        return;
    }
    let Some(anchor_point) = *anchor else {
        return;
    };
    let ty = terminal
        .selection
        .as_ref()
        .map(|selection| selection.ty)
        .unwrap_or(AlacrittySelectionType::Simple);
    // `Selection::new` stores the press as the fixed end and `update` moves the
    // other end. Output scrolling rotates both coordinates and does not swap
    // them. `drag_anchor` is not rotated, so matching it against the endpoints
    // can pick the moving end and a later pointer update collapses the span.
    let selection = terminal.selection.clone();
    let columns = terminal.grid().columns();
    let (mut anchor_point, mut moving) = match selection.as_ref() {
        Some(selection) => match selection.to_range(terminal) {
            Some(range) => match anchored_ends(selection, range, columns) {
                Some((fixed, moving)) => (fixed, moving),
                None => ends_from_anchor(range, anchor_point),
            },
            None => (anchor_point, anchor_point),
        },
        None => (anchor_point, anchor_point),
    };
    let last_column = terminal.grid().last_column();
    if delta > 0 {
        if moving.line < anchor_point.line {
            moving.line = Line(moving.line.0 - delta);
            moving.column = Column(0);
        } else if anchor_point.line < moving.line {
            anchor_point.line = Line(anchor_point.line.0 - delta);
            anchor_point.column = Column(0);
        } else {
            moving.line = Line(moving.line.0 - delta);
            moving.column = Column(0);
        }
    } else {
        let down = -delta;
        if moving.line > anchor_point.line {
            moving.line = Line(moving.line.0 + down);
            moving.column = last_column;
        } else if anchor_point.line > moving.line {
            anchor_point.line = Line(anchor_point.line.0 + down);
            anchor_point.column = last_column;
        } else {
            moving.line = Line(moving.line.0 + down);
            moving.column = last_column;
        }
    }
    let anchor_point = anchor_point.grid_clamp(terminal, Boundary::Grid);
    let moving = moving.grid_clamp(terminal, Boundary::Grid);
    let mut selection = Selection::new(ty, anchor_point, Side::Left);
    selection.update(moving, Side::Right);
    selection.include_all();
    terminal.selection = Some(selection);
    *anchor = Some(anchor_point);
}

/// Put a cleared drag back. The previous range keeps both ends. The press
/// point stays the fixed end, so the next pointer move does not replace it.
/// The anchor alone restarts a drag the application erased before it covered
/// any cells.
fn revive_selection<T>(
    terminal: &mut Term<T>,
    anchor: Option<Point>,
    previous: Option<SelectionRange>,
) -> bool {
    if terminal.selection.is_some() {
        return false;
    }
    if let Some(range) = previous {
        reinstall_selection(terminal, range, anchor);
        return true;
    }
    if let Some(anchor) = anchor {
        terminal.selection = Some(Selection::new(
            AlacrittySelectionType::Simple,
            anchor,
            Side::Left,
        ));
        return true;
    }
    false
}

fn reinstall_selection<T>(terminal: &mut Term<T>, range: SelectionRange, anchor: Option<Point>) {
    let ty = if range.is_block {
        AlacrittySelectionType::Block
    } else {
        AlacrittySelectionType::Simple
    };
    // `Selection::update` replaces `region.end`. `SelectionRange` is sorted
    // top-left to bottom-right, so an upward drag's press point is `range.end`.
    // Anchoring at `range.start` makes the next move drop that bottom endpoint.
    let (fixed, moving) = match anchor {
        Some(anchor) if point_closer(range.end, range.start, anchor) => (range.end, range.start),
        _ => (range.start, range.end),
    };
    let mut selection = Selection::new(ty, fixed, Side::Left);
    selection.update(moving, Side::Right);
    selection.include_all();
    terminal.selection = Some(selection);
}

fn ends_from_anchor(range: SelectionRange, anchor: Point) -> (Point, Point) {
    if point_closer(range.end, range.start, anchor) {
        (range.end, range.start)
    } else {
        (range.start, range.end)
    }
}

/// The press point is the selection's fixed end. Recover it from the live
/// selection instead of from `drag_anchor`, which output scrolling does not move.
fn refresh_drag_anchor<T>(terminal: &Term<T>, anchor: &mut Option<Point>) {
    if anchor.is_none() {
        return;
    }
    let Some(selection) = terminal.selection.as_ref() else {
        return;
    };
    let Some(range) = selection.to_range(terminal) else {
        return;
    };
    if let Some((fixed, _)) = anchored_ends(selection, range, terminal.grid().columns()) {
        *anchor = Some(fixed);
    }
}

fn anchored_ends(
    selection: &Selection,
    range: SelectionRange,
    columns: usize,
) -> Option<(Point, Point)> {
    let starts = point_candidates(range.start, columns);
    let ends = point_candidates(range.end, columns);
    for fixed in starts {
        let Some(fixed) = fixed else {
            continue;
        };
        for moving in ends {
            let Some(moving) = moving else {
                continue;
            };
            if selection_matches(selection, fixed, moving) {
                return Some((fixed, moving));
            }
        }
    }
    for fixed in ends {
        let Some(fixed) = fixed else {
            continue;
        };
        for moving in starts {
            let Some(moving) = moving else {
                continue;
            };
            if fixed == moving {
                continue;
            }
            if selection_matches(selection, fixed, moving) {
                return Some((fixed, moving));
            }
        }
    }
    None
}

fn selection_matches(selection: &Selection, fixed: Point, moving: Point) -> bool {
    for fixed_side in [Side::Left, Side::Right] {
        for moving_side in [Side::Left, Side::Right] {
            let mut candidate = Selection::new(selection.ty, fixed, fixed_side);
            candidate.update(moving, moving_side);
            if &candidate == selection {
                return true;
            }
        }
    }
    false
}

/// `to_range` can shift an endpoint by one cell. The selection still stores
/// the pre-adjustment point.
fn point_candidates(point: Point, columns: usize) -> [Option<Point>; 5] {
    let mut out = [None; 5];
    out[0] = Some(point);
    let mut count = 1;
    if point.column.0 > 0 {
        out[count] = Some(Point::new(point.line, Column(point.column.0 - 1)));
        count += 1;
    }
    if columns > 0 && point.column.0 + 1 < columns {
        out[count] = Some(Point::new(point.line, Column(point.column.0 + 1)));
        count += 1;
    }
    if columns > 0 {
        out[count] = Some(Point::new(Line(point.line.0 - 1), Column(columns - 1)));
        count += 1;
        if count < out.len() {
            out[count] = Some(Point::new(Line(point.line.0 + 1), Column(0)));
        }
    }
    out
}

/// `candidate` is nearer the press point than `other`. Line wins over column.
fn point_closer(candidate: Point, other: Point, anchor: Point) -> bool {
    let candidate_line = (candidate.line.0 - anchor.line.0).abs();
    let other_line = (other.line.0 - anchor.line.0).abs();
    if candidate_line != other_line {
        return candidate_line < other_line;
    }
    let candidate_col = (candidate.column.0 as i32 - anchor.column.0 as i32).abs();
    let other_col = (other.column.0 as i32 - anchor.column.0 as i32).abs();
    candidate_col < other_col
}

/// Wheel-generated cursor keys for alternate-scroll mode. Honors DECCKM
/// application-cursor state exactly like the keyboard arrow bindings: CSI
/// (`ESC [ A/B`) normally, SS3 (`ESC O A/B`) with `APP_CURSOR` set.
fn scroll_key_bytes(delta_value: i32, mode: &TermMode) -> Vec<u8> {
    let line_cmd = if delta_value > 0 { b'A' } else { b'B' };
    let middle = if mode.contains(TermMode::APP_CURSOR) {
        b'O'
    } else {
        b'['
    };
    let mut content = Vec::with_capacity(delta_value.abs() as usize * 3);
    for _ in 0..delta_value.abs() {
        content.push(0x1b);
        content.push(middle);
        content.push(line_cmd);
    }
    content
}

#[cfg(test)]
mod scroll_key_tests {
    use super::*;

    #[test]
    fn wheel_uses_csi_without_application_cursor() {
        let mode = TermMode::ALTERNATE_SCROLL | TermMode::ALT_SCREEN;
        assert_eq!(scroll_key_bytes(2, &mode), b"\x1b[A\x1b[A");
        assert_eq!(scroll_key_bytes(-1, &mode), b"\x1b[B");
    }

    #[test]
    fn wheel_uses_ss3_with_application_cursor() {
        let mode = TermMode::ALTERNATE_SCROLL | TermMode::ALT_SCREEN | TermMode::APP_CURSOR;
        assert_eq!(scroll_key_bytes(1, &mode), b"\x1bOA");
        assert_eq!(scroll_key_bytes(-3, &mode), b"\x1bOB\x1bOB\x1bOB");
    }

    #[test]
    fn zero_delta_sends_nothing() {
        assert!(scroll_key_bytes(0, &TermMode::empty()).is_empty());
        assert!(scroll_key_bytes(0, &TermMode::APP_CURSOR).is_empty());
    }
}

#[cfg(test)]
mod selection_scroll_tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::Point;
    use alacritty_terminal::vte::ansi::Handler;

    #[test]
    fn alternate_screen_local_scroll_does_not_move_the_viewport() {
        let mut term = Term::new(term::Config::default(), &TermSize::new(8, 6), VoidListener);
        term.swap_alt();
        assert!(term.mode().contains(TermMode::ALT_SCREEN));
        assert_eq!(term.grid().history_size(), 0);
        term.scroll_display(Scroll::Delta(4));
        assert_eq!(term.grid().display_offset(), 0);
        term.scroll_display(Scroll::Delta(-4));
        assert_eq!(term.grid().display_offset(), 0);
    }

    #[test]
    fn local_scroll_extends_the_selection_to_the_cell_under_the_pointer() {
        let mut term = Term::new(term::Config::default(), &TermSize::new(4, 5), VoidListener);
        term.grid_mut().scroll_up(&(Line(0)..Line(5)), 2);
        assert!(term.grid().history_size() >= 2);
        assert_eq!(term.grid().display_offset(), 0);

        let size = TerminalSize::from_layout(Size::new(32.0, 80.0), Size::new(8.0, 16.0));
        let bottom = 4.0 * 16.0;
        let start = TerminalBackend::selection_point(4.0, bottom, &size, 0);
        term.selection = Some(Selection::new(
            AlacrittySelectionType::Simple,
            start,
            Side::Right,
        ));
        term.scroll_display(Scroll::Delta(2));
        assert_eq!(term.grid().display_offset(), 2);
        let end =
            TerminalBackend::selection_point(4.0, bottom, &size, term.grid().display_offset());
        term.selection.as_mut().unwrap().update(end, Side::Right);

        let range = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        assert!(
            range.end.line.0 - range.start.line.0 >= 1,
            "selection did not grow across the local scroll"
        );
    }

    #[test]
    fn reviving_a_cleared_selection_keeps_the_dragged_text() {
        let mut term = Term::new(term::Config::default(), &TermSize::new(6, 3), VoidListener);
        for (col, c) in "hello".chars().enumerate() {
            term.grid_mut()[Line(1)][Column(col)].c = c;
        }
        let range = SelectionRange::new(
            Point::new(Line(1), Column(0)),
            Point::new(Line(1), Column(4)),
            false,
        );
        reinstall_selection(&mut term, range, None);
        assert_eq!(term.selection_to_string().as_deref(), Some("hello"));
        term.selection = None;
        assert!(revive_selection(&mut term, None, Some(range)));
        assert_eq!(term.selection_to_string().as_deref(), Some("hello"));
    }

    /// Press at the bottom, drag to the top, then the application clears the
    /// selection. The next move is still at the top and must keep the bottom.
    #[test]
    fn reviving_an_upward_drag_keeps_the_bottom_endpoint() {
        let mut term = Term::new(term::Config::default(), &TermSize::new(4, 4), VoidListener);
        put_line(&mut term, 0, "TOPP");
        put_line(&mut term, 1, "MMMM");
        put_line(&mut term, 2, "NNNN");
        put_line(&mut term, 3, "BOTT");
        let anchor = Point::new(Line(3), Column(2));
        let top = Point::new(Line(0), Column(1));
        let mut selection = Selection::new(AlacrittySelectionType::Simple, anchor, Side::Left);
        selection.update(top, Side::Right);
        selection.include_all();
        term.selection = Some(selection);
        let range = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        assert_eq!((range.start.line.0, range.end.line.0), (0, 3));
        term.selection = None;
        assert!(revive_selection(&mut term, Some(anchor), Some(range)));
        term.selection
            .as_mut()
            .unwrap()
            .update(Point::new(Line(0), Column(0)), Side::Left);
        let range = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        let selected = term.selection_to_string().unwrap_or_default();
        assert_eq!(
            (range.start.line.0, range.end.line.0),
            (0, 3),
            "restored upward drag lost an endpoint: {selected:?}"
        );
        assert!(selected.contains("MMMM") && selected.contains('B'));
    }

    #[test]
    fn reviving_a_downward_drag_keeps_the_top_endpoint() {
        let mut term = Term::new(term::Config::default(), &TermSize::new(4, 4), VoidListener);
        put_line(&mut term, 0, "TOPP");
        put_line(&mut term, 3, "BOTT");
        let anchor = Point::new(Line(0), Column(1));
        let bottom = Point::new(Line(3), Column(2));
        let mut selection = Selection::new(AlacrittySelectionType::Simple, anchor, Side::Left);
        selection.update(bottom, Side::Right);
        selection.include_all();
        term.selection = Some(selection);
        let range = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        assert_eq!((range.start.line.0, range.end.line.0), (0, 3));
        term.selection = None;
        assert!(revive_selection(&mut term, Some(anchor), Some(range)));
        term.selection.as_mut().unwrap().update(bottom, Side::Right);
        let range = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        let selected = term.selection_to_string().unwrap_or_default();
        assert_eq!(
            (range.start.line.0, range.end.line.0),
            (0, 3),
            "restored downward drag lost an endpoint: {selected:?}"
        );
    }

    /// Press on the right, drag left along the row, then scroll up. The press
    /// cell has to stay selected.
    #[test]
    fn backward_same_row_drag_keeps_the_press_cell_when_scrolling() {
        let mut term = Term::new(term::Config::default(), &TermSize::new(8, 3), VoidListener);
        put_line(&mut term, 1, "ABCDEFGH");
        let press = Point::new(Line(1), Column(6));
        let mut anchor = Some(press);
        let mut selection = Selection::new(AlacrittySelectionType::Simple, press, Side::Right);
        selection.update(Point::new(Line(1), Column(1)), Side::Left);
        term.selection = Some(selection);
        let before = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        assert_eq!(before.start.line, before.end.line);
        extend_drag_selection(&mut term, &mut anchor, 1);
        let after = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        assert!(
            after.contains(before.end),
            "backward same-row drag dropped the press cell: {after:?} was {before:?}"
        );
    }

    #[test]
    fn forward_same_row_drag_keeps_the_press_cell_when_scrolling() {
        let mut term = Term::new(term::Config::default(), &TermSize::new(8, 3), VoidListener);
        put_line(&mut term, 1, "ABCDEFGH");
        let press = Point::new(Line(1), Column(1));
        let mut anchor = Some(press);
        let mut selection = Selection::new(AlacrittySelectionType::Simple, press, Side::Left);
        selection.update(Point::new(Line(1), Column(6)), Side::Right);
        term.selection = Some(selection);
        let before = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        extend_drag_selection(&mut term, &mut anchor, 1);
        let after = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        assert!(
            after.contains(before.start),
            "forward same-row drag dropped the press cell: {after:?} was {before:?}"
        );
    }

    #[test]
    fn extend_after_clear_keeps_the_previous_selection() {
        let mut term = Term::new(term::Config::default(), &TermSize::new(4, 4), VoidListener);
        put_line(&mut term, 0, "AAAA");
        put_line(&mut term, 1, "BBBB");
        put_line(&mut term, 2, "CCCC");
        let anchor_point = Point::new(Line(0), Column(0));
        let mut anchor = Some(anchor_point);
        let mut selection =
            Selection::new(AlacrittySelectionType::Simple, anchor_point, Side::Left);
        selection.update(Point::new(Line(2), Column(3)), Side::Right);
        selection.include_all();
        term.selection = Some(selection);
        let previous = term.selection.as_ref().unwrap().to_range(&term);
        assert!(term
            .selection_to_string()
            .unwrap_or_default()
            .contains("CCCC"));
        term.selection = None;
        restore_then_extend(&mut term, &mut anchor, previous, 1);
        let selected = term.selection_to_string().unwrap_or_default();
        assert!(
            selected.contains("CCCC"),
            "scroll after clear dropped the previous selection: {selected:?}"
        );
    }

    fn put_line(term: &mut Term<VoidListener>, line: i32, text: &str) {
        for (col, ch) in text.chars().enumerate() {
            term.grid_mut()[Line(line)][Column(col)].c = ch;
        }
    }

    /// A drag that already reaches the top of the view, then a wheel into
    /// history. Updating the end to a stationary pointer leaves the line that
    /// entered at the top unselected.
    #[test]
    fn drag_scroll_selects_the_history_line_that_enters_the_view() {
        let mut term = Term::new(term::Config::default(), &TermSize::new(4, 3), VoidListener);
        put_line(&mut term, 0, "HIST");
        put_line(&mut term, 1, "AAAA");
        put_line(&mut term, 2, "BBBB");
        term.grid_mut().scroll_up(&(Line(0)..Line(3)), 1);
        assert_eq!(term.grid()[Line(-1)][Column(0)].c, 'H');

        let mut anchor = Some(Point::new(Line(0), Column(0)));
        term.selection = Some(Selection::new(
            AlacrittySelectionType::Simple,
            anchor.unwrap(),
            Side::Left,
        ));
        term.selection
            .as_mut()
            .unwrap()
            .update(Point::new(Line(2), Column(3)), Side::Right);
        term.scroll_display(Scroll::Delta(1));
        extend_drag_selection(&mut term, &mut anchor, 1);

        let selected = term.selection_to_string().unwrap_or_default();
        assert!(
            selected.contains("HIST"),
            "history line that entered the view was not selected: {selected:?}"
        );
        let offset = term.grid().display_offset();
        let range = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        let snap = snapshot_viewport(term.grid());
        assert_eq!(snap[Line(0)][Column(0)].c, 'H');
        let live = Point::new(Line(0 - offset as i32), Column(0));
        assert!(
            range.contains(live),
            "top viewport row is not highlighted: {range:?} offset {offset}"
        );
    }

    /// One selected row at the bottom, two lines of history, a five-line request.
    /// Extending by the request would run through rows the viewport never reached.
    #[test]
    fn local_scroll_extends_by_the_lines_the_viewport_moved() {
        let config = term::Config {
            scrolling_history: 2,
            ..term::Config::default()
        };
        let mut term = Term::new(config, &TermSize::new(4, 4), VoidListener);
        term.grid_mut().scroll_up(&(Line(0)..Line(4)), 2);
        assert_eq!(term.grid().history_size(), 2);

        let mut anchor = Some(Point::new(Line(3), Column(0)));
        term.selection = Some(Selection::new(
            AlacrittySelectionType::Simple,
            anchor.unwrap(),
            Side::Left,
        ));
        let applied = scroll_local_drag(&mut term, &mut anchor, 5);
        assert_eq!(applied, 2);
        let range = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        assert_eq!(range.end.line.0 - range.start.line.0, applied);

        let stuck = term.selection_to_string().unwrap_or_default();
        let again = scroll_local_drag(&mut term, &mut anchor, 5);
        assert_eq!(again, 0);
        assert_eq!(term.grid().display_offset(), 2);
        assert_eq!(term.selection_to_string().unwrap_or_default(), stuck);
    }

    #[test]
    fn local_scroll_with_no_history_does_not_grow_the_selection() {
        let mut term = Term::new(term::Config::default(), &TermSize::new(4, 6), VoidListener);
        term.swap_alt();
        assert_eq!(term.grid().history_size(), 0);
        let mut anchor = Some(Point::new(Line(2), Column(0)));
        term.selection = Some(Selection::new(
            AlacrittySelectionType::Simple,
            anchor.unwrap(),
            Side::Left,
        ));
        term.selection
            .as_mut()
            .unwrap()
            .update(Point::new(Line(3), Column(3)), Side::Right);
        let before = term.selection.as_ref().unwrap().to_range(&term).unwrap();

        for lines in [4, -4] {
            let applied = scroll_local_drag(&mut term, &mut anchor, lines);
            assert_eq!(applied, 0);
            assert_eq!(term.grid().display_offset(), 0);
            let after = term.selection.as_ref().unwrap().to_range(&term).unwrap();
            assert_eq!(after.start.line, before.start.line);
            assert_eq!(after.end.line, before.end.line);
        }
    }

    /// Press above the moving end. Output scrolls both endpoints up and leaves
    /// the stored anchor on the old line, which now belongs to the moving end.
    #[test]
    fn output_during_drag_does_not_swap_fixed_endpoint() {
        let mut term = Term::new(term::Config::default(), &TermSize::new(8, 6), VoidListener);
        let mut anchor = Some(Point::new(Line(2), Column(0)));
        let mut selection =
            Selection::new(AlacrittySelectionType::Simple, anchor.unwrap(), Side::Left);
        selection.update(Point::new(Line(4), Column(7)), Side::Right);
        term.selection = Some(selection);
        Handler::scroll_up(&mut term, 2);
        let shifted = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        assert_eq!(shifted.start.line, Line(0));
        extend_drag_selection(&mut term, &mut anchor, 1);
        term.selection
            .as_mut()
            .unwrap()
            .update(Point::new(Line(1), Column(7)), Side::Right);
        let after = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        assert!(
            after.contains(shifted.start),
            "original fixed endpoint {:?} lost in {:?}",
            shifted.start,
            after
        );
    }

    #[test]
    fn output_during_same_column_drag_does_not_swap_fixed_endpoint() {
        let mut term = Term::new(term::Config::default(), &TermSize::new(8, 6), VoidListener);
        let mut anchor = Some(Point::new(Line(2), Column(0)));
        let mut selection =
            Selection::new(AlacrittySelectionType::Simple, anchor.unwrap(), Side::Left);
        selection.update(Point::new(Line(4), Column(0)), Side::Right);
        term.selection = Some(selection);
        Handler::scroll_up(&mut term, 2);
        let shifted = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        assert_eq!(shifted.end, anchor.unwrap());
        extend_drag_selection(&mut term, &mut anchor, 1);
        term.selection
            .as_mut()
            .unwrap()
            .update(Point::new(Line(1), Column(0)), Side::Right);
        let after = term.selection.as_ref().unwrap().to_range(&term).unwrap();
        assert!(
            after.contains(shifted.start),
            "original fixed endpoint {:?} lost in {:?}",
            shifted.start,
            after
        );
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

/// One grid row as searchable text. `columns[c]` is the grid column of the
/// `c`-th char in `text`.
#[derive(Clone, Debug)]
pub struct SearchRow {
    pub line: Line,
    pub text: String,
    pub columns: Vec<usize>,
}

/// One literal find hit in grid coordinates (single row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoundMatch {
    pub line: i32,
    pub start_col: usize,
    pub end_col: usize,
}

/// Result of [`TerminalBackend::find`]. `truncated` is set when hits hit
/// the [`crate::find::MAX_MATCHES`] cap.
#[derive(Clone, Debug, Default)]
pub struct FindOutcome {
    pub matches: Vec<FoundMatch>,
    pub truncated: bool,
}

pub struct RenderableContent {
    /// Visible cells only. Scrollback stays in the live `Term`.
    pub grid: Grid<Cell>,
    /// Live `Term` display offset. Mouse reports and fixtures use this, not `grid`.
    pub display_offset: usize,
    pub selected_text: String,
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
            display_offset: 0,
            selected_text: String::new(),
            hovered_hyperlink: None,
            selectable_range: None,
            cursor: Cell::default(),
            terminal_mode: TermMode::empty(),
            terminal_size: TerminalSize::default(),
        }
    }
}

fn snapshot_viewport(src: &Grid<Cell>) -> Grid<Cell> {
    let lines = src.screen_lines().max(1);
    let cols = src.columns().max(1);
    let offset = src.display_offset();
    let mut dst = Grid::new(lines, cols, 0);
    for indexed in src.display_iter() {
        let row = indexed.point.line.0 + offset as i32;
        if row < 0 {
            continue;
        }
        let row = row as usize;
        if row >= lines {
            continue;
        }
        dst[Line(row as i32)][indexed.point.column] = indexed.cell.clone();
    }
    dst.cursor = src.cursor.clone();
    dst.cursor.point.line = Line(src.cursor.point.line.0 + offset as i32);
    dst
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

#[cfg(test)]
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
    fn from_layout_uses_floor_cell_size() {
        let size = TerminalSize::from_layout(Size::new(80.0, 48.0), Size::new(8.0, 16.0));
        assert_eq!(size.num_cols, 10);
        assert_eq!(size.num_lines, 3);
        assert_eq!(size.cell_width, 8);
        assert_eq!(size.cell_height, 16);
        assert!(!size.is_empty());
        assert!(TerminalSize::from_layout(Size::new(4.0, 16.0), Size::new(8.0, 16.0)).is_empty());
    }

    #[test]
    fn narrow_pane_pty_is_not_the_80x50_default() {
        let font = Size::new(8.0, 16.0);
        let pane = TerminalSize::from_pane(egui::vec2(70.0 * 8.0, 24.0 * 16.0), font);
        assert_eq!(pane.num_cols, 70);
        assert_eq!(pane.num_lines, 24);
        let settings = BackendSettings {
            size: pane,
            ..BackendSettings::default()
        };
        let pty = pty_size(&settings);
        assert_eq!(pty.num_cols, 70);
        assert_ne!(pty.num_cols, TerminalSize::default().num_cols);
        assert_eq!(pty.num_lines, 24);
    }

    #[test]
    fn empty_pane_falls_back_to_the_default_pty() {
        let empty = TerminalSize::from_layout(Size::new(0.0, 0.0), Size::new(8.0, 16.0));
        assert!(empty.is_empty());
        let settings = BackendSettings {
            size: empty,
            ..BackendSettings::default()
        };
        let pty = pty_size(&settings);
        assert_eq!(pty.num_cols, 80);
        assert_eq!(pty.num_lines, 50);
        assert_eq!(pty_size(&BackendSettings::default()).num_cols, 80);
    }

    #[test]
    fn viewport_snapshot_does_not_retain_scrollback_rows() {
        let mut grid = Grid::<Cell>::new(4, 8, 10_000);
        grid.scroll_up(&(Line(0)..Line(4)), 500);
        assert!(grid.history_size() >= 500);
        let snap = snapshot_viewport(&grid);
        assert_eq!(snap.screen_lines(), 4);
        assert_eq!(snap.history_size(), 0);
        assert_eq!(snap.display_offset(), 0);
    }

    #[test]
    fn viewport_snapshot_copies_scrolled_visible_row_at_line_zero() {
        let mut grid = Grid::<Cell>::new(2, 8, 100);
        for (col, c) in "history!".chars().enumerate() {
            grid[Line(0)][Column(col)].c = c;
        }
        grid.scroll_up(&(Line(0)..Line(2)), 1);
        for (col, c) in "onscrn!!".chars().enumerate() {
            grid[Line(0)][Column(col)].c = c;
        }
        grid.scroll_display(Scroll::Delta(1));
        assert_eq!(grid.display_offset(), 1);
        let snap = snapshot_viewport(&grid);
        assert_eq!(snap.display_offset(), 0);
        assert_eq!(snap.history_size(), 0);
        let text: String = (0..8).map(|col| snap[Line(0)][Column(col)].c).collect();
        assert_eq!(text, "history!");
    }

    #[test]
    fn offscreen_copy_uses_cached_string_because_snapshot_has_no_history() {
        let mut grid = Grid::<Cell>::new(2, 6, 4);
        for (col, c) in "first".chars().enumerate() {
            grid[Line(0)][Column(col)].c = c;
        }
        grid.scroll_up(&(Line(0)..Line(2)), 1);
        for (col, c) in "second".chars().enumerate() {
            grid[Line(0)][Column(col)].c = c;
        }
        let content = RenderableContent {
            grid: snapshot_viewport(&grid),
            selected_text: "first\nsecond".into(),
            selectable_range: Some(SelectionRange::new(
                Point::new(Line(-1), Column(0)),
                Point::new(Line(0), Column(5)),
                false,
            )),
            ..Default::default()
        };
        assert_eq!(content.selected_text, "first\nsecond");
        assert_eq!(content.grid.history_size(), 0);
        assert_eq!(content.grid.topmost_line(), Line(0));
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

#[cfg(test)]
mod repaint_tests {
    use super::*;

    #[test]
    fn split_panes_share_one_output_frame_deadline() {
        let ctx = egui::Context::default();
        let left = repaint_budget(&ctx);
        let right = repaint_budget(&ctx);
        assert!(Arc::ptr_eq(&left, &right));
        let now = Instant::now();
        left.lock().unwrap().painted(now);
        assert_eq!(
            left.lock().unwrap().request(now + Duration::from_millis(5)),
            Some(Duration::from_millis(28))
        );
        assert_eq!(
            right
                .lock()
                .unwrap()
                .request(now + Duration::from_millis(20)),
            None
        );
        // A second producer at the deadline must not queue an immediate extra frame.
        assert_eq!(right.lock().unwrap().request(now + REPAINT_FRAME), None);
        left.lock().unwrap().painted(now + REPAINT_FRAME);
        assert_eq!(
            right
                .lock()
                .unwrap()
                .request(now + Duration::from_millis(34)),
            Some(Duration::from_millis(32))
        );
    }

    #[test]
    fn first_output_after_idle_is_immediate_and_contexts_are_independent() {
        let now = Instant::now();
        let mut budget = RepaintBudget::default();
        assert_eq!(budget.request(now), Some(Duration::ZERO));
        budget.painted(now);
        assert_eq!(
            budget.request(now + Duration::from_secs(1)),
            Some(Duration::ZERO)
        );
        assert!(!Arc::ptr_eq(
            &repaint_budget(&egui::Context::default()),
            &repaint_budget(&egui::Context::default())
        ));
    }
}
