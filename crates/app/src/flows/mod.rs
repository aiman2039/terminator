//! Flows: steps for agents, with an order and human gates
//! (`.terminator/flows/NAME.toml`, see `terminator_core::flow`).
//!
//! The GUI never runs a step itself. Run flow… opens a new terminal and
//! types `terminator-hook flow run` with Enter, only on an explicit Run
//! and only for a trusted project; the runner in that terminal does the
//! work and continues when the GUI closes. The Flows tool window shows the
//! runs from their state files and writes the user's answers (approve,
//! reject, answer, cancel) with the same code as the CLI. New flow and
//! Edit open the native flow editor (`editor`). Files are read and written
//! on worker threads.
mod editor;
mod gates;
#[cfg(test)]
mod gates_tests;
pub(crate) use editor::{FLOW_ACTIONS, FLOW_TAB_ICON};
mod inputs_form;
mod panel;
mod picker;
#[cfg(test)]
mod retry_tests;
mod runs_view;
mod status;
#[cfg(test)]
mod tests;

use crate::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use terminator_core::flow::{Input, RunState, control, files};

/// One flow file of a project: its name and step count, or why it is
/// refused.
#[derive(Clone, Debug)]
pub(crate) struct FlowFile {
    /// The file name without `.toml`.
    pub(crate) file: String,
    pub(crate) flow: Result<(String, usize), String>,
    /// The flow's run inputs (none for a refused file): Run asks for them.
    pub(crate) inputs: Vec<(String, Input)>,
}

/// The flow files of one project.
#[derive(Clone, Debug, Default)]
pub(crate) struct LoadedFlows {
    pub(crate) project: String,
    pub(crate) root: PathBuf,
    pub(crate) files: Vec<FlowFile>,
}

/// One run with its folder.
#[derive(Clone)]
pub(crate) struct Run {
    pub(crate) dir: PathBuf,
    pub(crate) state: RunState,
    /// The run did not end, but its runner is gone (its terminal closed,
    /// the process was killed, or the computer restarted): nothing
    /// changes it again. Its steps that did not end show as cancelled.
    pub(crate) gone: bool,
}

impl Run {
    /// A run as the worker thread reads it. Blocking (the runner's lock):
    /// call it off the GUI thread.
    pub(crate) fn read(dir: PathBuf, state: RunState) -> Self {
        // A gone run's open steps show as cancelled: Approve, answers and
        // Cancel go to no runner, so they do not show.
        let view = control::run_view(&dir, state);
        Self {
            gone: view.status == control::RunStatus::RunnerGone,
            dir,
            state: view.state,
        }
    }

    /// The run's status: running, done, stopped or runner gone.
    pub(crate) fn status(&self) -> control::RunStatus {
        control::RunStatus::of(&self.state, !self.gone)
    }

    /// Ended, or its runner is gone: Remove and Run again apply.
    pub(crate) fn ended(&self) -> bool {
        self.status().ended()
    }

    /// True for a run of the flow file `file` of `project`.
    pub(crate) fn of_flow(&self, project: &str, file: &str) -> bool {
        self.state.project == project && self.state.file == file
    }
}

