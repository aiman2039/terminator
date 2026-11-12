//! Grouped change sections and the per-folder change tree.
use super::*;

/// Change rows painted beyond the visible part, above and below.
const CULL_MARGIN: f32 = 400.0;

/// Thousands of changes: lay out and paint only rows near the visible
/// part. The first row sets the stride (rows are one fixed height); rows
/// far outside only take their height, so the scroll range is unchanged.
/// The margin keeps the next row there for arrow keys, so keyboard focus
/// can move to it and scroll it into view.
pub(super) fn cull_rows<T>(
    ui: &mut egui::Ui,
    rows: impl IntoIterator<Item = T>,
    mut draw: impl FnMut(&mut egui::Ui, T),
) {
    let visible = ui.clip_rect().expand2(egui::vec2(0.0, CULL_MARGIN));
    let mut stride: Option<f32> = None;
    for row in rows {
        let top = ui.cursor().min.y;
        if let Some(stride) = stride
            && (top + stride < visible.min.y || top > visible.max.y)
        {
            ui.add_space(stride);
            continue;
        }
        draw(ui, row);
        stride.get_or_insert(ui.cursor().min.y - top);
    }
}

/// Orca-style section header with per-section stage/unstage all. The body is a
/// directory tree (JetBrains-style), collapsible per folder.
pub(super) struct GitGroupSectionInput<'a, 'b> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) ctx: &'a GitRowCtx<'b>,
    pub(super) outcome: &'a mut GitPanelOutcome,
    pub(super) root: Option<&'a Path>,
    pub(super) prepared: &'a PreparedGroup,
}

pub(super) fn git_group_section(input: GitGroupSectionInput<'_, '_>) {
    let GitGroupSectionInput {
        ui,
        ctx,
        outcome,
        root,
        prepared,
    } = input;
    let group = prepared.group;
    let entries = &prepared.entries;
    let label = git_group_label(group);
    let id = ui.make_persistent_id(("git-section", root, label, ctx.generation));
    let state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, ctx.open);
    let mut group_check_clicked = false;
    // The group row has the tree's highlight, like the `JetBrains` root.
    let background = ui.painter().add(egui::Shape::Noop);
    let (toggle, header, _) = state
        .show_header(ui, |ui| {
            group_check_clicked = git_group_header(ui, prepared, ctx.lock.index, outcome);
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
    let row = toggle.rect.union(header.response.rect);
    let row = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), row.y_range());
    let fill = ui.visuals().widgets.hovered.bg_fill.gamma_multiply(0.6);
    ui.painter().set(
        background,
        egui::Shape::rect_filled(row, appearance::RADIUS_SMALL, fill),
    );
    if group_check_clicked {
        for change in entries {
            outcome.toggle_staged(group, &change.path);
        }
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
    /// `N files`, built once with the tree.
    hint: String,
}

impl ChangeTree {
    pub(in crate::sidebar_ui) fn count_files(&self) -> usize {
        self.count
    }
    pub(super) fn files_hint(&self) -> &str {
        &self.hint
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
    node.hint = files_hint(node.count);
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
    if node.files.is_empty() {
        return;
    }
    // Files have no chevron. Reserve its width so a file is one level deeper
    // than its folder, aligned with sibling folders' checkboxes.
    let gutter = ui.spacing().indent + ui.spacing().item_spacing.x;
    ui.horizontal_top(|ui| {
        ui.add_space(gutter);
        ui.vertical(|ui| {
            cull_rows(
                ui,
                node.files.iter().zip(&node.labels),
                |ui, (change, label)| {
                    git_change_row(GitChangeRowInput {
                        ui,
                        ctx,
                        outcome,
                        change,
                        group,
                        label,
                        hint: "",
                    });
                },
            );
        });
    });
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
    let id = ui.make_persistent_id((
        "git-tree",
        folder.key.to_owned(),
        folder.group.label(),
        ctx.generation,
    ));
    let count = folder.child.count_files();
    let mut folder_response = None;
    let mut folder_check_clicked = false;
    let mut header =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, ctx.open)
            .show_header(ui, |ui| {
                let (clicked, response) = folder.header(ui, ctx);
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
        appearance::context_menu(&response, |ui| folder.menu(ui, ctx.lock.index, outcome));
    }
}

impl TreeFolder<'_> {
    /// Folder checkbox and row. Returns whether the checkbox was clicked
    /// and the row response.
    fn header(&self, ui: &mut egui::Ui, ctx: &GitRowCtx<'_>) -> (bool, egui::Response) {
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
            let state = check_state(CheckStateInput {
                checked,
                total: count,
            });
            check_clicked = tri_checkbox(ui, state, ctx.lock.index);
        }
        let hint = self.child.files_hint();
        let color = ui.visuals().text_color();
        let response = name_row(NameRowInput {
            ui,
            icon: "Folder",
            icon_tint: appearance::icon_color(),
            name: self.name,
            color,
            hint,
            trailing: "",
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

    /// Stage or unstage the whole folder, unless `lock` disables it.
    fn menu(&self, ui: &mut egui::Ui, lock: Option<&str>, outcome: &mut GitPanelOutcome) {
        let (label, icon) = match self.group {
            terminator_git::GitGroup::Staged => ("Unstage folder", UNSTAGE_ICON),
            terminator_git::GitGroup::Changes | terminator_git::GitGroup::Untracked => {
                ("Stage folder", STAGE_ICON)
            }
            terminator_git::GitGroup::Conflicts => return,
        };
        let item = appearance::enabled_unless(ui, lock, |ui| {
            appearance::menu_item(appearance::MenuItemInput {
                ui,
                label,
                icon,
                shortcut: "",
            })
        });
        if item.clicked() {
            if self.group == terminator_git::GitGroup::Staged {
                outcome.unstage.extend(self.paths());
            } else {
                outcome.stage.extend(self.paths());
            }
            ui.close();
        }
    }
}
