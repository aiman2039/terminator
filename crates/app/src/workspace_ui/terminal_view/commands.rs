//! Command navigation from shell integration marks (OSC 133): prompt
//! jumps, command selection, the last command's output, exit-code marks in
//! the pane margin, and the long-command notice. The marks describe shell
//! commands only; nothing here reads or changes agent state.
use super::super::super::*;
use egui_term::{CommandEnded, PromptMark};
use terminator_core::TERMINAL_NOTICES_CAPABILITY;

/// Actions on the focused terminal's grid. Their chords run only while a
/// terminal has the keyboard; Find Action runs them on the active terminal.
pub(crate) const TERMINAL_ACTIONS: &[Action] = &[
    Action::PrevPrompt,
    Action::NextPrompt,
    Action::SelectCommand,
    Action::CopyLastOutput,
    Action::HintsMode,
    Action::CopyMode,
];

const NO_MARKS: &str = "No shell prompt marks in this terminal (shell integration is off)";

/// Width of an exit-code mark in the pane's left margin.
const MARK_WIDTH: f32 = 2.0;

impl App {
    /// Whether a terminal action has a terminal to act on.
    pub(crate) fn terminal_action_applies(&self) -> bool {
        self.active_session
            .as_ref()
            .is_some_and(|sid| self.backends.contains_key(sid))
    }

    pub(crate) fn run_terminal_action(&mut self, ctx: &egui::Context, action: Action) {
        if let Some(sid) = self.active_session.clone() {
            self.terminal_action_on(ctx, &sid, action);
        }
    }

    fn terminal_action_on(&mut self, ctx: &egui::Context, sid: &str, action: Action) {
        // The modes follow keyboard focus: they start on the active terminal.
        match action {
            Action::HintsMode => return self.toggle_hints_mode(),
            Action::CopyMode => return self.toggle_copy_mode(),
            _ => {}
        }
        let Some(backend) = self.backends.get_mut(sid) else {
            return;
        };
        let done = match action {
            Action::PrevPrompt | Action::NextPrompt => {
                backend.jump_to_prompt(action == Action::NextPrompt)
            }
            Action::SelectCommand => backend.select_command(),
            Action::CopyLastOutput => match backend.last_command_output() {
                Some(output) => {
                    self.copy_with_note(ctx, output, "Command output");
                    true
                }
                None => false,
            },
            _ => true,
        };
        if !done
            && self
                .backends
                .get_mut(sid)
                .is_some_and(|backend| backend.prompt_marks().is_empty())
        {
            self.info = Some(NO_MARKS.into());
        }
        ctx.request_repaint();
    }

    /// The terminal context menu's command items. They act on `sid`, the
    /// terminal the menu belongs to.
    pub(crate) fn terminal_command_menu(&mut self, ui: &mut egui::Ui, sid: &str) {
        for &action in TERMINAL_ACTIONS {
            if appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: action.label(),
                icon: action.icon(),
                shortcut: &self.shortcut_label(action),
            })
            .clicked()
            {
                self.terminal_action_on(ui.ctx(), sid, action);
                ui.close();
            }
        }
    }

    /// A long shell command ended in a pane without focus: raise a
    /// terminal notice (`terminal-notices-v1`), like OSC 9. Off at 0 s.
    pub(crate) fn notice_long_commands(&mut self, ctx: &egui::Context) {
        let threshold = Duration::from_secs(u64::from(self.preferences.long_command_notice_secs));
        let window_focused = ctx.input(|i| i.viewport().focused.unwrap_or(false));
        let mut notices = Vec::new();
        for (sid, backend) in &self.backends {
            let ended = backend.take_ended_commands();
            if threshold.is_zero() {
                continue;
            }
            let focused = window_focused
                && self.active_session.as_ref() == Some(sid)
                && self.visible_sessions.contains(sid);
            notices.extend(
                ended
                    .into_iter()
                    .filter(|command| long_command_notice(command, threshold, focused))
                    .map(|command| (sid.clone(), command)),
            );
        }
        for (sid, command) in notices {
            if !self.terminal_capability(&sid, TERMINAL_NOTICES_CAPABILITY) {
                continue;
            }
            let (title, body) = notice_text(&command);
            self.send(Request::TerminalNotify {
                session: sid,
                title,
                body,
            });
        }
    }

    /// Whether the daemon generation that owns session `sid` advertises
    /// `capability`.
    pub(crate) fn terminal_capability(&self, sid: &str, capability: &str) -> bool {
        let has = |caps: &[String]| caps.iter().any(|c| c == capability);
        match self.state.sessions.iter().find(|s| s.id == sid) {
            Some(session)
                if !session.generation.is_empty()
                    && session.generation != self.state.generation =>
            {
                self.state
                    .generations
                    .iter()
                    .any(|g| g.owner.id == session.generation && has(&g.capabilities))
            }
            _ => has(&self.state.capabilities),
        }
    }

    /// Exit-code marks beside the visible prompts: green for exit 0, the
    /// failure color otherwise, dim when the shell sent no code.
    pub(crate) fn paint_prompt_marks(&mut self, ui: &egui::Ui, sid: &str, rect: egui::Rect) {
        let ok = appearance::color(&self.theme.git_added);
        let failed = appearance::color(&self.theme.status_failed);
        let unknown = appearance::color(&self.theme.secondary);
        let Some(backend) = self.backends.get_mut(sid) else {
            return;
        };
        let (offset, size, rows) = {
            let content = backend.last_content();
            let rows = i32::from(content.terminal_size.rows());
            (content.display_offset, content.terminal_size, rows)
        };
        let painter = ui.painter().with_clip_rect(rect);
        for mark in backend.prompt_marks() {
            let Some(row) = visible_row(mark, offset, rows) else {
                continue;
            };
            let color = match (mark.finished, mark.exit_code) {
                (false, _) => continue,
                (true, Some(0)) => ok,
                (true, Some(_)) => failed,
                (true, None) => unknown,
            };
            let top = rect.top() + row as f32 * f32::from(size.cell_height);
            let mark_rect = egui::Rect::from_min_size(
                egui::pos2(rect.left(), top),
                egui::vec2(MARK_WIDTH, f32::from(size.cell_height)),
            );
            painter.rect_filled(mark_rect, egui::CornerRadius::ZERO, color);
        }
    }
}

