//! Resume conversation: an explicit click runs the stored resume command.
use crate::*;

impl App {
    /// The stored resume command of an agent in `session`, if any.
    pub(crate) fn resume_command(&self, session: &str) -> Option<String> {
        self.state
            .agents
            .iter()
            .filter(|agent| agent.session_id == session)
            .find_map(|agent| agent.resume.as_ref().map(Resume::display))
    }

    /// Opens a new terminal tab in the ended session's project and folder
    /// and runs `command` there. Only call it for an explicit user click.
    pub(crate) fn resume_conversation(&mut self, session: &Session, command: String) {
        self.hide_center_overlay();
        if self.missing_projects.contains(&session.project_id) {
            return;
        }
        let tab_id = id();
        self.post_create.by_tab.insert(
            tab_id.clone(),
            PostCreate {
                input: command,
                enter: true,
            },
        );
        let _ = self.jobs.send(Job::rpc(
            Request::Create {
                project: session.project_id.clone(),
                cwd: Some(session.cwd.clone()),
                file: None,
                line: None,
                column: None,
                editor: false,
            },
            After::Workspace(tab_id, Vec::new()),
        ));
    }
}
