//! Child sessions (`ctl spawn`, `session-parent-v1`) under their parent's
//! row. Children in the same project draw indented below the parent; one
//! rollup line counts the hook-reported state of every live child, in any
//! project. Links are display data: rows never start or stop a session.
use super::*;
use std::collections::BTreeSet;
use terminator_core::State;

/// Deepest nesting drawn under one top-level row.
const MAX_DEPTH: usize = 4;

/// IDs of `live` sessions whose parent is also in `live`: they draw under
/// that parent instead of in their own place.
pub(super) fn nested_ids(state: &State, live: &[Arc<Session>]) -> BTreeSet<String> {
    if state.session_parents.is_empty() {
        return BTreeSet::new();
    }
    let here: BTreeSet<&str> = live.iter().map(|s| s.id.as_str()).collect();
    live.iter()
        .filter(|s| {
            state
                .session_parents
                .get(&s.id)
                .is_some_and(|parent| here.contains(parent.as_str()))
        })
        .map(|s| s.id.clone())
        .collect()
}

/// `2 child sessions · 1 waiting · 1 working`.
pub(crate) fn rollup_text(children: usize, counts: crate::agent_presence::StateCounts) -> String {
    let noun = if children == 1 {
        "child session"
    } else {
        "child sessions"
    };
    let mut text = format!("{children} {noun}");
    for (label, count) in [
        ("waiting", counts.waiting),
        ("failed", counts.failed),
        ("working", counts.working),
        ("done", counts.done),
    ] {
        if count > 0 {
            text.push_str(&format!(" · {count} {label}"));
        }
    }
    text
}

impl App {
    /// Draws the children of `parent` (same project: indented rows, then
    /// their own children) and the rollup line.
    pub(super) fn session_children(&mut self, ui: &mut egui::Ui, project_id: &str, parent: &str) {
        self.session_children_at(ui, (project_id, parent), 0);
    }

    /// `(project, parent)`: the project tree being drawn and the parent row.
    fn session_children_at(
        &mut self,
        ui: &mut egui::Ui,
        (project_id, parent): (&str, &str),
        depth: usize,
    ) {
        if depth >= MAX_DEPTH || !self.state.session_parents.values().any(|p| p == parent) {
            return;
        }
        let children: Vec<Session> =
            terminator_core::session_parents::live_children(&self.state, parent)
                .into_iter()
                .cloned()
                .collect();
        if children.is_empty() {
            return;
        }
        let counts = crate::agent_presence::StateCounts::of(
            children
                .iter()
                .filter_map(|s| self.present_session(&s.id).lifecycle),
        );
        ui.indent(format!("sidebar-children:{parent}"), |ui| {
            ui.weak(rollup_text(children.len(), counts))
                .on_hover_text("Sessions started with `terminator-hook ctl spawn` from this one");
            for child in children.iter().filter(|s| s.project_id == project_id) {
                self.session_row(ui, child);
                self.session_children_at(ui, (project_id, &child.id), depth.saturating_add(1));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_presence::StateCounts;

    #[test]
    fn rollup_counts_children_by_state() {
        let counts = StateCounts::of([AgentState::WaitingInput, AgentState::Running]);
        assert_eq!(
            rollup_text(2, counts),
            "2 child sessions · 1 waiting · 1 working"
        );
        assert_eq!(rollup_text(1, StateCounts::default()), "1 child session");
    }

    fn session(id: &str) -> Arc<Session> {
        Arc::new(Session {
            remote: None,
            review: false,
            id: id.into(),
            project_id: "p".into(),
            label: id.into(),
            cwd: "/p".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle: terminator_core::Lifecycle::Running,
            created: 0,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "g".into(),
            pid: None,
            truncated: false,
            cwd_confirmed: true,
        })
    }

    #[test]
    fn only_children_of_a_parent_in_the_same_list_nest() {
        let mut state = State::default();
        state
            .session_parents
            .insert("child".into(), "parent".into());
        state
            .session_parents
            .insert("orphan".into(), "elsewhere".into());
        let live = [session("parent"), session("child"), session("orphan")];
        let nested = nested_ids(&state, &live);
        assert_eq!(nested.into_iter().collect::<Vec<_>>(), ["child"]);
        assert!(nested_ids(&State::default(), &live).is_empty());
    }
}
