//! Prompt composer: a multi-line prompt for the focused agent terminal,
//! with `@file` completion from the file index, prompt history per project
//! (a GUI preference) and a follow-up queue. Text reaches the agent only
//! through the Ask agent paste path on an explicit click or key: Paste
//! types it without Enter, Paste and send also presses Enter. A queued
//! prompt never sends itself: when a hook reports the agent waiting or
//! completed, the composer offers "Send next" for one click.
mod mention;
mod view;

use crate::App;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use terminator_core::AgentState;

pub(crate) use mention::{insert_mention, mention_at};

/// Prompts kept per project, newest first.
pub(crate) const HISTORY_PER_PROJECT: usize = 50;
/// Projects with a kept history.
pub(crate) const HISTORY_PROJECTS: usize = 100;
/// Longest prompt kept in history, in bytes.
pub(crate) const HISTORY_PROMPT_BYTES: usize = 8 * 1024;
/// Most queued prompts per session.
pub(crate) const QUEUE_MAX: usize = 20;

/// The open composer.
#[derive(Clone, Debug, Default)]
pub(crate) struct Composer {
    /// Agent session that receives the prompt.
    pub(crate) target: String,
    pub(crate) text: String,
    /// Focus the text area on the next frame.
    pub(crate) focus: bool,
    /// Byte offset of the caret, from the last frame.
    pub(crate) caret: usize,
    /// Highlighted `@` completion row.
    pub(crate) pick: usize,
}

/// Composer state: the window and the follow-up queues by session (GUI
/// memory only).
#[derive(Default)]
pub(crate) struct ComposerState {
    pub(crate) open: Option<Composer>,
    pub(crate) queues: HashMap<String, VecDeque<String>>,
}

/// Whether a queued prompt may be offered for sending: the agent's last
/// hook state says it waits for input or finished its turn. Never from
/// terminal text.
#[must_use]
pub(crate) fn queue_ready(state: Option<AgentState>) -> bool {
    matches!(
        state,
        Some(AgentState::WaitingInput | AgentState::Completed)
    )
}

/// The text to paste. A terminal without bracketed paste would take a line
/// break as Enter, so the prompt goes as one line there; trailing line
/// breaks never go.
#[must_use]
pub(crate) fn paste_text(text: &str, bracketed: bool) -> String {
    let text = text.trim_end();
    if bracketed {
        text.trim_start_matches(['\n', '\r']).to_owned()
    } else {
        crate::diff::notes::one_line(text)
    }
}

/// `history` with `prompt` first, without duplicates, bounded.
pub(crate) fn remember(history: &mut Vec<String>, prompt: &str) {
    let prompt = prompt.trim();
    if prompt.is_empty() || prompt.len() > HISTORY_PROMPT_BYTES {
        return;
    }
    history.retain(|old| old != prompt);
    history.insert(0, prompt.to_owned());
    history.truncate(HISTORY_PER_PROJECT);
}

/// Adds `prompt` to the history of `project` and drops the first projects
/// past [`HISTORY_PROJECTS`].
pub(crate) fn remember_for(all: &mut BTreeMap<PathBuf, Vec<String>>, project: &Path, prompt: &str) {
    remember(all.entry(project.to_path_buf()).or_default(), prompt);
    while all.len() > HISTORY_PROJECTS {
        let Some(first) = all.keys().find(|key| key.as_path() != project).cloned() else {
            break;
        };
        all.remove(&first);
    }
}

impl App {
    /// The newest hook-reported state of the agent in `session`.
    pub(crate) fn hook_agent_state(&self, session: &str) -> Option<AgentState> {
        self.state
            .agents
            .iter()
            .filter(|agent| agent.session_id == session)
            .max_by_key(|agent| agent.updated)
            .map(|agent| agent.state)
    }

    /// Opens the composer for the focused agent terminal, else the first
    /// live agent terminal.
    pub(crate) fn open_composer(&mut self) {
        let targets = self.ask_targets();
        let Some((target, _)) = targets.first() else {
            self.info = Some("No open agent terminal to write to.".into());
            return;
        };
        self.open_composer_for(target.clone());
    }

    pub(crate) fn open_composer_for(&mut self, target: String) {
        if !self.ask_targets().iter().any(|(id, _)| *id == target) {
            self.info = Some("This terminal has no running agent.".into());
            return;
        }
        let keep = self
            .composer
            .open
            .take()
            .filter(|open| open.target == target);
        self.composer.open = Some(Composer {
            focus: true,
            ..keep.unwrap_or(Composer {
                target,
                ..Composer::default()
            })
        });
    }

    /// The project folder of `session`.
    pub(crate) fn session_root(&self, session: &str) -> Option<PathBuf> {
        let session = self.state.sessions.iter().find(|s| s.id == session)?;
        self.state
            .projects
            .iter()
            .find(|p| p.id == session.project_id)
            .map(|p| p.path.clone())
    }

    /// Pastes `text` into `session` through the Ask agent path; `enter`
    /// also sends it. Keeps it in the project's history. True when pasted.
    pub(crate) fn composer_paste(&mut self, session: &str, text: &str, enter: bool) -> bool {
        let bracketed = self.backends.get(session).is_some_and(|backend| {
            backend
                .last_content()
                .terminal_mode
                .contains(egui_term::TerminalMode::BRACKETED_PASTE)
        });
        let text = paste_text(text, bracketed);
        if !self.paste_prompt(session, &text, enter) {
            return false;
        }
        if let Some(root) = self.session_root(session) {
            remember_for(&mut self.preferences.prompt_history, &root, &text);
        }
        true
    }

    /// Queues `text` for `session`; it waits for a "Send next" click.
    pub(crate) fn queue_prompt(&mut self, session: &str, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let queue = self.composer.queues.entry(session.to_owned()).or_default();
        if queue.len() >= QUEUE_MAX {
            self.info = Some(format!("The queue holds at most {QUEUE_MAX} prompts."));
            return;
        }
        queue.push_back(text.to_owned());
    }

    /// The explicit "Send next" click: pastes and sends the first queued
    /// prompt, only while the agent waits. True when sent.
    pub(crate) fn send_next_queued(&mut self, session: &str) -> bool {
        if !queue_ready(self.hook_agent_state(session)) {
            return false;
        }
        let Some(next) = self
            .composer
            .queues
            .get(session)
            .and_then(|queue| queue.front().cloned())
        else {
            return false;
        };
        if !self.composer_paste(session, &next, true) {
            return false;
        }
        if let Some(queue) = self.composer.queues.get_mut(session) {
            queue.pop_front();
        }
        true
    }

    /// "Prompt composer…" in an agent terminal's pane menu.
    pub(crate) fn composer_menu_item(&mut self, ui: &mut eframe::egui::Ui, sid: &str) {
        if !self.ask_targets().iter().any(|(id, _)| id == sid) {
            return;
        }
        let queued = self.composer.queues.get(sid).map_or(0, VecDeque::len);
        let label = if queued == 0 {
            "Prompt composer…".to_owned()
        } else {
            format!("Prompt composer… ({queued} queued)")
        };
        if crate::appearance::menu_action(ui, true, &label, "Pencil").clicked() {
            self.open_composer_for(sid.to_owned());
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests;
