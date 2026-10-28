//! Grouped change sections and the per-folder change tree.
use super::*;

/// Orca-style section header with per-section stage/unstage all. The body is a
/// directory tree (JetBrains-style), collapsible per folder.
pub(super) struct GitGroupSectionInput<'a, 'b> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) ctx: &'a GitRowCtx<'b>,
    pub(super) outcome: &'a mut GitPanelOutcome,
    pub(super) root: Option<&'a Path>,
    pub(super) prepared: &'a PreparedGroup,
    pub(super) collapse_generation: u64,
}

pub(super) fn git_group_section(input: GitGroupSectionInput<'_, '_>) {
    let GitGroupSectionInput {
        ui,
        ctx,
        outcome,
        root,
        prepared,
        collapse_generation,
    } = input;
    let group = prepared.group;
    let entries = &prepared.entries;
    let label = git_group_label(group);
    let id = ui.make_persistent_id(("git-section", root, label, collapse_generation));
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        collapse_generation == 0,
    );
    let mut group_check_clicked = false;
    let _ = state
        .show_header(ui, |ui| {
            group_check_clicked = git_group_header(ui, prepared, outcome);
        })
        .body(|ui| {
            let tree = &prepared.tree;
            ui.spacing_mut().indent = GIT_TREE_INDENT;
            ui.spacing_mut().item_spacing.y = 0.0;
            git_change_tree(GitChangeTreeInput {
                ui,
                ctx,
                outcome,
                node: tree,
                prefix: "",
                group,
            });
        });
    if group_check_clicked {
        for change in entries {
            outcome.toggle_staged(group, &change.path);
        }
    }
}

/// Group checkbox, `LABEL count`, and the stage/unstage-all action.
/// Returns whether the checkbox was clicked.
fn git_group_header(
    ui: &mut egui::Ui,
    prepared: &PreparedGroup,
    outcome: &mut GitPanelOutcome,
) -> bool {
    let group = prepared.group;
    let total = prepared.entries.len();
    let label = git_group_label(group);
    ui.spacing_mut().item_spacing.x = 2.0;
    let mut check_clicked = false;
    if group != terminator_git::GitGroup::Conflicts {
        let checked = if group == terminator_git::GitGroup::Staged {
            total
        } else {
            0
        };
        check_clicked = tri_checkbox(ui, check_state(CheckStateInput { checked, total }));
    }
    ui.add(
        egui::Label::new(RichText::new(format!("{label} {total}")).small().strong())
            .selectable(false),
    );
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        git_group_bulk_action(ui, prepared, outcome);
    });
    check_clicked
}

fn git_group_bulk_action(
    ui: &mut egui::Ui,
    prepared: &PreparedGroup,
    outcome: &mut GitPanelOutcome,
) {
    let entries = &prepared.entries;
    match prepared.group {
        terminator_git::GitGroup::Staged => {
            let action = appearance::sidebar_action(ui, "ArrowLeft", "Unstage all");
            if action.clicked() {
                outcome
                    .unstage
                    .extend(entries.iter().map(|change| change.path.clone()));
            }
        }
        terminator_git::GitGroup::Changes | terminator_git::GitGroup::Untracked => {
            let action = appearance::sidebar_action(ui, "Plus", "Stage all");
            if action.clicked() {
                outcome
                    .stage
                    .extend(entries.iter().map(|change| change.path.clone()));
            }
        }
        terminator_git::GitGroup::Conflicts => {}
    }
}

/// Prepared once per accepted filesystem/Git result, reused while scrolling.
pub(crate) struct PreparedGit {
    pub(super) groups: Vec<PreparedGroup>,
}

pub(super) struct PreparedGroup {
    pub(super) group: terminator_git::GitGroup,
    pub(super) entries: Vec<terminator_git::Change>,
    pub(super) labels: Vec<String>,
    pub(super) tree: ChangeTree,
}

