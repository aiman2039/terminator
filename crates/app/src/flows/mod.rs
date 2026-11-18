//! Flows: steps for agents, with an order and human gates
//! (`.terminator/flows/NAME.toml`, see `terminator_core::flow`).
//!
//! The GUI never runs a step itself. Run flow… opens a new terminal and
//! types `terminator-hook flow run` with Enter, only on an explicit Run
//! and only for a trusted project; the runner in that terminal does the
//! work and continues when the GUI closes. The Flows tool window shows the
//! runs from their state files and writes the user's answers (approve,
//! reject, answer, cancel) with the same code as the CLI. Files are read
//! and written on worker threads.
mod panel;
#[cfg(test)]
mod tests;

use crate::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use terminator_core::flow::{self, FLOW_DIR, RunState};

/// One flow file of a project: its name and step count, or why it is
/// refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FlowFile {
    /// The file name without `.toml`.
    pub(crate) file: String,
    pub(crate) flow: Result<(String, usize), String>,
}

/// The flow files of one project.
#[derive(Clone, Debug, Default)]
pub(crate) struct LoadedFlows {
    pub(crate) project: String,
    pub(crate) root: PathBuf,
    pub(crate) files: Vec<FlowFile>,
}

/// One run with its folder.
pub(crate) struct Run {
    pub(crate) dir: PathBuf,
    pub(crate) state: RunState,
}

/// The Run flow window and the Flows tool window.
#[derive(Default)]
pub(crate) struct FlowsState {
    open: bool,
    loading: Option<Receiver<LoadedFlows>>,
    loaded: Option<LoadedFlows>,
    /// The runs of the last read, newest first.
    runs: Vec<Run>,
    reading: Option<Receiver<Vec<Run>>>,
    read_at: Option<Instant>,
    /// Answers that a worker thread writes now.
    acting: Vec<Receiver<Result<(), String>>>,
    /// Typed answers to open questions, by run folder and step.
    drafts: HashMap<(PathBuf, String), String>,
    /// The run whose Cancel waits for confirmation.
    confirm_cancel: Option<PathBuf>,
}

