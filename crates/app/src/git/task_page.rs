//! The Changes since task start page (a center card) and its entry from
//! the worktree row menu. A row opens the native revision diff of one file
//! in the worktree's project: the task base against the working file.
use super::task_changes::{TaskChanges, TaskFile, totals};
use crate::*;

impl App {
    /// The worktree row's context menu (Projects sidebar).
    pub(crate) fn worktree_card_menu(&mut self, ui: &mut egui::Ui, project: &Project) {
        let items: [(&str, &str, String); 3] = [
            ("Open", "FolderOpen", String::new()),
            (
                "New terminal",
                "Terminal",
                self.shortcut_label(Action::NewTerminal),
            ),
            (
                "Changes since task start",
                "GitCompareArrows",
                String::new(),
            ),
        ];
        for (index, (label, icon, shortcut)) in items.iter().enumerate() {
            let item = appearance::menu_item(appearance::MenuItemInput {
                ui,
                label,
                icon,
                shortcut,
            });
            if !item.clicked() {
                continue;
            }
            match index {
                0 => self.open_worktree_card(project),
                1 => self.new_worktree_terminal(project),
                _ => self.open_task_changes(project),
            }
            ui.close();
        }
        ui.separator();
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Remove worktree…",
            icon: "X",
            shortcut: "",
        })
        .clicked()
        {
            self.confirm_remove_worktree(&project.id);
            ui.close();
        }
    }

    /// What a worktree's task base is called: the saved Start-from name,
    /// else the main checkout's branch, else a short hash.
    pub(crate) fn task_base_label(&self, worktree: &Path) -> Option<String> {
        if let Some(base) = self.preferences.task_bases.get(worktree) {
            return Some(base.name.clone());
        }
        let git = self.worktree_stats.get(worktree)?;
        git.base_name
            .clone()
            .or_else(|| git.base.as_ref().map(|hash| short_hash(hash).to_owned()))
    }

    /// Open the page for a managed worktree once its base is known. The
    /// worktree's project is selected, so a row's diff opens there.
    pub(crate) fn open_task_changes(&mut self, project: &Project) {
        let Some(base) = self
            .worktree_stats
            .get(&project.path)
            .and_then(|git| git.base.clone())
        else {
            self.info = Some("The task base of this worktree is not known yet.".into());
            return;
        };
        let label = self
            .task_base_label(&project.path)
            .unwrap_or_else(|| short_hash(&base).to_owned());
        self.select_project(project.id.clone());
        let _ = self
            .jobs
            .send(Job::TaskChanges(project.path.clone(), base.clone()));
        self.task_changes = Some(TaskChanges {
            worktree: project.path.clone(),
            base,
            label,
            files: None,
        });
        self.show_center(CenterPage::TaskChanges);
    }

    /// A job's answer; one for another worktree or base is stale.
    pub(crate) fn apply_task_changes(
        &mut self,
        (worktree, base): (PathBuf, String),
        files: Result<Vec<TaskFile>, String>,
    ) {
        if let Some(page) = &mut self.task_changes
            && page.worktree == worktree
            && page.base == base
        {
            page.files = Some(files);
        }
    }

    pub(crate) fn task_changes_center(&mut self, ui: &mut egui::Ui) {
        let Some(mut page) = self.task_changes.take() else {
            self.hide_center_overlay();
            return;
        };
        let mut action = PageAction::None;
        let style = appearance::CenterCardStyle::new(&self.theme);
        let ((), dismissed) = appearance::center_card(ui, style, |ui| {
            action = self.task_changes_card(ui, &page);
        });
        match action {
            PageAction::Open(path) => {
                let tab = Tab::RevisionDiff {
                    cwd: page.worktree.clone(),
                    path,
                    left: page.base.clone(),
                    right: String::new(),
                };
                self.hide_center_overlay();
                self.open_git_tab(tab);
            }
            PageAction::Reload => {
                let job = Job::TaskChanges(page.worktree.clone(), page.base.clone());
                let _ = self.jobs.send(job);
                page.files = None;
                self.task_changes = Some(page);
            }
            PageAction::Close => self.hide_center_overlay(),
            PageAction::None if dismissed => self.hide_center_overlay(),
            PageAction::None => self.task_changes = Some(page),
        }
    }

    fn task_changes_card(&self, ui: &mut egui::Ui, page: &TaskChanges) -> PageAction {
        let mut action = PageAction::None;
        ui.horizontal(|ui| {
            ui.strong("Changes since task start");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    action = PageAction::Close;
                }
                if appearance::sidebar_action(ui, "RefreshCw", "Reload").clicked() {
                    action = PageAction::Reload;
                }
            });
        });
        let name = page.worktree.file_name().unwrap_or_default();
        ui.weak(format!(
            "{} · vs {} ({}) · committed and uncommitted work. Read-only.",
            name.to_string_lossy(),
            page.label,
            short_hash(&page.base)
        ));
        ui.add_space(8.0);
        match &page.files {
            None => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.weak("Loading…");
                });
            }
            Some(Err(error)) => {
                ui.colored_label(ui.visuals().error_fg_color, error);
            }
            Some(Ok(files)) if files.is_empty() => {
                ui.weak("No changes since the task started.");
            }
            Some(Ok(files)) => {
                if let Some(path) = self.task_file_rows(ui, &page.worktree, files) {
                    action = PageAction::Open(path);
                }
            }
        }
        action
    }

    /// The summary line, then one row per file. Returns the clicked file.
    fn task_file_rows(
        &self,
        ui: &mut egui::Ui,
        worktree: &Path,
        files: &[TaskFile],
    ) -> Option<PathBuf> {
        let (added, deleted) = totals(files);
        let (green, red) = (
            appearance::color(&self.theme.git_added),
            appearance::color(&self.theme.git_deleted),
        );
        ui.horizontal(|ui| {
            ui.label(format!("{} file(s)", files.len()));
            ui.colored_label(green, format!("+{added}"));
            ui.colored_label(red, format!("−{deleted}"));
        });
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .id_salt("task-changes")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for file in files {
                    let shown = file.path.strip_prefix(worktree).unwrap_or(&file.path);
                    let counts = file
                        .lines
                        .map_or_else(String::new, |(a, d)| format!("  +{a} −{d}"));
                    let text = format!("{}  {}{counts}", file.status, shown.display());
                    let row = ui
                        .add(egui::Button::selectable(false, text).frame(false))
                        .on_hover_text("Open the diff from the task base");
                    #[cfg(feature = "test-support")]
                    diagnostics::record(
                        ui.ctx(),
                        &format!("task-change:{}", shown.display()),
                        row.rect,
                    );
                    if row.clicked() {
                        clicked = Some(file.path.clone());
                    }
                }
            });
        clicked
    }
}

/// What a click on the page asked for.
enum PageAction {
    None,
    Open(PathBuf),
    Reload,
    Close,
}

/// The first 8 characters of a commit hash.
fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