/// The Run flow window and the Flows tool window.
#[derive(Default)]
pub(crate) struct FlowsState {
    open: bool,
    loading: Option<Receiver<LoadedFlows>>,
    loaded: Option<LoadedFlows>,
    /// The typed name of a new flow.
    new_name: String,
    /// The flow editors by flow file path: one per editor tab, and each
    /// flow with changes that are not saved (also when its tab is closed).
    pub(crate) editors: HashMap<PathBuf, editor::Slot>,
    /// Where the current quit is with editors whose content is not saved.
    quit: editor::Quit,
    /// The editors that a pane painted this frame.
    visible: std::collections::HashSet<PathBuf>,
    /// The editor whose Run keeps the app on its tab.
    stay: Option<editor::Stay>,
    /// The editor whose Run was clicked a moment ago: its banner says
    /// "Starting" until the new run shows.
    starting: Option<editor::Starting>,
    /// Ended runs whose banner the user closed, by run folder.
    banner_hidden: std::collections::HashSet<PathBuf>,
    /// The runs of the last read, newest first.
    runs: Vec<Run>,
    reading: Option<Receiver<Vec<Run>>>,
    read_at: Option<Instant>,
    /// Older ended run folders that the reads skip (one read at a time).
    ended_runs: std::sync::Arc<std::sync::Mutex<control::EndedRuns>>,
    /// Answers that a worker thread writes now.
    acting: Vec<Receiver<Result<(), String>>>,
    /// Typed answers to open questions, by run folder and step.
    drafts: HashMap<(PathBuf, String), String>,
    /// The run whose Cancel waits for confirmation.
    confirm_cancel: Option<PathBuf>,
    /// Clear ended runs waits for confirmation.
    confirm_clear: bool,
    /// Runs whose steps the user showed (true) or hid, by run folder.
    expanded: HashMap<PathBuf, bool>,
    /// The flow files of the selected project for the Flows tool window,
    /// read on a worker thread while it shows.
    tool_flows: Option<LoadedFlows>,
    tool_loading: Option<Receiver<LoadedFlows>>,
    tool_read_at: Option<Instant>,
    /// Where the Run flow window moves next frame, so its list fits.
    picker_top: Option<egui::Pos2>,
    /// The Run inputs form of a flow with inputs, after a Run click.
    inputs_form: Option<inputs_form::InputsForm>,
}

impl FlowsState {
    pub(crate) fn begin_frame(&mut self) {
        self.visible.clear();
    }

    /// A pane shows the editor of `path` this frame.
    pub(crate) fn mark_visible(&mut self, path: &std::path::Path) {
        self.visible.insert(path.into());
    }
}

/// Read and check every flow file of a project. Blocking: call it on a
/// worker thread. A missing folder gives no files.
pub(crate) fn load_flows(project: String, root: PathBuf) -> LoadedFlows {
    let files = files::names(&root)
        .into_iter()
        .map(|file| {
            let loaded = files::load(&root, &file);
            let inputs = loaded
                .as_ref()
                .map(|flow| flow.inputs.clone().into_iter().collect())
                .unwrap_or_default();
            FlowFile {
                flow: loaded
                    .map(|flow| (flow.name, flow.steps.len()))
                    .map_err(|error| error.to_string()),
                inputs,
                file,
            }
        })
        .collect();
    LoadedFlows {
        project,
        root,
        files,
    }
}

/// The line typed in the new terminal: the helper beside this program,
/// the project ID, the flow file name and one `--set NAME=VALUE` per input
/// value, each as one shell word.
pub(crate) fn run_command(
    helper: &std::path::Path,
    (project, file): (&str, &str),
    values: &[(String, String)],
) -> String {
    let mut line = format!(
        "{} flow run {} {}",
        terminator_core::quote(&helper.to_string_lossy()),
        terminator_core::quote(project),
        terminator_core::quote(file)
    );
    for (name, value) in values {
        let set = terminator_core::quote(&format!("{name}={value}"));
        line.push_str(&format!(" --set {set}"));
    }
    line
}

impl App {
    /// Open the Flows window for the selected local project and read its
    /// flow files in the background.
    pub(crate) fn open_flow_picker(&mut self) {
        let Some(project) = self.selected_project() else {
            self.info = Some("Select a local project, then run a flow.".into());
            return;
        };
        let (id, root) = (project.id.clone(), project.path.clone());
        let (sender, receiver) = channel();
        let spawned = std::thread::Builder::new()
            .name("flow-files".into())
            .spawn(move || {
                let _ = sender.send(load_flows(id, root));
            });
        if spawned.is_err() {
            self.info = Some("Could not read the flow files.".into());
            return;
        }
        self.flows.loading = Some(receiver);
        self.flows.loaded = None;
        self.flows.open = true;
    }

