//! Closing editor sessions: unsaved-buffer prompts and the close jobs.

use crate::*;

impl App {
    pub(crate) fn editors_only(&self, ids: &[String]) -> bool {
        !ids.is_empty()
            && !self.state.agents.iter().any(|agent| {
                ids.contains(&agent.session_id)
                    && !matches!(
                        agent.state,
                        AgentState::Completed | AgentState::Failed | AgentState::Stopped
                    )
            })
            && ids.iter().all(|id| {
                self.state
                    .sessions
                    .iter()
                    .any(|s| &s.id == id && s.kind == SessionKind::Editor)
            })
    }
    pub(crate) fn editor_close_busy(&self, ids: &[String]) -> bool {
        ids.iter().any(|id| self.editor_close_sessions.contains(id))
    }

    pub(crate) fn editor_close_prompted(&self, ids: &[String]) -> bool {
        self.editor_close_prompts
            .iter()
            .any(|(_, prompted, _)| prompted.iter().any(|id| ids.contains(id)))
    }

    pub(crate) fn skip_editor_close_request(&self, ids: &[String]) -> bool {
        self.editor_close_busy(ids) || self.editor_close_prompted(ids)
    }

    pub(crate) fn unsaved_close_prompt(
        &self,
        sid: &str,
    ) -> Option<(editor_close::Target, Vec<String>, String)> {
        self.editor_close_prompts
            .iter()
            .find(|(_, ids, _)| ids.iter().any(|id| id == sid))
            .cloned()
    }

    pub(crate) fn upsert_unsaved_close(
        &mut self,
        target: editor_close::Target,
        ids: Vec<String>,
        error: String,
    ) {
        if let Some(prompt) = self
            .editor_close_prompts
            .iter_mut()
            .find(|(_, prompted, _)| prompted == &ids || prompted.iter().any(|id| ids.contains(id)))
        {
            *prompt = (target, ids, error);
            return;
        }
        self.editor_close_prompts.push((target, ids, error));
    }

    pub(crate) fn apply_unsaved_close_choice(
        &mut self,
        choice: appearance::UnsavedCloseChoice,
        target: editor_close::Target,
        ids: Vec<String>,
    ) {
        match choice {
            appearance::UnsavedCloseChoice::Cancel => {
                self.editor_close_prompts
                    .retain(|(_, prompted, _)| prompted != &ids);
                if matches!(target, editor_close::Target::Workspace(..)) {
                    self.abort_workspace_close();
                }
            }
            appearance::UnsavedCloseChoice::Save => {
                self.close_editors(target, ids, editor_close::Mode::Save);
            }
            appearance::UnsavedCloseChoice::Discard => {
                self.close_editors(target, ids, editor_close::Mode::Discard);
            }
        }
    }

    pub(crate) fn editors_closed(
        &mut self,
        target: editor_close::Target,
        ids: Vec<String>,
        result: Result<(), String>,
    ) {
        for id in &ids {
            self.editor_close_sessions.remove(id);
        }
        match result {
            Ok(()) => {
                self.editor_close_prompts
                    .retain(|(_, prompted, _)| !prompted.iter().any(|id| ids.contains(id)));
                match target {
                    editor_close::Target::Workspace(project, id) => {
                        self.close_workspace_tab_now(&project, &id);
                    }
                    editor_close::Target::Pane(sid) => self.remove_tab(&sid),
                }
            }
            Err(error) => self.upsert_unsaved_close(target, ids, error),
        }
    }

    pub(crate) fn close_editors(
        &mut self,
        target: editor_close::Target,
        ids: Vec<String>,
        mode: editor_close::Mode,
    ) {
        if self.editor_close_busy(&ids) {
            return;
        }
        self.editor_close_sessions.extend(ids.iter().cloned());
        let timeout = Duration::from_secs(self.state.settings.editor_close_timeout_secs);
        let _ = self
            .jobs
            .send(Job::CloseEditors(target, ids, mode, timeout));
    }
}