impl PreparedGit {
    pub fn new(context: &ContextData) -> Self {
        let groups = terminator_git::GitGroup::ALL
            .into_iter()
            .filter_map(|group| {
                let entries: Vec<_> = context
                    .changes
                    .iter()
                    .filter(|c| c.in_group(group))
                    .cloned()
                    .collect();
                if entries.is_empty() {
                    return None;
                }
                let labels = entries
                    .iter()
                    .map(|c| {
                        workspace_ops::relative_display(
                            context.root.as_deref().unwrap_or(&c.path),
                            &c.path,
                        )
                    })
                    .collect();
                let tree =
                    build_change_tree(&entries.iter().collect::<Vec<_>>(), context.root.as_deref());
                Some(PreparedGroup {
                    group,
                    entries,
                    labels,
                    tree,
                })
            })
            .collect();
        Self { groups }
    }
}

#[derive(Default)]
pub(in crate::sidebar_ui) struct ChangeTree {
    pub(in crate::sidebar_ui) dirs: std::collections::BTreeMap<String, ChangeTree>,
    pub(in crate::sidebar_ui) files: Vec<terminator_git::Change>,
    labels: Vec<String>,
    count: usize,
}

impl ChangeTree {
    pub(in crate::sidebar_ui) fn count_files(&self) -> usize {
        self.count
    }
    pub(in crate::sidebar_ui) fn collect<'a>(&'a self, out: &mut Vec<&'a terminator_git::Change>) {
        out.extend(self.files.iter());
        for dir in self.dirs.values() {
            dir.collect(out);
        }
    }
}

pub(in crate::sidebar_ui) fn build_change_tree(
    entries: &[&terminator_git::Change],
    root: Option<&Path>,
) -> ChangeTree {
    fn insert(node: &mut ChangeTree, dirs: &[String], change: &terminator_git::Change) {
        node.count = node.count.saturating_add(1);
        match dirs.split_first() {
            None => {
                node.files.push(change.clone());
                node.labels.push(
                    change
                        .path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
            }
            Some((dir, rest)) => insert(node.dirs.entry(dir.clone()).or_default(), rest, change),
        }
    }
    let mut tree = ChangeTree::default();
    for change in entries {
        let relative = root
            .and_then(|root| change.path.strip_prefix(root).ok())
            .unwrap_or(&change.path);
        let components: Vec<String> = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        if !components.is_empty() {
            let parent = components.len().saturating_sub(1);
            if let Some(dirs) = components.get(..parent) {
                insert(&mut tree, dirs, change);
            }
        }
    }
    compress_change_tree(&mut tree);
    tree
}

/// Merge single-child directory chains into one `a/b/c` node (`JetBrains` style).
/// style. A directory with files or several subdirectories stops the chain.
pub(super) fn compress_change_tree(node: &mut ChangeTree) {
    let entries: Vec<(String, ChangeTree)> = std::mem::take(&mut node.dirs).into_iter().collect();
    for (mut key, mut child) in entries {
        while child.files.is_empty() && child.dirs.len() == 1 {
            let (next_key, next) = std::mem::take(&mut child.dirs)
                .into_iter()
                .next()
                .unwrap_or_default();
            key = format!("{key}/{next_key}");
            child = next;
        }
        compress_change_tree(&mut child);
        node.dirs.insert(key, child);
    }
}

pub(super) struct GitChangeTreeInput<'a, 'b> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) ctx: &'a GitRowCtx<'b>,
    pub(super) outcome: &'a mut GitPanelOutcome,
    pub(super) node: &'a ChangeTree,
    pub(super) prefix: &'a str,
    pub(super) group: terminator_git::GitGroup,
}

pub(super) fn git_change_tree(input: GitChangeTreeInput<'_, '_>) {
    let GitChangeTreeInput {
        ui,
        ctx,
        outcome,
        node,
        prefix,
        group,
    } = input;
    for (name, child) in &node.dirs {
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        git_tree_folder(GitTreeFolderInput {
            ui,
            ctx,
            outcome,
            folder: TreeFolder {
                name,
                key: &key,
                child,
                group,
            },
        });
    }
    for (change, label) in node.files.iter().zip(&node.labels) {
        git_change_row(GitChangeRowInput {
            ui,
            ctx,
            outcome,
            change,
            group,
            label,
        });
    }
}

