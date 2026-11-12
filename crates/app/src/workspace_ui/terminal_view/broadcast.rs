//! Broadcast input: while the explicit per-tab toggle is on, keys typed
//! and text pasted in one terminal of a workspace tab go to every terminal
//! pane of that tab. It is GUI memory only (never saved), stops when the
//! tab changes or on Esc pressed twice, and a paste of more than one line
//! waits for one confirmation before it goes to several panes.
use super::super::super::*;

/// Two Esc presses inside this window turn broadcast off.
const ESC_TWICE: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(crate) struct Broadcast {
    /// Project and workspace tab id while broadcast is on.
    on: Option<(String, String)>,
    last_esc: Option<Instant>,
    /// A multi-line paste that waits for its confirmation.
    pending: Option<PendingPaste>,
}

#[derive(Clone, Debug)]
pub(crate) struct PendingPaste {
    /// The terminal the paste came from; its pane shows the question.
    pub(crate) from: String,
    pub(crate) text: String,
}

impl PendingPaste {
    fn lines(&self) -> usize {
        self.text.lines().count().max(1)
    }
}

/// True when the text has more than one line.
fn multi_line(text: &str) -> bool {
    text.trim_end_matches(['\r', '\n']).contains(['\r', '\n'])
}

impl App {
    /// The project and workspace tab that broadcast input targets.
    pub(crate) fn broadcast_tab(&self) -> Option<(&str, &str)> {
        self.broadcast
            .on
            .as_ref()
            .map(|(project, tab)| (project.as_str(), tab.as_str()))
    }

    /// Broadcast for the selected project's active tab, or off when on.
    pub(crate) fn toggle_broadcast(&mut self) {
        if self.broadcast.on.is_some() {
            self.stop_broadcast("Broadcast input off.");
            return;
        }
        let Some(project) = self.selected.clone() else {
            return;
        };
        let Some(tab) = self.layouts.get(&project).map(|w| w.active.clone()) else {
            return;
        };
        self.start_broadcast(project, tab);
    }

    /// The pane menu toggle for the tab the pane is in.
    pub(crate) fn toggle_broadcast_for(&mut self, project: &str, tab: &str) {
        if self.broadcast_tab() == Some((project, tab)) {
            self.stop_broadcast("Broadcast input off.");
        } else {
            self.start_broadcast(project.to_owned(), tab.to_owned());
        }
    }

    fn start_broadcast(&mut self, project: String, tab: String) {
        self.broadcast = Broadcast {
            on: Some((project, tab)),
            ..Broadcast::default()
        };
        let count = self.broadcast_targets().len();
        self.info = Some(format!(
            "Broadcast input on: keys go to {count} terminals in this tab. Press Esc twice to stop."
        ));
    }

    pub(crate) fn stop_broadcast(&mut self, note: &str) {
        if self.broadcast.on.is_none() {
            return;
        }
        self.broadcast = Broadcast::default();
        self.info = Some(note.to_owned());
    }

    /// Live shell terminals of the broadcast tab, in layout order.
    pub(crate) fn broadcast_targets(&self) -> Vec<String> {
        let Some((project, tab)) = self.broadcast_tab() else {
            return Vec::new();
        };
        let Some(group) = self
            .layouts
            .get(project)
            .and_then(|w| w.tabs.iter().find(|group| group.id == tab))
        else {
            return Vec::new();
        };
        group
            .layout
            .iter_all_tabs()
            .filter_map(|(_, tab)| match tab {
                Tab::Terminal(sid) => Some(sid),
                _ => None,
            })
            .filter(|sid| {
                self.state.sessions.iter().any(|s| {
                    &s.id == *sid && s.lifecycle.live() && s.kind == SessionKind::Shell && !s.review
                })
            })
            .cloned()
            .collect()
    }

    /// Whether `sid` takes part in broadcast input now.
    pub(crate) fn broadcasting_to(&self, sid: &str) -> bool {
        self.broadcast.on.is_some() && self.broadcast_targets().iter().any(|t| t == sid)
    }

