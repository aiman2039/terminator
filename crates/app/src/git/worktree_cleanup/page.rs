//! The Worktrees center page: one row per task worktree with its facts and
//! actions, multi-select Remove, and New task.
use super::{ApplyMode, Ask, probe::Probe};
use crate::*;
use terminator_core::worktrees::Registration;

/// One row's text, gathered before painting.
struct RowFacts {
    project: String,
    path: PathBuf,
    title: String,
    detail: String,
    merged: bool,
    source_branch: String,
    apply_blocked: Option<&'static str>,
}

/// `3h ago`, `2d ago`, or `just now`.
fn age(then: u64, now: u64) -> String {
    let age = now.saturating_sub(then);
    match age {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", age / 60),
        3600..86_400 => format!("{}h ago", age / 3600),
        _ => format!("{}d ago", age / 86_400),
    }
}

/// The second line: changes since the task start, last commit, size, PR
/// and live sessions.
fn detail_line(
    stats: Option<&crate::worktree_stats::WorktreeGit>,
    probe: Option<&Probe>,
    pr: Option<u64>,
    live: usize,
) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let mut parts = Vec::new();
    if let Some(git) = stats {
        parts.push(format!("+{} −{} since task start", git.added, git.deleted));
    }
    if let Some(time) = probe.and_then(|probe| probe.last_commit) {
        parts.push(format!("last commit {}", age(time, now)));
    }
    match probe.and_then(|probe| probe.size) {
        Some((bytes, cut)) => parts.push(format!(
            "{}{}",
            crate::chrome::session_info::format_bytes(bytes),
            if cut { "+" } else { "" }
        )),
        None => parts.push("Measuring…".into()),
    }
    if let Some(number) = pr {
        parts.push(format!("PR #{number}"));
    }
    parts.push(appearance::count_noun(live, "live session"));
    parts.join("  ·  ")
}