/// Viewport row of a prompt, when it is on screen.
fn visible_row(mark: &PromptMark, display_offset: usize, rows: i32) -> Option<i32> {
    let row = mark.line.checked_add(i32::try_from(display_offset).ok()?)?;
    (0..rows).contains(&row).then_some(row)
}

/// The command ran at least `threshold` and its pane had no focus.
fn long_command_notice(command: &CommandEnded, threshold: Duration, focused: bool) -> bool {
    !focused && !threshold.is_zero() && command.duration >= threshold
}

fn notice_text(command: &CommandEnded) -> (String, String) {
    let took = format!("Ran for {}", duration_text(command.duration));
    match command.exit_code {
        Some(0) => ("Command finished".into(), took),
        Some(code) => (format!("Command failed (exit {code})"), took),
        None => ("Command ended".into(), took),
    }
}

fn duration_text(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{secs} s")
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ended(secs: u64, exit_code: Option<i32>) -> CommandEnded {
        CommandEnded {
            exit_code,
            duration: Duration::from_secs(secs),
        }
    }

    #[test]
    fn only_long_commands_in_unfocused_panes_raise_a_notice() {
        let threshold = Duration::from_secs(30);
        assert!(long_command_notice(&ended(45, Some(0)), threshold, false));
        assert!(!long_command_notice(&ended(45, Some(0)), threshold, true));
        assert!(!long_command_notice(&ended(5, Some(1)), threshold, false));
        assert!(!long_command_notice(
            &ended(45, Some(1)),
            Duration::ZERO,
            false
        ));
    }

    #[test]
    fn notice_names_the_exit_code_and_run_time() {
        assert_eq!(
            notice_text(&ended(125, Some(2))),
            (
                "Command failed (exit 2)".to_owned(),
                "Ran for 2m 5s".to_owned()
            )
        );
        assert_eq!(notice_text(&ended(40, Some(0))).0, "Command finished");
        assert_eq!(notice_text(&ended(40, None)).0, "Command ended");
    }

    /// Cmd+Up in a text field moves its caret; the prompt jump must leave
    /// it there when no terminal has the keyboard.
    #[test]
    fn prompt_chords_stay_with_a_focused_text_field() {
        let (mut app, ctx, _dir) = crate::app::fixture();
        let backend = egui_term::TerminalBackend::new(
            900,
            ctx.clone(),
            app.pty_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                args: vec!["-c".into(), "sleep 30".into()],
                ..Default::default()
            },
        )
        .unwrap();
        app.backends.insert("prompt-shell".into(), backend);
        app.focus_terminal("prompt-shell".into());
        assert!(app.shortcut_applies(Action::PrevPrompt));
        let keys = app.state.settings.keybindings.clone();
        let chord = shortcuts::parse(&shortcuts::binding(&keys, Action::PrevPrompt.id())).unwrap();
        let mut left = 0;
        let raw = egui::RawInput {
            events: vec![egui::Event::Key {
                key: chord.key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: chord.modifiers,
            }],
            ..Default::default()
        };
        let mut text = String::new();
        let mut output = ctx.run_ui(raw, |ui| {
            ui.text_edit_singleline(&mut text).request_focus();
            app.dispatch_shortcuts(ui.ctx(), true);
            left = ui.input(|i| {
                i.events
                    .iter()
                    .filter(|e| matches!(e, egui::Event::Key { .. }))
                    .count()
            });
        });
        output.textures_delta.clear();
        assert_eq!(left, 1);
    }

    #[test]
    fn marks_paint_only_on_visible_rows() {
        let mark = |line| PromptMark {
            line,
            output: None,
            exit_code: Some(0),
            finished: true,
        };
        assert_eq!(visible_row(&mark(-3), 5, 24), Some(2));
        assert_eq!(visible_row(&mark(-3), 0, 24), None);
        assert_eq!(visible_row(&mark(23), 0, 24), Some(23));
        assert_eq!(visible_row(&mark(24), 0, 24), None);
    }
}
