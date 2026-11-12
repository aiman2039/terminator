//! Tasks: named shell commands for a project, run only on an explicit
//! Run (Find Action "Run task…", the project menu) or Rerun last task.
//! Each run opens a new terminal in the project and types the command;
//! see [`model::plan`] for when it also presses Enter. Task files are
//! read on a worker thread. Project task files wait for the project
//! trust answer that language servers use (`lsp_manager/trust.rs`).
mod model;
#[cfg(test)]
mod tests;

pub(crate) use model::{LoadedTasks, RunPlan, TaskDef, TaskSource};

use crate::*;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

/// The Run task window and the last run.
#[derive(Default)]
pub(crate) struct TaskState {
    open: bool,
    loading: Option<Receiver<LoadedTasks>>,
    loaded: Option<LoadedTasks>,
    /// Project, project folder and task of the last run.
    last: Option<(String, PathBuf, TaskDef)>,
}

impl App {
    /// The user task file: `tasks.toml` next to `config.toml`.
    fn user_task_file(&self) -> Option<PathBuf> {
        let config = terminator_core::appearance::config_path(&self.paths).ok()?;
        Some(config.parent()?.join(model::USER_TASK_FILE))
    }

    /// Open the Run task window for the selected local project and read
    /// its task files in the background.
    pub(crate) fn open_task_picker(&mut self) {
        let Some(project) = self.selected_project() else {
            self.info = Some("Select a local project, then run a task.".into());
            return;
        };
        let (id, root) = (project.id.clone(), project.path.clone());
        let user_file = self.user_task_file();
        let (sender, receiver) = channel();
        let spawned = std::thread::Builder::new()
            .name("task-files".into())
            .spawn(move || {
                let _ = sender.send(model::load_tasks(id, root, user_file));
            });
        if spawned.is_err() {
            self.info = Some("Could not read the task files.".into());
            return;
        }
        self.tasks.loading = Some(receiver);
        self.tasks.loaded = None;
        self.tasks.open = true;
    }

    /// Run the last task again, or open the picker when none ran yet.
    pub(crate) fn rerun_last_task(&mut self) {
        match self.tasks.last.clone() {
            Some((project, root, task)) => self.run_task(&project, &root, &task),
            None => self.open_task_picker(),
        }
    }

    /// Saved trust answer for a project folder.
    fn task_trust(&self, root: &Path) -> Option<bool> {
        self.lsp.trust_answer(root)
    }

    /// Run one task on an explicit user action. A project task without
    /// the trust answer types nothing.
    pub(crate) fn run_task(&mut self, project: &str, root: &Path, task: &TaskDef) {
        let words = model::agent_words(&self.preferences.agent_presets);
        let post = match model::plan(task, self.task_trust(root), &words) {
            RunPlan::NeedsTrust => {
                self.info = Some("Trust the project before running its tasks.".into());
                return;
            }
            RunPlan::NotTrusted => {
                self.info = Some("This project is not trusted; its tasks stay off.".into());
                return;
            }
            RunPlan::Type(post) => post,
        };
        if !self.project_available(project) {
            self.info = Some("The task's project is not open.".into());
            return;
        }
        if self.selected.as_deref() != Some(project) {
            self.select_project(project.to_owned());
        }
        let cwd = task.cwd.as_ref().map(|cwd| root.join(cwd));
        self.create_workspace_tab_with(None, cwd, Some(post));
        self.tasks.last = Some((project.to_owned(), root.to_path_buf(), task.clone()));
    }

    /// Record a trust answer for a project folder, as the editor's trust
    /// bar does, so language servers and tasks share it.
    fn answer_task_trust(&mut self, root: &Path, trusted: bool) {
        self.lsp.set_trust(root, trusted);
        self.preferences
            .lsp_trust
            .insert(root.to_path_buf(), trusted);
    }

    /// The Run task window.
    pub(crate) fn task_dialog(&mut self, ctx: &egui::Context) {
        if !self.tasks.open {
            return;
        }
        if let Some(receiver) = &self.tasks.loading
            && let Ok(loaded) = receiver.try_recv()
        {
            self.tasks.loading = None;
            self.tasks.loaded = Some(loaded);
        }
        let mut open = true;
        let mut run = None;
        let mut trust = None;
        self.popups
            .window(ctx, "Run task")
            .open(&mut open)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                let Some(loaded) = &self.tasks.loaded else {
                    ui.weak("Reading task files…");
                    return;
                };
                let answer = self.lsp.trust_answer(&loaded.root);
                trust = task_trust_bar(ui, loaded, answer);
                run = task_rows(ui, loaded, answer);
            });
        if self.tasks.loading.is_some() {
            ctx.request_repaint();
        }
        let loaded = self.tasks.loaded.clone().unwrap_or_default();
        if let Some(trusted) = trust {
            self.answer_task_trust(&loaded.root, trusted);
        }
        if let Some(task) = run.and_then(|index| loaded.tasks.get(index)) {
            self.run_task(&loaded.project, &loaded.root, task);
            open = false;
        }
        self.tasks.open = open;
    }
}

/// Trust question for a project task file. Returns the clicked answer.
fn task_trust_bar(ui: &mut egui::Ui, loaded: &LoadedTasks, answer: Option<bool>) -> Option<bool> {
    let has_project = loaded
        .tasks
        .iter()
        .any(|task| task.source == TaskSource::Project);
    if !has_project || answer == Some(true) {
        return None;
    }
    let text = if answer == Some(false) {
        "This project is not trusted, so its own tasks stay off."
    } else {
        "Trust this project? Its .terminator/tasks.toml commands run in your shell."
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

/// One row per task with its Run button. Returns the clicked row.
fn task_rows(ui: &mut egui::Ui, loaded: &LoadedTasks, answer: Option<bool>) -> Option<usize> {
    for error in &loaded.errors {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    if loaded.tasks.is_empty() {
        settings_controls::description_line(
            ui,
            "No tasks. Add [[task]] entries (name, command) to .terminator/tasks.toml in the \
             project, or to tasks.toml next to config.toml for every project.",
        );
        return None;
    }
    let mut clicked = None;
    egui::Grid::new("run-task-rows")
        .num_columns(3)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            for (index, task) in loaded.tasks.iter().enumerate() {
                let allowed = task.source == TaskSource::User || answer == Some(true);
                let button = ui
                    .add_enabled(allowed, egui::Button::new("Run"))
                    .on_disabled_hover_text("Trust the project first");
                if button.clicked() {
                    clicked = Some(index);
                }
                ui.vertical(|ui| {
                    ui.strong(&task.name);
                    ui.weak(&task.command);
                });
                ui.weak(task.source.label());
                ui.end_row();
            }
        });
    clicked
}