    /// Shows the Flows tool window in the right sidebar. Projects there
    /// take precedence over the tool, so clear them.
    pub(crate) fn open_flows_tool(&mut self) {
        self.preferences.right_projects = false;
        self.preferences.tool = SidebarTool::Flows;
        self.preferences.visible = true;
    }

    /// The Flows rows of the workspace new-tab menu: the Flows window (run,
    /// new, edit) and the Flows tool window (the runs).
    pub(crate) fn flow_menu_items(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.weak("Flows");
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: Action::RunFlow.label(),
            icon: Action::RunFlow.icon(),
            shortcut: &self.shortcut_label(Action::RunFlow),
        })
        .on_hover_text("Run, make or edit a flow of this project's .terminator/flows folder")
        .clicked()
        {
            self.open_flow_picker();
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "New flow…",
            icon: "Pencil",
            shortcut: "",
        })
        .on_hover_text("Name a new flow, then make it in the flow editor")
        .clicked()
        {
            self.open_flow_picker();
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Show flows",
            icon: "PanelRight",
            shortcut: "",
        })
        .on_hover_text("Open the Flows tool window: runs, gates and questions")
        .clicked()
        {
            self.open_flows_tool();
            ui.close();
        }
    }

    /// Run one flow on an explicit Run click: a new terminal with the
    /// runner command and Enter. Without the trust answer nothing is typed.
    /// The Flows tool window then shows, so the run is in view. Gives the
    /// runner line that the new terminal types, or `None` when the run did
    /// not start.
    #[cfg(test)]
    pub(crate) fn run_flow(&mut self, loaded: &LoadedFlows, file: &str) -> Option<String> {
        self.run_flow_with(loaded, file, &[])
    }

    /// [`App::run_flow`] with the run's input values (`--set`). A value with
    /// a control character is refused: it would be a key in the terminal.
    pub(crate) fn run_flow_with(
        &mut self,
        loaded: &LoadedFlows,
        file: &str,
        values: &[(String, String)],
    ) -> Option<String> {
        if values
            .iter()
            .any(|(_, value)| !terminator_core::flow::inputs::value_ok(value))
        {
            self.info = Some("An input value has a control character or is too long.".into());
            return None;
        }
        if self.flow_run_waits_for_save(loaded, file) {
            return None;
        }
        match self.lsp.trust_answer(&loaded.root) {
            Some(true) => {}
            Some(false) => {
                self.info = Some("This project is not trusted; its flows stay off.".into());
                return None;
            }
            None => {
                self.info = Some("Trust the project before running its flows.".into());
                return None;
            }
        }
        if !files::plain_name(file) || !self.project_available(&loaded.project) {
            self.info = Some("The flow's project is not open.".into());
            return None;
        }
        let Ok(program) = std::env::current_exe() else {
            self.info = Some("Could not find the terminator-hook helper.".into());
            return None;
        };
        let helper = terminator_core::sibling_exe(&program, "terminator-hook");
        if self.selected.as_deref() != Some(loaded.project.as_str()) {
            self.select_project(loaded.project.clone());
        }
        let line = run_command(&helper, (&loaded.project, file), values);
        let post = PostCreate {
            input: line.clone(),
            enter: true,
        };
        self.create_workspace_tab_with(None, Some(loaded.root.clone()), Some(post));
        self.open_flows_tool();
        Some(line)
    }

    /// The runner line `line` waits in a new terminal: its tab is not
    /// there yet, or its shell has not shown a prompt. The line is typed
    /// only while its pane paints, so nothing may hide that tab before.
    pub(crate) fn runner_line_queued(&self, line: &str) -> bool {
        let queue = &self.post_create;
        queue
            .by_tab
            .values()
            .chain(queue.by_session.values())
            .any(|post| post.input == line)
    }
}
