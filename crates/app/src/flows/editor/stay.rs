//! After Run in a flow editor, the app stays on that editor's tab, because
//! its cards show the run. GUI memory only.
use crate::document_view::DocumentKind;
use crate::*;
use std::path::{Path, PathBuf};

/// An editor that keeps the app on its tab during the run that its Run
/// button started.
pub(crate) struct Stay {
    pub(super) path: PathBuf,
    /// The line that the runner's terminal types.
    runner: String,
    /// The runner's terminal did not take the focus yet.
    pub(super) pending: bool,
    /// When Run was clicked or, later, when the runner line was last seen
    /// waiting: `pending` counts only a short time after that.
    since: Instant,
    /// Step terminals of the runs of this editor's flow (project and flow
    /// file) at the last time the editor took the focus back. `None`
    /// until the first check after Run, which counts them.
    pub(super) sessions: Option<usize>,
    /// When another tab took the focus with no new step terminal.
    pub(super) away: Option<Instant>,
}

pub(super) const STAY_GRACE: Duration = Duration::from_secs(3);
/// How long the wait for the runner's terminal lasts after Run.
const STAY_PENDING: Duration = Duration::from_secs(10);

/// The number of step terminals in the runs of flow file `file` of
/// `project`. A run from an older build (no file) does not count.
pub(super) fn step_sessions(runs: &[crate::flows::Run], project: &str, file: &str) -> usize {
    runs.iter()
        .filter(|run| run.of_flow(project, file, None))
        .flat_map(|run| run.state.steps.values())
        .filter(|step| step.session.is_some())
        .count()
}

impl Stay {
    /// After Run in the editor of `path`. The step terminals of its flow
    /// are counted at the first check, in this frame, with the project of
    /// `path` (the runs then are the same as at the click).
    pub(super) fn after_run(path: &Path, runner: String, _runs: &[crate::flows::Run]) -> Self {
        Self {
            path: path.into(),
            runner,
            pending: true,
            since: Instant::now(),
            sessions: None,
            away: None,
        }
    }
}

impl App {
    /// The step terminals of the runs of the flow file at `path`, in the
    /// project whose folder holds it.
    fn flow_step_sessions(&self, path: &Path) -> usize {
        let Some(file) = path.file_stem().and_then(|stem| stem.to_str()) else {
            return 0;
        };
        // `ROOT/.terminator/flows/FILE.toml`.
        let root = path.ancestors().nth(3);
        self.state
            .projects
            .iter()
            .find(|project| Some(project.path.as_path()) == root)
            .map_or(0, |project| {
                step_sessions(&self.flows.runs, &project.id, file)
            })
    }

    /// Makes the workspace tab with the editor of `path` the active one.
    /// False when the selected project has no such tab.
    fn show_flow_tab(&mut self, path: &Path) -> bool {
        let Some(workspace) = self
            .selected
            .clone()
            .and_then(|project| self.layouts.get_mut(&project))
        else {
            return false;
        };
        let editor = |tab: &Tab| matches!(tab, Tab::Document { path: at, kind: DocumentKind::Flow } if at == path);
        let Some(id) = workspace
            .tabs
            .iter()
            .find(|group| group.layout.iter_all_tabs().any(|(_, tab)| editor(tab)))
            .map(|group| group.id.clone())
        else {
            return false;
        };
        workspace.active = id;
        workspace.record_active();
        self.hide_center_overlay();
        self.focus_non_terminal();
        true
    }

    /// After Run in an editor: the app stays on that editor's tab. The
    /// runner's terminal and each step's terminal take the focus when they
    /// start; this gives it back to the editor, which shows the run. When
    /// the user goes to another tab, the editor does not take it back.
    ///
    /// The runner line is typed only while the runner's pane paints, once
    /// its shell shows a prompt. Until then the editor takes nothing back:
    /// a hidden runner tab would keep the line, and the run would not start.
    pub(super) fn flow_stay_on_editor(&mut self) {
        let Some((waiting, sessions)) = self.flows.stay.as_ref().map(|stay| {
            let waiting = stay.pending && self.runner_line_queued(&stay.runner);
            (waiting, self.flow_step_sessions(&stay.path))
        }) else {
            return;
        };
        let Some(stay) = &mut self.flows.stay else {
            return;
        };
        let before = *stay.sessions.get_or_insert(sessions);
        if waiting {
            stay.since = Instant::now();
            stay.away = None;
            return;
        }
        if self.flows.visible.contains(&stay.path) {
            stay.away = None;
            return;
        }
        let pending = stay.pending && stay.since.elapsed() < STAY_PENDING;
        if pending || sessions > before {
            let path = stay.path.clone();
            if self.show_flow_tab(&path) {
                if let Some(stay) = &mut self.flows.stay {
                    stay.pending = false;
                    stay.sessions = Some(sessions);
                    stay.away = None;
                }
                return;
            }
            // The editor's tab is not there yet (it opens with the run).
        }
        let Some(stay) = &mut self.flows.stay else {
            return;
        };
        // The read of a new step's session can come a moment after its
        // terminal took the focus.
        let away = *stay.away.get_or_insert_with(Instant::now);
        if away.elapsed() > STAY_GRACE {
            self.flows.stay = None;
        }
    }
}
