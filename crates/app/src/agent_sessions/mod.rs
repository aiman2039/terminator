//! Agent sessions (#314): the past Claude Code and Codex conversations of
//! the selected local project, listed from the CLIs' own files by a
//! replaceable background read on the GUI services (bounded; see
//! `terminator_core::agents::conversations`). Only an explicit Enter or
//! click on a row resumes one: a new terminal in the project folder runs
//! the CLI's own resume command, the same path as History's resume picker.
//! The list is never a lifecycle or presence source.
mod job;
#[cfg(test)]
mod tests;
mod view;

use crate::App;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use terminator_core::agents::conversations::{Conversation, Listing, valid_id};
use terminator_core::async_service::CancellationToken;

type Slot = Arc<Mutex<Option<(u64, Listing)>>>;

/// The Agent sessions window (GUI memory only).
#[derive(Default)]
pub(crate) struct AgentSessions {
    pub(crate) open: bool,
    /// Project id and folder the list belongs to.
    project: Option<(String, PathBuf)>,
    generation: u64,
    pub(crate) listing: Option<Listing>,
    pub(crate) filter: String,
    /// Selected row among the filtered rows.
    pub(crate) cursor: usize,
    /// Give the filter field keyboard focus on the next paint.
    focus: bool,
    slot: Slot,
    cancel: Option<CancellationToken>,
}

impl AgentSessions {
    /// Takes a finished listing; an older generation is dropped.
    fn poll(&mut self) {
        let done = self
            .slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some((generation, listing)) = done
            && generation == self.generation
        {
            self.listing = Some(listing);
            self.cancel = None;
        }
    }

    /// Closes the window and stops a running read.
    pub(crate) fn close(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel.cancel();
        }
        self.open = false;
    }

    /// Indexes of the listed conversations whose title or agent name
    /// contains the filter, ignoring case.
    pub(crate) fn matching(&self) -> Vec<usize> {
        let Some(listing) = &self.listing else {
            return Vec::new();
        };
        let needle = self.filter.trim().to_lowercase();
        listing
            .conversations
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                needle.is_empty()
                    || c.title.to_lowercase().contains(&needle)
                    || terminator_core::agents::display_name(c.agent)
                        .to_lowercase()
                        .contains(&needle)
            })
            .map(|(index, _)| index)
            .collect()
    }
}

/// The CLI's own resume command for `conversation`, shell-quoted; `None`
/// when the id is not a plain id or the agent has no resume command.
pub(crate) fn resume_command(conversation: &Conversation) -> Option<String> {
    if !valid_id(&conversation.id) {
        return None;
    }
    terminator_integrations::resume_for(conversation.agent, &conversation.id)
        .map(|resume| resume.display())
}

impl App {
    /// Opens the Agent sessions window for the selected local project and
    /// lists its conversations in the background.
    pub(crate) fn open_agent_sessions(&mut self) {
        if self.selected_remote_project().is_some() {
            self.info = Some("Agent sessions are listed for local projects only.".into());
            return;
        }
        let Some(project) = self.selected_project() else {
            self.info = Some("Select a local project, then list its agent sessions.".into());
            return;
        };
        let (id, root) = (project.id.clone(), project.path.clone());
        let generation = self.agent_sessions.generation.wrapping_add(1);
        self.agent_sessions.close();
        self.agent_sessions = AgentSessions {
            open: true,
            project: Some((id, root.clone())),
            generation,
            focus: true,
            ..AgentSessions::default()
        };
        self.start_agent_sessions_job(root);
    }

    /// Resumes `conversation` on an explicit Enter or click: a new terminal
    /// at the project root runs the CLI's resume command with Enter, like
    /// History's resume picker. Nothing runs for another project.
    pub(crate) fn resume_agent_session(&mut self, conversation: &Conversation) {
        let project = self.agent_sessions.project.as_ref().map(|(id, _)| id);
        if project.is_none() || project != self.selected.as_ref() {
            self.agent_sessions.close();
            return;
        }
        let Some(command) = resume_command(conversation) else {
            self.info = Some("This conversation cannot be resumed.".into());
            return;
        };
        self.agent_sessions.close();
        self.open_resume_picker(&command);
    }
}
