//! Draws the inbox. Narrow layouts (a phone over SSH) keep one line per row.
use super::{
    keys::{Inbox, Mode},
    rows::{Rank, Row, age},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

fn rank_style(rank: Rank) -> Style {
    let color = match rank {
        Rank::Waiting => Color::Yellow,
        Rank::Failed => Color::Red,
        Rank::Working => Color::Cyan,
        Rank::DoneUnread => Color::Green,
        Rank::Idle => Color::DarkGray,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn row_line(row: &Row) -> Line<'_> {
    let mut spans = vec![
        Span::styled(format!("{:4} ", row.rank.label()), rank_style(row.rank)),
        Span::raw(format!("{}/{} ", row.project, row.label)),
        Span::styled(age(row.age), Style::default().fg(Color::DarkGray)),
    ];
    if let Some(notice) = &row.notice {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            notice.text(),
            Style::default().add_modifier(Modifier::ITALIC),
        ));
    }
    Line::from(spans)
}

fn footer(inbox: &Inbox) -> Line<'static> {
    let keys = match &inbox.mode {
        Mode::List => "Enter view  r reply  d dismiss  s snooze  q quit",
        Mode::Screen { .. } => "Enter refresh  r reply  Esc back",
        Mode::Reply { .. } => "Type the reply, Enter to review, Esc discard",
        Mode::Confirm { .. } => "s SEND with Enter  t type only (no Enter)  Esc edit",
    };
    let mut spans = vec![Span::styled(
        keys,
        Style::default().fg(Color::Black).bg(Color::Gray),
    )];
    if !inbox.status.is_empty() {
        spans.push(Span::raw(format!("  {}", inbox.status)));
    }
    Line::from(spans)
}

/// `screen` is the read-only text of the session in the Screen mode.
pub(super) fn draw(frame: &mut Frame<'_>, inbox: &Inbox, rows: &[Row], screen: &str) {
    let [body, prompt, keys] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let session = match &inbox.mode {
        Mode::Screen { session } | Mode::Reply { session, .. } | Mode::Confirm { session, .. } => {
            Some(session)
        }
        Mode::List => None,
    };
    match session.filter(|_| !screen.is_empty()) {
        Some(session) => draw_screen(frame, body, session, screen),
        None => draw_list(frame, body, inbox, rows),
    }
    let prompt_line = match &inbox.mode {
        Mode::Reply { line, .. } => format!("reply> {line}_"),
        Mode::Confirm { line, .. } => format!("send?> {line}"),
        _ => String::new(),
    };
    frame.render_widget(Paragraph::new(prompt_line), prompt);
    frame.render_widget(Paragraph::new(footer(inbox)), keys);
}

fn draw_list(frame: &mut Frame<'_>, area: ratatui::layout::Rect, inbox: &Inbox, rows: &[Row]) {
    let title = format!(" Terminator inbox ({}) ", rows.len());
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new("No agent sessions.")
                .block(Block::default().borders(Borders::TOP).title(title)),
            area,
        );
        return;
    }
    let items: Vec<ListItem<'_>> = rows
        .iter()
        .map(|row| ListItem::new(row_line(row)))
        .collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::TOP).title(title))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default().with_selected(Some(inbox.selected));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_screen(frame: &mut Frame<'_>, area: ratatui::layout::Rect, session: &str, screen: &str) {
    let height = usize::from(area.height.saturating_sub(1));
    let lines: Vec<&str> = screen.lines().collect();
    let tail = lines
        .get(lines.len().saturating_sub(height)..)
        .unwrap_or_default();
    let text: Vec<Line<'_>> = tail.iter().map(|line| Line::raw(*line)).collect();
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::TOP)
                .title(format!(" {session} (read-only) ")),
        ),
        area,
    );
}
