//! The Agent sessions window: a filter field and one row per conversation.
//! Up/Down move the selection; Enter or a click resumes the row. Esc
//! closes the window. The window opens at the pointer for a click and on
//! the focused pane for a key (`Popups::window`).
use super::AgentSessions;
use crate::*;
use terminator_core::agents::{self, conversations::Conversation};

fn filter_id() -> egui::Id {
    egui::Id::unique("agent-sessions-filter")
}

impl App {
    pub(crate) fn agent_sessions_dialog(&mut self, ctx: &egui::Context) {
        if !self.agent_sessions.open {
            return;
        }
        let project = self.agent_sessions.project.as_ref().map(|(id, _)| id);
        if project.is_none() || project != self.selected.as_ref() {
            // The list belongs to one project; another selection closes it.
            self.agent_sessions.close();
            return;
        }
        self.agent_sessions.poll();
        // Read before the field paints: Esc also takes its focus away.
        let escape = ctx.memory(|memory| memory.has_focus(filter_id()))
            && ctx.input(|input| input.key_pressed(egui::Key::Escape));
        let mut sessions = std::mem::take(&mut self.agent_sessions);
        let mut open = true;
        let mut picked = None;
        let tint = appearance::color(&self.theme.text);
        self.popups
            .window(ctx, "Agent sessions")
            .open(&mut open)
            .collapsible(false)
            .default_width(520.0)
            .show(ctx, |ui| picked = sessions_body(ui, &mut sessions, tint));
        if sessions.listing.is_none() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.agent_sessions = sessions;
        if !open || escape {
            self.agent_sessions.close();
        } else if let Some(conversation) = picked {
            self.resume_agent_session(&conversation);
        }
    }
}

/// The filter, the rows and the empty states. Returns the row picked by
/// Enter or a click.
fn sessions_body(
    ui: &mut egui::Ui,
    sessions: &mut AgentSessions,
    tint: egui::Color32,
) -> Option<Conversation> {
    ui.set_min_width(460.0);
    let field = ui.add(
        appearance::singleline(&mut sessions.filter)
            .id(filter_id())
            .hint_text("Filter by title or agent")
            .desired_width(f32::INFINITY),
    );
    if std::mem::take(&mut sessions.focus) {
        field.request_focus();
    }
    if field.changed() {
        sessions.cursor = 0;
    }
    // Only an Enter typed in the field resumes: the Enter that chose Find
    // Action's row lands before the field has focus.
    let enter = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
    ui.add_space(6.0);
    let Some(listing) = &sessions.listing else {
        ui.weak("Reading Claude Code and Codex conversations…");
        return None;
    };
    if listing.conversations.is_empty() {
        ui.weak("No Claude Code or Codex conversation was found for this project folder.");
        return None;
    }
    let rows = sessions.matching();
    if rows.is_empty() {
        ui.weak("No matching conversation.");
        return None;
    }
    let moved = move_cursor(ui, &mut sessions.cursor, rows.len());
    let cursor = sessions.cursor;
    let mut picked = enter.then_some(cursor);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |age| age.as_secs());
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (row, index) in rows.iter().enumerate() {
                let Some(conversation) = listing.conversations.get(*index) else {
                    continue;
                };
                if session_row(ui, (conversation, now, tint), row == cursor, moved) {
                    picked = Some(row);
                }
            }
        });
    if listing.stopped {
        ui.weak("Showing the newest conversations read within the time and size limits.");
    }
    let index = rows.get(picked?)?;
    listing.conversations.get(*index).cloned()
}

/// Up/Down move the selection within `count` rows. True when it moved.
fn move_cursor(ui: &mut egui::Ui, cursor: &mut usize, count: usize) -> bool {
    let (down, up) = ui.input_mut(|input| {
        (
            input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
        )
    });
    let last = count.saturating_sub(1);
    let before = (*cursor).min(last);
    *cursor = if down {
        before.saturating_add(1).min(last)
    } else if up {
        before.saturating_sub(1)
    } else {
        before
    };
    down || up
}

/// One conversation row; true when clicked.
fn session_row(
    ui: &mut egui::Ui,
    (conversation, now, tint): (&Conversation, u64, egui::Color32),
    selected: bool,
    moved: bool,
) -> bool {
    let command = super::resume_command(conversation).unwrap_or_default();
    let trailing = format!(
        "{} · {}",
        agents::display_name(conversation.agent),
        when(conversation.updated, now)
    );
    let row = appearance::row(appearance::RowInput {
        ui,
        label: &conversation.title,
        icon: agents::icon_key(conversation.agent),
        selected,
        height: 26.0,
        trailing: &trailing,
        tint,
    });
    if selected && moved {
        row.scroll_to_me(None);
    }
    row.on_hover_text(format!(
        "{}\nEnter or click runs in a new terminal:\n{command}",
        conversation.title
    ))
    .clicked()
}

/// `5m ago`, `3h ago`, `2d ago`, else the date.
pub(super) fn when(updated: u64, now: u64) -> String {
    let age = now.saturating_sub(updated);
    if age < 3600 {
        format!("{}m ago", age / 60)
    } else if age < 86_400 {
        format!("{}h ago", age / 3600)
    } else if age < 604_800 {
        format!("{}d ago", age / 86_400)
    } else {
        crate::native_editor::ymd(i64::try_from(updated).unwrap_or(0))
    }
}