    /// Broadcast stops once its tab is no longer the selected project's
    /// active tab. Runs every frame.
    pub(crate) fn follow_broadcast_tab(&mut self) {
        let Some((project, tab)) = self.broadcast.on.clone() else {
            return;
        };
        let current = self.selected.as_deref() == Some(project.as_str())
            && self.layouts.get(&project).is_some_and(|w| w.active == tab);
        if !current {
            self.stop_broadcast("Broadcast input stopped: the tab changed.");
        }
    }

    /// Before the focused terminal reads input: Esc pressed twice stops
    /// broadcast (the second Esc never reaches a PTY), and pastes go to
    /// every target through [`Self::broadcast_paste`].
    pub(crate) fn broadcast_pre_input(&mut self, ctx: &egui::Context, sid: &str) {
        if !self.broadcasting_to(sid) {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape) && !i.modifiers.any()) {
            let now = Instant::now();
            if self
                .broadcast
                .last_esc
                .is_some_and(|last| now.saturating_duration_since(last) <= ESC_TWICE)
            {
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
                self.stop_broadcast("Broadcast input off.");
                return;
            }
            self.broadcast.last_esc = Some(now);
        }
        let pastes = ctx.input_mut(|input| take_pastes(&mut input.events, input.modifiers));
        for text in pastes {
            self.broadcast_paste(sid, text);
        }
    }

    /// One paste into every target. More than one line waits for
    /// [`Self::answer_broadcast_paste`].
    pub(crate) fn broadcast_paste(&mut self, from: &str, text: String) {
        if multi_line(&text) {
            self.broadcast.pending = Some(PendingPaste {
                from: from.to_owned(),
                text,
            });
            return;
        }
        for sid in self.broadcast_targets() {
            self.handle_clipboard_paste(&sid, &text);
        }
    }

    /// The pending multi-line paste, if any.
    #[cfg(test)]
    pub(crate) fn pending_broadcast_paste(&self) -> Option<&PendingPaste> {
        self.broadcast.pending.as_ref()
    }

    /// Send (true) or drop (false) the pending multi-line paste.
    pub(crate) fn answer_broadcast_paste(&mut self, send: bool) {
        let Some(pending) = self.broadcast.pending.take() else {
            return;
        };
        if !send {
            return;
        }
        for sid in self.broadcast_targets() {
            self.handle_clipboard_paste(&sid, &pending.text);
        }
    }

    /// Bytes the focused terminal's keys wrote go to the other targets.
    pub(crate) fn broadcast_typed(&mut self, from: &str, typed: Vec<Vec<u8>>) {
        if typed.is_empty() || !self.broadcasting_to(from) {
            return;
        }
        for sid in self.broadcast_targets() {
            if sid == from {
                continue;
            }
            if let Some(backend) = self.backends.get_mut(&sid) {
                for bytes in &typed {
                    backend.process_command(egui_term::BackendCommand::Write(bytes.clone()));
                }
            }
        }
    }

    /// The question for a multi-line paste, in the pane it came from.
    pub(crate) fn broadcast_paste_bar(&mut self, ui: &mut egui::Ui, sid: &str) {
        let Some(pending) = self.broadcast.pending.clone().filter(|p| p.from == sid) else {
            return;
        };
        let count = self.broadcast_targets().len();
        let mut answer = None;
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                appearance::color(&self.theme.status_waiting),
                format!("Paste {} lines into {count} terminals?", pending.lines()),
            );
            if ui.button("Paste to all").clicked() {
                answer = Some(true);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(false);
            }
        });
        if let Some(send) = answer {
            self.answer_broadcast_paste(send);
        }
    }
}

/// Paste events that type text (egui_term sends ^V for a plain Ctrl+V
/// paste off macOS; that byte stays a key).
fn take_pastes(events: &mut Vec<egui::Event>, modifiers: egui::Modifiers) -> Vec<String> {
    let types_text = cfg!(target_os = "macos")
        || modifiers.contains(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    if !types_text {
        return Vec::new();
    }
    let mut pastes = Vec::new();
    events.retain(|event| match event {
        egui::Event::Paste(text) => {
            pastes.push(text.clone());
            false
        }
        _ => true,
    });
    pastes
}

#[cfg(test)]
#[path = "broadcast_tests.rs"]
mod tests;
