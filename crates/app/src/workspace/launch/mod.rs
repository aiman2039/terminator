//! Launch layouts: "Open launch layout" (Find Action, project menu) reads
//! the project's `.terminator/layout.toml` on a worker thread and, after an
//! explicit Open click, creates new terminals in new top-level tabs with
//! the file's splits, folders and commands. It never closes, stops or
//! restarts a live session. The file comes from the repository, so the
//! project trust answer (shared with tasks and language servers) comes
//! first. "Save layout as launch file" writes the project's tabs.
mod model;
mod run;
mod save;
#[cfg(test)]
mod tests;

#[cfg(test)]
use model::Split;
use model::{LAUNCH_FILE, LaunchFile};
pub(crate) use run::LaunchRun;

use crate::*;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

/// What a read of the launch file found.
#[derive(Clone, Debug)]
pub(crate) struct Loaded {
    pub(crate) project: String,
    pub(crate) root: PathBuf,
    pub(crate) file: Result<LaunchFile, String>,
}

/// The launch window, a read or save in flight, and the run in progress.
#[derive(Default)]
pub(crate) struct LaunchState {
    open: bool,
    loading: Option<Receiver<Loaded>>,
    loaded: Option<Loaded>,
    saving: Option<Receiver<save::Saved>>,
    /// A save that found an existing file: Replace writes it anyway.
    replace: Option<(PathBuf, String)>,
    pub(crate) run: Option<LaunchRun>,
}

impl App {
    /// Open the launch window for the selected local project and read its
    /// launch file in the background. Nothing opens before the Open click.
    pub(crate) fn open_launch_layout(&mut self) {
        let Some(project) = self.selected_project() else {
            self.info = Some("Select a local project, then open its launch layout.".into());
            return;
        };
        let (id, root) = (project.id.clone(), project.path.clone());
        let (sender, receiver) = channel();
        let spawned = std::thread::Builder::new()
            .name("launch-file".into())
            .spawn(move || {
                let file = std::fs::read_to_string(root.join(LAUNCH_FILE))
                    .map_err(|e| format!("{LAUNCH_FILE}: {e}"))
                    .and_then(|text| model::parse(&text));
                let _ = sender.send(Loaded {
                    project: id,
                    root,
                    file,
                });
            });
        if spawned.is_err() {
            self.info = Some("Could not read the launch file.".into());
            return;
        }
        self.launch.loading = Some(receiver);
        self.launch.loaded = None;
        self.launch.open = true;
    }

    /// Project menu entries: Open launch layout and Save layout as launch
    /// file.
    pub(crate) fn launch_layout_items(&mut self, ui: &mut egui::Ui, project: &str) {
        for (action, run) in [
            (
                Action::OpenLaunchLayout,
                Self::open_launch_layout as fn(&mut Self),
            ),
            (Action::SaveLaunchLayout, Self::save_launch_layout),
        ] {
            if appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: action.label(),
                icon: action.icon(),
                shortcut: "",
            })
            .clicked()
            {
                self.select_project(project.to_owned());
                run(self);
                ui.close();
            }
        }
    }

    /// Record a trust answer as the editor's trust bar and tasks do.
    fn answer_launch_trust(&mut self, root: &std::path::Path, trusted: bool) {
        self.lsp.set_trust(root, trusted);
        self.preferences
            .lsp_trust
            .insert(root.to_path_buf(), trusted);
    }

    /// The launch window, the run in progress and a finished save.
    pub(crate) fn launch_dialog(&mut self, ctx: &egui::Context) {
        self.advance_launch();
        self.finish_launch_save();
        if !self.launch.open {
            return;
        }
        if let Some(receiver) = &self.launch.loading
            && let Ok(loaded) = receiver.try_recv()
        {
            self.launch.loading = None;
            self.launch.loaded = Some(loaded);
        }
        let mut open = true;
        let mut answer = None;
        let mut start = false;
        let mut replace = None;
        self.popups
            .window(ctx, "Open launch layout")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                replace = self.launch.replace.as_ref().and_then(|_| replace_bar(ui));
                let Some(loaded) = &self.launch.loaded else {
                    if self.launch.loading.is_some() {
                        ui.weak("Reading the launch file…");
                    }
                    return;
                };
                let trust = self.lsp.trust_answer(&loaded.root);
                (answer, start) = launch_body(ui, loaded, trust);
            });
        if self.launch.loading.is_some() {
            ctx.request_repaint();
        }
        self.apply_launch_answers(answer, start, replace);
        if start {
            open = false;
        }
        self.launch.open = open;
    }

    fn apply_launch_answers(&mut self, answer: Option<bool>, start: bool, replace: Option<bool>) {
        let loaded = self.launch.loaded.clone();
        if let (Some(trusted), Some(loaded)) = (answer, &loaded) {
            self.answer_launch_trust(&loaded.root, trusted);
        }
        if start
            && let Some(Loaded {
                project,
                root,
                file: Ok(file),
            }) = loaded
        {
            self.start_launch(&project, &root, &file);
        }
        match replace {
            Some(true) => self.replace_launch_file(),
            Some(false) => self.launch.replace = None,
            None => {}
        }
    }
}

/// The trust question, the tabs to open, and Open. Returns the trust
/// answer and whether Open was clicked.
fn launch_body(ui: &mut egui::Ui, loaded: &Loaded, trust: Option<bool>) -> (Option<bool>, bool) {
    let file = match &loaded.file {
        Ok(file) => file,
        Err(error) => {
            ui.colored_label(ui.visuals().error_fg_color, error);
            settings_controls::description_line(
                ui,
                "Add [[tab]] and [[tab.pane]] entries (cwd, command, split = \"right\" or \"down\") \
                 to .terminator/layout.toml, or use Save layout as launch file.",
            );
            return (None, false);
        }
    };
    let mut answer = None;
    if trust != Some(true) {
        settings_controls::description_line(
            ui,
            if trust == Some(false) {
                "This project is not trusted, so its launch file stays off."
            } else {
                "Trust this project? Its .terminator/layout.toml opens terminals and types commands."
            },
        );
        ui.horizontal(|ui| {
            if ui.button("Trust project").clicked() {
                answer = Some(true);
            } else if trust.is_none() && ui.button("Don't trust").clicked() {
                answer = Some(false);
            }
        });
    }
    for (index, tab) in file.tabs.iter().enumerate() {
        let name = workspace::group_display_name(tab.name.as_deref(), index);
        ui.strong(format!("{name}: {} panes", tab.panes.len()));
        for pane in &tab.panes {
            let folder = pane.cwd.as_deref().unwrap_or(".");
            let command = pane.command.as_deref().unwrap_or("(shell)");
            ui.weak(format!("{folder}  {command}"));
        }
    }
    settings_controls::description_line(
        ui,
        "Opens new terminals only. Agent commands are typed, never sent.",
    );
    let open = ui
        .add_enabled(trust == Some(true), egui::Button::new("Open"))
        .on_disabled_hover_text("Trust the project first")
        .clicked();
    (answer, open)
}

/// Replace or keep an existing launch file after Save.
fn replace_bar(ui: &mut egui::Ui) -> Option<bool> {
    settings_controls::description_line(ui, "The project already has a launch file.");
    ui.horizontal(|ui| {
        if ui.button("Replace it").clicked() {
            Some(true)
        } else if ui.button("Keep it").clicked() {
            Some(false)
        } else {
            None
        }
    })
    .inner
}
