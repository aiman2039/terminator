//! Undo/redo history and on-disk modification checks for the flow editor.
use super::inputs::InputRow;
use super::model::{Node, input, output};
use egui_snarl::{NodeId, Snarl};
use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

const MAX_UNDO: usize = 50;

/// Lossless snapshot of one flow editor state (including partial edits).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Snapshot {
    pub(super) title: String,
    pub(super) rules: String,
    pub(super) inputs: Vec<InputRow>,
    pub(super) nodes: Vec<(egui::Pos2, Node)>,
    pub(super) wires: Vec<(usize, usize)>,
}

impl Snapshot {
    pub(super) fn capture(
        title: &str,
        rules: &str,
        inputs: &[InputRow],
        snarl: &Snarl<Node>,
    ) -> Self {
        let mut nodes = Vec::new();
        let mut indices: HashMap<NodeId, usize> = HashMap::new();
        for (id, pos, value) in snarl.nodes_pos_ids() {
            indices.insert(id, nodes.len());
            nodes.push((pos, value.clone()));
        }
        let mut wires: Vec<(usize, usize)> = snarl
            .wires()
            .filter_map(|(from, to)| Some((*indices.get(&from.node)?, *indices.get(&to.node)?)))
            .collect();
        wires.sort_unstable();
        Self {
            title: title.to_owned(),
            rules: rules.to_owned(),
            inputs: inputs.to_vec(),
            nodes,
            wires,
        }
    }

    pub(super) fn restore_snarl(&self) -> Snarl<Node> {
        let mut snarl = Snarl::new();
        let ids: Vec<NodeId> = self
            .nodes
            .iter()
            .map(|(pos, node)| snarl.insert_node(*pos, node.clone()))
            .collect();
        for &(from, to) in &self.wires {
            if let (Some(&a), Some(&b)) = (ids.get(from), ids.get(to)) {
                snarl.connect(output(a), input(b));
            }
        }
        snarl
    }
}

/// Bounded undo/redo stack for flow editor changes.
pub(super) struct History {
    current: Snapshot,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl History {
    pub(super) fn new(initial: Snapshot) -> Self {
        Self {
            current: initial,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub(super) fn sync(&mut self, next: Snapshot) {
        if next == self.current {
            return;
        }
        let prev = std::mem::replace(&mut self.current, next);
        self.undo.push(prev);
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub(super) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(super) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(super) fn undo(&mut self) -> Option<Snapshot> {
        let prev = self.undo.pop()?;
        let current = std::mem::replace(&mut self.current, prev.clone());
        self.redo.push(current);
        Some(prev)
    }

    pub(super) fn redo(&mut self) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        let current = std::mem::replace(&mut self.current, next.clone());
        self.undo.push(current);
        Some(next)
    }
}

/// Last modification time of `path`, when readable.
pub(super) fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Result of a background flow save.
pub(super) enum SaveResult {
    Saved(Option<SystemTime>),
    ChangedOnDisk,
    Failed(String),
}

fn save_flow_file(
    root: &Path,
    file: &str,
    write: &terminator_core::flow::Draft,
    replace: bool,
) -> SaveResult {
    match terminator_core::flow::files::save(root, file, write, replace) {
        Ok(path) => SaveResult::Saved(file_mtime(&path)),
        Err(error) => SaveResult::Failed(error.to_string()),
    }
}

impl super::FlowEditor {
    pub(super) fn snapshot(&self) -> Snapshot {
        Snapshot::capture(&self.title, &self.rules, &self.inputs, &self.snarl)
    }

    pub(super) fn sync_history(&mut self) {
        let snap = self.snapshot();
        self.history.sync(snap);
    }

    fn apply_snapshot(&mut self, snap: Snapshot) {
        self.snarl = snap.restore_snarl();
        self.title = snap.title;
        self.rules = snap.rules;
        self.inputs = snap.inputs;
        self.selected = None;
        self.marked.clear();
        self.sizes.clear();
    }

    pub(super) fn undo(&mut self) -> bool {
        let Some(snap) = self.history.undo() else {
            return false;
        };
        self.apply_snapshot(snap);
        true
    }

    pub(super) fn redo(&mut self) -> bool {
        let Some(snap) = self.history.redo() else {
            return false;
        };
        self.apply_snapshot(snap);
        true
    }

    /// Save: the graph must be a correct flow, then a worker thread writes
    /// the file. A new flow never replaces a file.
    pub(super) fn save(&mut self) {
        self.save_with_force(false);
    }

    pub(super) fn save_with_force(&mut self, force: bool) {
        if self.saving.is_some() {
            return;
        }
        let draft = self.draft();
        if let Some(error) = super::inputs::duplicate(&self.inputs) {
            self.error = Some(error);
            return;
        }
        if let Err(error) = draft.check() {
            self.error = Some(error.to_string());
            return;
        }
        let (root, file, replace) = (self.root.clone(), self.file.clone(), self.on_disk);
        let expected_mtime = self.disk_mtime;
        let (sender, receiver) = std::sync::mpsc::channel();
        let write = draft.clone();
        let spawned = std::thread::Builder::new()
            .name("flow-save".into())
            .spawn(move || {
                let changed = replace
                    && !force
                    && expected_mtime.is_some()
                    && terminator_core::flow::files::path_of(&root, &file)
                        .ok()
                        .is_some_and(|path| file_mtime(&path) != expected_mtime);
                let result = if changed {
                    SaveResult::ChangedOnDisk
                } else {
                    save_flow_file(&root, &file, &write, replace)
                };
                let _ = sender.send(result);
            });
        if spawned.is_ok() {
            self.error = None;
            self.disk_conflict = false;
            self.saving = Some((draft, receiver));
        } else {
            self.error = Some("Could not start the save.".into());
        }
    }

    /// Takes the result of a save. Gives a note for the status bar.
    pub(super) fn poll(&mut self) -> Option<String> {
        let (draft, result) = self.saving.as_ref()?;
        match result.try_recv() {
            Ok(SaveResult::Saved(mtime)) => {
                self.saved = draft.clone();
                self.on_disk = true;
                self.disk_mtime = mtime;
                self.disk_conflict = false;
                self.error = None;
                self.saving = None;
                Some(format!("Saved the flow {}.", self.file))
            }
            Ok(SaveResult::ChangedOnDisk) => {
                self.disk_conflict = true;
                self.error = Some(
                    "This file changed on disk since you opened it. Reload or overwrite anyway."
                        .into(),
                );
                self.saving = None;
                None
            }
            Ok(SaveResult::Failed(error)) => {
                self.error = Some(error);
                self.saving = None;
                None
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.error = Some("The save did not finish.".into());
                self.saving = None;
                None
            }
        }
    }
}
