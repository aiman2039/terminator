//! "Save layout as launch file": the project's top-level tabs, each pane's
//! folder and its split beside the previous pane, written on a worker
//! thread. Commands are not saved: a running program is not known.
use super::model::{self, LaunchFile, LaunchPane, LaunchTab, Split};
use crate::*;
use egui_dock::Node;
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;

/// What a save did.
pub(crate) enum Saved {
    Written(PathBuf),
    /// The file exists; nothing was written.
    Exists(PathBuf, String),
    Failed(String),
}

/// The panes of one dock in reading order, each with its split beside
/// the previous pane: the right child of a horizontal split is "right",
/// of a vertical split "down".
pub(crate) fn dock_panes(dock: &egui_dock::DockState<Tab>) -> Vec<(Option<Split>, Vec<Tab>)> {
    let nodes: Vec<&Node<Tab>> = dock.main_surface().iter().collect();
    let mut out = Vec::new();
    walk(&nodes, 0, None, &mut out);
    out
}

fn walk(
    nodes: &[&Node<Tab>],
    index: usize,
    split: Option<Split>,
    out: &mut Vec<(Option<Split>, Vec<Tab>)>,
) {
    let Some(node) = nodes.get(index) else {
        return;
    };
    let left = index.saturating_mul(2).saturating_add(1);
    let right = left.saturating_add(1);
    match node {
        Node::Leaf(_) => out.push((split, node.tabs().unwrap_or_default().to_vec())),
        Node::Horizontal(_) => {
            walk(nodes, left, split, out);
            walk(nodes, right, Some(Split::Right), out);
        }
        Node::Vertical(_) => {
            walk(nodes, left, split, out);
            walk(nodes, right, Some(Split::Down), out);
        }
        Node::Empty => {}
    }
}

/// A pane folder relative to `root`; `None` for the root itself or a
/// folder outside it.
fn relative(cwd: &Path, root: &Path) -> Option<String> {
    let rest = cwd.strip_prefix(root).ok()?;
    let text = rest.to_string_lossy().into_owned();
    (!text.is_empty()).then_some(text)
}

impl App {
    /// The launch file for `project`'s tabs: one pane per split leaf that
    /// shows a terminal.
    pub(crate) fn launch_file_for(&self, project: &str, root: &Path) -> LaunchFile {
        let mut file = LaunchFile::default();
        let Some(workspace) = self.layouts.get(project) else {
            return file;
        };
        for group in &workspace.tabs {
            let mut tab = LaunchTab {
                name: group.name.clone(),
                panes: Vec::new(),
            };
            for (split, tabs) in dock_panes(&group.layout) {
                let Some(session) = tabs.iter().find_map(|tab| match tab {
                    Tab::Terminal(sid) => self.state.sessions.iter().find(|s| &s.id == sid),
                    _ => None,
                }) else {
                    continue;
                };
                tab.panes.push(LaunchPane {
                    split: (!tab.panes.is_empty()).then(|| split.unwrap_or_default()),
                    cwd: relative(&session.cwd, root),
                    command: None,
                    enter: None,
                });
            }
            if !tab.panes.is_empty() {
                file.tabs.push(tab);
            }
        }
        file
    }

    /// Write the selected project's tabs to its launch file, unless one
    /// exists (the launch window then offers Replace).
    pub(crate) fn save_launch_layout(&mut self) {
        let Some(project) = self.selected_project() else {
            self.info = Some("Select a local project, then save its launch layout.".into());
            return;
        };
        let root = project.path.clone();
        let file = self.launch_file_for(&project.id.clone(), &root);
        if file.tabs.is_empty() {
            self.info = Some("This project has no terminal tabs to save.".into());
            return;
        }
        match model::render(&file) {
            Ok(text) => self.write_launch_file(root.join(model::LAUNCH_FILE), text, false),
            Err(error) => self.info = Some(format!("Could not save the launch file: {error}")),
        }
    }

    pub(super) fn replace_launch_file(&mut self) {
        if let Some((path, text)) = self.launch.replace.take() {
            self.write_launch_file(path, text, true);
        }
    }

    fn write_launch_file(&mut self, path: PathBuf, text: String, replace: bool) {
        let (sender, receiver) = channel();
        let spawned = std::thread::Builder::new()
            .name("launch-save".into())
            .spawn(move || {
                let _ = sender.send(write(&path, text, replace));
            });
        if spawned.is_ok() {
            self.launch.saving = Some(receiver);
        }
    }

    /// Show what a finished save did.
    pub(super) fn finish_launch_save(&mut self) {
        let Some(saved) = self
            .launch
            .saving
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok())
        else {
            return;
        };
        self.launch.saving = None;
        match saved {
            Saved::Written(path) => {
                self.info = Some(format!("Saved {}.", path.display()));
            }
            Saved::Exists(path, text) => {
                self.launch.replace = Some((path, text));
                self.launch.loaded = None;
                self.launch.loading = None;
                self.launch.open = true;
            }
            Saved::Failed(error) => {
                self.info = Some(format!("Could not save the launch file: {error}"));
            }
        }
    }
}

/// Blocking: runs on the save thread.
fn write(path: &Path, text: String, replace: bool) -> Saved {
    if let Some(parent) = path.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return Saved::Failed(error.to_string());
    }
    let result = if replace {
        std::fs::write(path, &text)
    } else {
        use std::io::Write as _;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .and_then(|mut file| file.write_all(text.as_bytes()))
    };
    match result {
        Ok(()) => Saved::Written(path.to_path_buf()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Saved::Exists(path.to_path_buf(), text)
        }
        Err(error) => Saved::Failed(error.to_string()),
    }
}