/// A flow file name that is safe as one shell word: the runner accepts
/// only these.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// Read and check every flow file of a project. Blocking: call it on a
/// worker thread. A missing folder gives no files.
pub(crate) fn load_flows(project: String, root: PathBuf) -> LoadedFlows {
    let mut names: Vec<String> = std::fs::read_dir(root.join(FLOW_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .filter_map(|path| Some(path.file_stem()?.to_string_lossy().into_owned()))
        .collect();
    names.sort_unstable();
    let files = names
        .into_iter()
        .map(|file| {
            let flow = if plain_name(&file) {
                std::fs::read_to_string(root.join(FLOW_DIR).join(format!("{file}.toml")))
                    .map_err(|error| error.to_string())
                    .and_then(|text| flow::parse(&text).map_err(|error| error.to_string()))
                    .map(|flow| (flow.name, flow.steps.len()))
            } else {
                Err("The file name must be letters, digits, '-' or '_'".to_owned())
            };
            FlowFile { file, flow }
        })
        .collect();
    LoadedFlows {
        project,
        root,
        files,
    }
}

/// The line typed in the new terminal: the helper beside this program,
/// the project ID and the flow file name, each as one shell word.
pub(crate) fn run_command(helper: &std::path::Path, project: &str, file: &str) -> String {
    format!(
        "{} flow run {} {}",
        terminator_core::quote(&helper.to_string_lossy()),
        terminator_core::quote(project),
        terminator_core::quote(file)
    )
}

impl App {
    /// Open the Run flow window for the selected local project and read
    /// its flow files in the background.
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

    /// The Flows rows of the workspace new-tab menu: Run flow… and the
    /// Flows tool window.
    pub(crate) fn flow_menu_items(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.weak("Flows");
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: Action::RunFlow.label(),
            icon: Action::RunFlow.icon(),
            shortcut: &self.shortcut_label(Action::RunFlow),
        })
        .on_hover_text("Start a flow of this project's .terminator/flows folder")
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
    /// The Flows tool window then shows, so the run is in view.
    pub(crate) fn run_flow(&mut self, loaded: &LoadedFlows, file: &str) {
        match self.lsp.trust_answer(&loaded.root) {
            Some(true) => {}
            Some(false) => {
                self.info = Some("This project is not trusted; its flows stay off.".into());
                return;
            }
            None => {
                self.info = Some("Trust the project before running its flows.".into());
                return;
            }
        }
        if !plain_name(file) || !self.project_available(&loaded.project) {
            self.info = Some("The flow's project is not open.".into());
            return;
        }
        let Ok(program) = std::env::current_exe() else {
            self.info = Some("Could not find the terminator-hook helper.".into());
            return;
        };
        let helper = terminator_core::sibling_exe(&program, "terminator-hook");
        if self.selected.as_deref() != Some(loaded.project.as_str()) {
            self.select_project(loaded.project.clone());
        }
        let post = PostCreate {
            input: run_command(&helper, &loaded.project, file),
            enter: true,
        };
        self.create_workspace_tab_with(None, Some(loaded.root.clone()), Some(post));
        self.open_flows_tool();
    }

    /// The Run flow window.
    pub(crate) fn flow_dialog(&mut self, ctx: &egui::Context) {
        if !self.flows.open {
            return;
        }
        if let Some(receiver) = &self.flows.loading
            && let Ok(loaded) = receiver.try_recv()
        {
            self.flows.loading = None;
            self.flows.loaded = Some(loaded);
        }
        let mut open = true;
        let mut run = None;
        let mut trust = None;
        self.popups
            .window(ctx, "Run flow")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                let Some(loaded) = &self.flows.loaded else {
                    ui.weak("Reading flow files…");
                    return;
                };
                let answer = self.lsp.trust_answer(&loaded.root);
                trust = flow_trust_bar(ui, loaded, answer);
                run = flow_rows(ui, loaded, answer);
            });
        if self.flows.loading.is_some() {
            ctx.request_repaint();
        }
        let loaded = self.flows.loaded.clone().unwrap_or_default();
        if let Some(trusted) = trust {
            self.answer_task_trust(&loaded.root, trusted);
        }
        if let Some(file) = run.and_then(|index| loaded.files.get(index)) {
            self.run_flow(&loaded, &file.file);
            open = false;
        }
        self.flows.open = open;
    }
}

/// Trust question for a project's flow files. Returns the clicked answer.
fn flow_trust_bar(ui: &mut egui::Ui, loaded: &LoadedFlows, answer: Option<bool>) -> Option<bool> {
    if loaded.files.is_empty() || answer == Some(true) {
        return None;
    }
    let text = if answer == Some(false) {
        "This project is not trusted, so its flows stay off."
    } else {
        "Trust this project? A flow starts its commands and agents in your shell, with Enter."
    };
    settings_controls::description_line(ui, text);
    ui.horizontal(|ui| {
        if ui.button("Trust project").clicked() {
            Some(true)
        } else if answer.is_none() && ui.button("Don't trust").clicked() {
            Some(false)
        } else {
            None
        }
    })
    .inner
}

/// One row per flow file with its Run button. Returns the clicked row.
fn flow_rows(ui: &mut egui::Ui, loaded: &LoadedFlows, answer: Option<bool>) -> Option<usize> {
    if loaded.files.is_empty() {
        settings_controls::description_line(
            ui,
            "No flows. Add a file such as .terminator/flows/build.toml to the project: \
             version = 1, name, and [[step]] entries (id, command or gate, needs).",
        );
        return None;
    }
    let mut clicked = None;
    egui::Grid::new("run-flow-rows")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            for (index, file) in loaded.files.iter().enumerate() {
                let allowed = file.flow.is_ok() && answer == Some(true);
                let why = if file.flow.is_err() {
                    "Correct the flow file first"
                } else {
                    "Trust the project first"
                };
                let button = ui
                    .add_enabled(allowed, egui::Button::new("Run"))
                    .on_disabled_hover_text(why);
                if button.clicked() {
                    clicked = Some(index);
                }
                ui.vertical(|ui| match &file.flow {
                    Ok((name, steps)) => {
                        ui.strong(name);
                        ui.weak(format!("{}.toml, {steps} steps", file.file));
                    }
                    Err(error) => {
                        ui.strong(&file.file);
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                });
                ui.end_row();
            }
        });
    clicked
}
