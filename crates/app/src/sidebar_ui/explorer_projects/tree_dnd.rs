//! Drag in the project session tree: drop a session row on a tab group row
//! to move its pane into that tab; drop a tab group row on another to put
//! it at that position. Only inside one project.
use super::*;

/// What a tree row carries while it is dragged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TreeDrag {
    Session { project: String, sid: String },
    Group { project: String, tab: String },
}

impl TreeDrag {
    fn project(&self) -> &str {
        match self {
            Self::Session { project, .. } | Self::Group { project, .. } => project,
        }
    }
}

/// Agent states of a tab group's sessions as "1 waiting · 2 working", or
/// the session count when no agent reports a state.
pub(super) fn group_activity(states: &[Option<AgentState>], count: usize) -> String {
    let count_of = |wanted: &[AgentState]| {
        states
            .iter()
            .filter(|state| state.is_some_and(|state| wanted.contains(&state)))
            .count()
    };
    let waiting = count_of(&[AgentState::WaitingInput, AgentState::WaitingPermission]);
    let waiting = if waiting > 0 {
        format!("{waiting} waiting")
    } else {
        String::new()
    };
    let activity = crate::sidebar_ui::attention::activity_label(
        count_of(&[AgentState::Running]),
        count_of(&[AgentState::Completed]),
    );
    let label = crate::sidebar_ui::attention::join_labels(&waiting, &activity);
    if label.is_empty() {
        count.to_string()
    } else {
        label
    }
}

impl App {
    /// Makes a painted row a drag source; its clicks stay as they were.
    pub(super) fn tree_drag_source(response: &egui::Response, payload: TreeDrag) {
        let response = response.interact(egui::Sense::drag());
        if response.dragged() {
            response.dnd_set_drag_payload(payload);
        }
    }

    /// A session row in the tree: drag source of its session.
    pub(super) fn tree_session_row_dnd(response: &egui::Response, (project, sid): (&str, &str)) {
        Self::tree_drag_source(
            response,
            TreeDrag::Session {
                project: project.to_owned(),
                sid: sid.to_owned(),
            },
        );
    }

    /// Makes a tab group row a drop target for rows of the same project.
    pub(super) fn tree_drop_target(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        (project, tab_id): (&str, &str),
    ) {
        if response
            .dnd_hover_payload::<TreeDrag>()
            .is_some_and(|payload| payload.project() == project)
        {
            ui.painter().rect_stroke(
                response.rect,
                appearance::WIDGET_RADIUS,
                egui::Stroke::new(1.5, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(payload) = response.dnd_release_payload::<TreeDrag>() {
            self.tree_drop(&payload, project, tab_id);
        }
    }

    pub(super) fn tree_drop(&mut self, payload: &TreeDrag, project: &str, tab_id: &str) {
        match payload {
            TreeDrag::Session { project: from, sid } if from == project => {
                self.move_session_to_tab_group(project, sid, tab_id);
            }
            TreeDrag::Group { project: from, tab } if from == project => {
                self.reorder_tab_group(project, tab, tab_id);
            }
            _ => {}
        }
    }

    /// Moves a terminal pane into another top-level tab of its project.
    pub(crate) fn move_session_to_tab_group(&mut self, project: &str, sid: &str, tab_id: &str) {
        let moved = self.layouts.get_mut(project).is_some_and(|workspace| {
            workspace.move_pane_to_group(&Tab::Terminal(sid.to_owned()), tab_id)
        });
        self.after_tree_move(moved);
    }

    /// Puts the tab `tab` at the position of the tab `target`.
    pub(crate) fn reorder_tab_group(&mut self, project: &str, tab: &str, target: &str) {
        let moved = tab != target
            && self.layouts.get_mut(project).is_some_and(|workspace| {
                workspace
                    .tabs
                    .iter()
                    .position(|group| group.id == target)
                    .is_some_and(|index| workspace.reorder_group(tab, index))
            });
        self.after_tree_move(moved);
    }

    fn after_tree_move(&mut self, moved: bool) {
        if moved {
            self.pending_layout_save = true;
            self.pane_index = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_activity_counts_agent_states_or_sessions() {
        let states = [
            Some(AgentState::WaitingPermission),
            Some(AgentState::Running),
            Some(AgentState::Running),
            None,
        ];
        assert_eq!(group_activity(&states, 4), "1 waiting · 2 working");
        assert_eq!(group_activity(&[None, None], 2), "2");
        assert_eq!(group_activity(&[Some(AgentState::Completed)], 1), "1 done");
    }
}