impl App {
    pub(crate) fn worktree_management_center(&mut self, ui: &mut egui::Ui) {
        self.poll_cleanup(ui.ctx());
        let style = appearance::CenterCardStyle::new(&self.theme);
        let ((), dismissed) = appearance::center_card(ui, style, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                ui.strong("Worktrees");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if appearance::sidebar_action(ui, "X", "Close").clicked() {
                        self.hide_center_overlay();
                    }
                });
            });
            ui.add_space(8.0);
            self.worktree_page_body(ui);
        });
        if dismissed {
            self.hide_center_overlay();
        }
        self.cleanup_dialog(ui.ctx());
    }

    fn worktree_page_body(&mut self, ui: &mut egui::Ui) {
        if self.worktree_draft.is_some() {
            self.worktree_draft_form(ui);
        }
        let rows = self.worktree_rows();
        if rows.is_empty() && self.worktree_draft.is_none() {
            ui.weak("No worktrees yet.");
        }
        // The rows scroll inside the card; the footer stays under them.
        let footer = ui.spacing().interact_size.y + 16.0;
        egui::ScrollArea::vertical()
            .id_salt("worktree-cleanup")
            .max_height((ui.available_height() - footer).max(80.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for row in &rows {
                    self.cleanup_row(ui, row);
                    ui.separator();
                }
            });
        ui.add_space(8.0);
        let selected: Vec<String> = self.git_extras.cleanup.selected.iter().cloned().collect();
        appearance::footer_row(ui, |ui| {
            if self.worktree_draft.is_none() && ui.button("New task…").clicked() {
                self.open_worktree_wizard();
            }
            let label = format!("Remove selected ({})…", selected.len());
            let why = selected
                .is_empty()
                .then_some("Tick the worktrees to remove");
            let remove = appearance::enabled_unless(ui, why, |ui| ui.button(label));
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "worktree-remove-selected", remove.rect);
            if remove.clicked() {
                self.git_extras.cleanup.ask = Some(Ask::Remove(selected.clone()));
            }
        });
    }

    /// Facts for every managed worktree; starts the due probes.
    fn worktree_rows(&mut self) -> Vec<RowFacts> {
        let worktrees: Vec<Registration> = self
            .state
            .worktrees
            .iter()
            .filter(|w| !w.removed)
            .cloned()
            .collect();
        let mut rows = Vec::new();
        for worktree in worktrees {
            let Some(project) = self
                .state
                .projects
                .iter()
                .find(|p| p.id == worktree.project_id)
            else {
                continue;
            };
            let (project, name) = (project.id.clone(), project.name.clone());
            self.ask_cleanup_probe(&worktree.path);
            rows.push(self.row_facts(&worktree, project, name));
        }
        rows
    }

    fn row_facts(&self, worktree: &Registration, project: String, name: String) -> RowFacts {
        let path = worktree.path.clone();
        let stats = self.worktree_stats.get(&path);
        let probe = self.git_extras.cleanup.probe(&path);
        let branch = stats.and_then(|git| git.branch.clone());
        let pr = self
            .known_pr(&path, branch.as_deref())
            .map(|(number, _)| number);
        let live = self
            .state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live() && (s.project_id == project || s.cwd.starts_with(&path)))
            .count();
        RowFacts {
            title: branch.unwrap_or(name),
            detail: detail_line(stats, probe, pr, live),
            merged: probe.and_then(|probe| probe.merged) == Some(true),
            source_branch: probe
                .and_then(|probe| probe.source_branch.clone())
                .unwrap_or_else(|| "the source checkout".into()),
            apply_blocked: self.apply_blocker(&path),
            project,
            path,
        }
    }

    fn cleanup_row(&mut self, ui: &mut egui::Ui, row: &RowFacts) {
        ui.horizontal(|ui| {
            let mut ticked = self.git_extras.cleanup.selected.contains(&row.project);
            let tick = ui.checkbox(&mut ticked, "");
            #[cfg(feature = "test-support")]
            diagnostics::record(
                ui.ctx(),
                &format!("worktree-tick:{}", row.project),
                tick.rect,
            );
            if tick.changed() {
                if ticked {
                    self.git_extras.cleanup.selected.insert(row.project.clone());
                } else {
                    self.git_extras.cleanup.selected.remove(&row.project);
                }
            }
            if ui
                .link(&row.title)
                .on_hover_text(row.path.display().to_string())
                .clicked()
            {
                self.select_project(row.project.clone());
                self.close_center_page(&CenterPage::Worktrees);
            }
            if row.merged {
                ui.colored_label(appearance::color(&self.theme.status_running), "Merged")
                    .on_hover_text(format!("Its commits are on {}", row.source_branch));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                appearance::icon_menu_button(ui, "EllipsisVertical", |ui| {
                    self.worktree_row_menu(ui, &row.project);
                });
                self.apply_menu(ui, row);
            });
        });
        ui.weak(egui::RichText::new(&row.detail).small());
    }

    /// Apply to source: merge, or cherry-pick the commits since the task start.
    fn apply_menu(&mut self, ui: &mut egui::Ui, row: &RowFacts) {
        let mut picked = None;
        let source = &row.source_branch;
        let merge = format!("Merge into {source}…");
        let pick = format!("Cherry-pick commits into {source}…");
        ui.add_enabled_ui(row.apply_blocked.is_none(), |ui| {
            appearance::text_menu_button(ui, "Apply to source", |ui| {
                let items = [
                    (&merge, "GitBranch", ApplyMode::Merge),
                    (&pick, "GitCompareArrows", ApplyMode::CherryPick),
                ];
                for (label, icon, mode) in items {
                    let item = appearance::menu_item(appearance::MenuItemInput {
                        ui,
                        label,
                        icon,
                        shortcut: "",
                    });
                    if item.clicked() {
                        picked = Some(mode);
                        ui.close();
                    }
                }
            })
        })
        .response
        .on_disabled_hover_text(row.apply_blocked.unwrap_or_default());
        if let Some(mode) = picked {
            self.git_extras.cleanup.ask = Some(Ask::Apply {
                project: row.project.clone(),
                mode,
            });
        }
    }

    /// The pending Apply or Remove confirmation.
    fn cleanup_dialog(&mut self, ctx: &egui::Context) {
        let Some(ask) = self.git_extras.cleanup.ask.clone() else {
            return;
        };
        let (title, body, detail, button) = match &ask {
            Ask::Apply { project, mode } => {
                let name = self.project_display_name(project);
                (
                    "Apply to source",
                    format!("{} the commits of {name} into the source checkout?", mode.verb()),
                    "The source checkout must have no uncommitted changes. A conflict stops Git; finish or abort it in a terminal.".to_owned(),
                    mode.verb(),
                )
            }
            Ask::Remove(projects) => (
                "Remove worktrees",
                format!("Remove {}?", appearance::count_noun(projects.len(), "worktree")),
                "Git refuses a checkout with changes, ignored files or a lock, and live sessions block it. Branches are kept.".to_owned(),
                "Remove",
            ),
        };
        let blocked = match &ask {
            Ask::Apply { project, .. } => self
                .managed_worktree(project)
                .map(|w| w.path.clone())
                .and_then(|path| self.apply_blocker(&path)),
            Ask::Remove(_) => None,
        };
        let window = self.popups.window(ctx, title);
        let confirm = appearance::ConfirmButton::new(button)
            .id("worktree-cleanup-confirm")
            .disabled(blocked);
        let dialog = match ask {
            Ask::Remove(_) => appearance::Confirm::new(body, detail).destructive(confirm),
            Ask::Apply { .. } => appearance::Confirm::new(body, detail).default_action(confirm),
        };
        match appearance::confirm(ctx, window, dialog, |_| {}) {
            Some(appearance::ConfirmAnswer::Cancel) => self.git_extras.cleanup.ask = None,
            Some(_) => {
                self.git_extras.cleanup.ask = None;
                match ask {
                    Ask::Apply { project, mode } => self.start_apply(&project, mode),
                    Ask::Remove(projects) => self.remove_worktrees(&projects),
                }
            }
            None => {}
        }
    }

    fn project_display_name(&self, project: &str) -> String {
        self.state
            .projects
            .iter()
            .find(|p| p.id == project)
            .map_or_else(|| "the worktree".into(), |p| p.name.clone())
    }
}