/// One folder of the change tree within one group.
struct TreeFolder<'a> {
    name: &'a str,
    key: &'a str,
    child: &'a ChangeTree,
    group: terminator_git::GitGroup,
}

struct GitTreeFolderInput<'a, 'b> {
    ui: &'a mut egui::Ui,
    ctx: &'a GitRowCtx<'b>,
    outcome: &'a mut GitPanelOutcome,
    folder: TreeFolder<'a>,
}

/// A collapsible folder row with its checkbox, then its subtree.
fn git_tree_folder(input: GitTreeFolderInput<'_, '_>) {
    let GitTreeFolderInput {
        ui,
        ctx,
        outcome,
        folder,
    } = input;
    let id = ui.make_persistent_id(("git-tree", folder.key.to_owned(), folder.group.label()));
    let count = folder.child.count_files();
    let mut folder_response = None;
    let mut folder_check_clicked = false;
    let mut header =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
            .show_header(ui, |ui| {
                let (clicked, response) = folder.header(ui, ctx.theme);
                folder_check_clicked = clicked;
                folder_response = Some(response);
            });
    if folder_check_clicked {
        for path in folder.paths() {
            outcome.toggle_staged(folder.group, &path);
        }
    }
    if folder_response
        .as_ref()
        .is_some_and(eframe::egui::Response::clicked)
    {
        header.toggle();
    }
    let _ = header.body(|ui| {
        git_change_tree(GitChangeTreeInput {
            ui,
            ctx,
            outcome,
            node: folder.child,
            prefix: folder.key,
            group: folder.group,
        })
    });
    if let Some(response) = folder_response {
        let response = response.on_hover_text(format!("{} ({count} changed)", folder.key));
        appearance::context_menu(&response, |ui| folder.menu(ui, outcome));
    }
}

impl TreeFolder<'_> {
    /// Folder checkbox and row. Returns whether the checkbox was clicked
    /// and the row response.
    fn header(&self, ui: &mut egui::Ui, theme: &AppearanceConfig) -> (bool, egui::Response) {
        let group = self.group;
        let count = self.child.count_files();
        ui.spacing_mut().item_spacing.x = 2.0;
        let mut check_clicked = false;
        if group != terminator_git::GitGroup::Conflicts {
            let checked = if group == terminator_git::GitGroup::Staged {
                count
            } else {
                0
            };
            check_clicked = tri_checkbox(
                ui,
                check_state(CheckStateInput {
                    checked,
                    total: count,
                }),
            );
        }
        let trailing = count.to_string();
        let response = appearance::file_row(appearance::FileRowInput {
            ui,
            label: self.name,
            icon: "Folder",
            selected: false,
            height: GIT_TREE_ROW_HEIGHT,
            trailing: &trailing,
            tint: appearance::color(&theme.secondary),
        });
        (check_clicked, response)
    }

    /// Every file under this folder.
    fn paths(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        self.child.collect(&mut files);
        files
            .into_iter()
            .map(|change| change.path.clone())
            .collect()
    }

    /// Stage or unstage the whole folder.
    fn menu(&self, ui: &mut egui::Ui, outcome: &mut GitPanelOutcome) {
        match self.group {
            terminator_git::GitGroup::Staged => {
                if appearance::menu_item(ui, "Unstage folder", "ArrowLeft", "").clicked() {
                    outcome.unstage.extend(self.paths());
                    ui.close();
                }
            }
            terminator_git::GitGroup::Changes | terminator_git::GitGroup::Untracked => {
                if appearance::menu_item(ui, "Stage folder", "Plus", "").clicked() {
                    outcome.stage.extend(self.paths());
                    ui.close();
                }
            }
            terminator_git::GitGroup::Conflicts => {}
        }
    }
}
