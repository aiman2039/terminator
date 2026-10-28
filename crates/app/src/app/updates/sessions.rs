//! Session creation results and the focus changes that follow them.

use crate::app::InsertFloatInput;
use crate::*;

pub(super) struct FloatCreatedInput {
    pub(super) session: Session,
    pub(super) viewport: egui::ViewportId,
    pub(super) split: Option<String>,
    pub(super) anchors: Vec<Tab>,
}

impl App {
    pub(super) fn handle_focus_viewport(ctx: &egui::Context, viewport: egui::ViewportId) {
        ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
    }

    pub(super) fn handle_reveal_file_session(
        &mut self,
        ctx: &egui::Context,
        session: String,
        project: String,
    ) {
        if self.selected.as_deref() != Some(project.as_str()) {
            return;
        }
        self.go_session(&session);
        if let Some(viewport) = self.floating.iter().find_map(|window| {
            window
                .dock
                .as_ref()
                .filter(|dock| dock.find_tab(&Tab::Terminal(session.clone())).is_some())
                .map(|_| window.viewport)
        }) {
            Self::handle_focus_viewport(ctx, viewport);
        }
    }

    pub(super) fn handle_opened_project(&mut self, state: State, project: String, generation: u64) {
        self.missing_projects.remove(&project);
        self.preferences.setup_completed = true;
        self.refresh_request = None;
        self.apply_state(state);
        if generation == self.selection_generation {
            self.reveal_project(project);
        } else if let Some(project) = self.selected.clone() {
            // Undo older AddProject selection side effects on the daemon.
            self.send(Request::SelectProject { project });
        }
    }

    pub(super) fn handle_worktree_created(
        &mut self,
        state: State,
        project: String,
        open_terminal: bool,
    ) {
        self.missing_projects.remove(&project);
        self.apply_state(state);
        self.reveal_project(project);
        if open_terminal {
            self.create(None);
        }
    }

    pub(super) fn handle_workspace_created(
        &mut self,
        session: Session,
        id: String,
        anchors: Vec<Tab>,
    ) {
        let project = session.project_id.clone();
        if self.selected.as_ref() == Some(&project) {
            self.finish_rename(true);
        }
        if session.kind == SessionKind::Editor {
            self.editor_origins
                .insert(session.id.clone(), anchors.clone());
        }
        let index = self.workspace_insert.remove(&id).unwrap_or(usize::MAX);
        self.layouts
            .entry(project.clone())
            .or_insert_with(Workspace::empty)
            .add_at_from(crate::workspace::AddAtFromInput {
                index,
                id,
                pane: Tab::Terminal(session.id.clone()),
                anchors: &anchors,
            });
        if self.selected.as_ref() == Some(&project) {
            self.active_session = Some(session.id.clone());
        }
        if !self.state.sessions.iter().any(|s| s.id == session.id) {
            self.state.sessions.push(session);
            self.reconcile_presentations();
        }
    }

    pub(super) fn handle_created(
        &mut self,
        session: Session,
        split: Option<String>,
        target: Option<Vec<Tab>>,
    ) {
        let previous_workspace = self
            .layouts
            .get(&session.project_id)
            .map(|workspace| workspace.active.clone());
        #[cfg(feature = "test-support")]
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
            crate::console::Console::err(format_args!(
                "Fixture created {:?}, split {:?}",
                session.kind, split
            ));
        }
        if session.kind == SessionKind::Editor {
            let anchors = target.clone().unwrap_or_default();
            self.editor_origins.insert(session.id.clone(), anchors);
        }
        if let Some(target) = target {
            self.focus_created_target(&session.project_id, &target);
        }
        let same_workspace = previous_workspace.as_ref()
            == self
                .layouts
                .get(&session.project_id)
                .map(|workspace| &workspace.active);
        if same_workspace && self.selected.as_ref() == Some(&session.project_id) {
            self.active_session = Some(session.id.clone());
        }
        self.track_session(&session);
        self.insert(
            &session.project_id,
            Tab::Terminal(session.id),
            split.as_deref(),
        );
        if !same_workspace
            && let Some(previous) = previous_workspace
            && let Some(workspace) = self.layouts.get_mut(&session.project_id)
            && workspace.tabs.iter().any(|tab| tab.id == previous)
        {
            workspace.active = previous;
        }
    }

    /// Activates the workspace and pane holding the first anchor tab.
    fn focus_created_target(&mut self, project: &str, target: &[Tab]) {
        let Some(dock) = self.layouts.get_mut(project) else {
            return;
        };
        if let Some(tab) = target.iter().find(|tab| dock.contains(tab)) {
            dock.activate_containing(tab);
        }
        if let Some(path) = target.iter().find_map(|tab| dock.find_tab(tab)) {
            dock.set_focused_node_and_surface(path.node_path());
        }
    }

    /// Adds a session the daemon has not reported yet to local state.
    fn track_session(&mut self, session: &Session) {
        if !self.state.sessions.iter().any(|s| s.id == session.id) {
            self.state.sessions.push(session.clone());
            self.reconcile_presentations();
        }
    }

    pub(super) fn handle_strip_created(
        &mut self,
        session: Session,
        split: Option<String>,
        anchors: Vec<Tab>,
    ) {
        self.track_session(&session);
        if let Some(dock) = self
            .preferences
            .ide_strip_docks
            .0
            .get_mut(&session.project_id)
            && let Some(path) = anchors.iter().find_map(|tab| dock.find_tab(tab))
        {
            dock.set_focused_node_and_surface(path.node_path());
        }
        self.insert_strip(
            &session.project_id,
            Tab::Terminal(session.id.clone()),
            split.as_deref(),
        );
        if self.selected.as_ref() == Some(&session.project_id)
            && self.preferences.ide_mode
            && !self.preferences.ide_terminal_collapsed
        {
            self.active_session = Some(session.id.clone());
        }
    }

    pub(super) fn handle_float_created(&mut self, input: FloatCreatedInput) {
        let FloatCreatedInput {
            session,
            viewport,
            split,
            anchors,
        } = input;
        if session.kind == SessionKind::Editor {
            self.editor_origins
                .insert(session.id.clone(), anchors.clone());
        }
        self.track_session(&session);
        // The originating window wins even after focus or
        // project changes. A window closed mid-flight falls
        // back to the project dock so the tab is not lost.
        if self.insert_float(InsertFloatInput {
            viewport,
            tab: Tab::Terminal(session.id.clone()),
            split: split.as_deref(),
            anchors: &anchors,
        }) {
            self.active_session = Some(session.id.clone());
        } else {
            self.insert(
                &session.project_id,
                Tab::Terminal(session.id.clone()),
                split.as_deref(),
            );
        }
    }

    pub(super) fn handle_clipboard_paste(&mut self, session: &str, text: &str) {
        // Resolve by captured identity, never by current focus.
        if let Some(backend) = self.backends.get_mut(session) {
            let mode = backend.last_content().terminal_mode;
            let bytes = egui_term::paste_input(text, mode);
            backend.process_command(egui_term::BackendCommand::Write(bytes));
        }
    }
}
