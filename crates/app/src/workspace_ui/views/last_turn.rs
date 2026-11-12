//! Last turn page body: the scope row, the files the agent's newest turn
//! changed, a diff of the selected file against its turn-start snapshot,
//! and the confirmed "Revert last turn…".
use super::super::super::*;
use super::super::diff_paint;
use crate::local_history::turn::{self, LastTurn};
use terminator_core::local_history::{Turn, TurnFile};

const LIST_WIDTH: f32 = 300.0;

/// Short age such as `5m ago`.
fn age(timestamp_ms: u64, now_ms: u64) -> String {
    let secs = now_ms.saturating_sub(timestamp_ms) / 1000;
    match secs {
        0..90 => format!("{secs}s ago"),
        90..5400 => format!("{}m ago", secs / 60),
        _ => format!("{}h ago", secs / 3600),
    }
}

/// Status letter of a row: `M` snapshot base, `I` Git index base, `A`
/// no base (new in the turn).
fn marker(file: &TurnFile) -> &'static str {
    match (&file.base, file.base_is_git_index) {
        (None, _) => "A",
        (Some(_), true) => "I",
        (Some(_), false) => "M",
    }
}

/// What a click on the page asked for.
enum PageAction {
    None,
    Close,
    Reload,
    Uncommitted,
    TaskStart(Project),
    Select(PathBuf),
}

impl App {
    pub(crate) fn last_turn_center(&mut self, ui: &mut egui::Ui) {
        let Some(page) = self.git_extras.last_turn.take() else {
            self.hide_center_overlay();
            return;
        };
        ui.set_min_size(ui.available_size());
        let mut action = self.last_turn_header(ui, &page);
        ui.separator();
        let height = ui.available_height();
        let mut page = page;
        ui.horizontal_top(|ui| {
            ui.allocate_ui(egui::vec2(LIST_WIDTH, height), |ui| {
                ui.set_width(LIST_WIDTH);
                if let Some(path) = self.last_turn_list(ui, &page) {
                    action = PageAction::Select(path);
                }
            });
            ui.separator();
            ui.vertical(|ui| self.last_turn_diff(ui, &mut page));
        });
        let confirmed = page.confirm && self.last_turn_confirm(ui.ctx(), &mut page);
        self.git_extras.last_turn = Some(page);
        if confirmed {
            self.revert_last_turn();
        }
        self.apply_last_turn_action(action);
    }

    fn apply_last_turn_action(&mut self, action: PageAction) {
        match action {
            PageAction::None => {}
            PageAction::Close => self.hide_center_overlay(),
            PageAction::Reload => self.reload_last_turn(),
            PageAction::Uncommitted => {
                self.hide_center_overlay();
                self.open_changes_diff(false);
            }
            PageAction::TaskStart(project) => self.open_task_changes(&project),
            PageAction::Select(path) => self.select_turn_file(path),
        }
    }

    /// Title, turn facts, and the scope row.
    fn last_turn_header(&self, ui: &mut egui::Ui, page: &LastTurn) -> PageAction {
        let mut action = PageAction::None;
        ui.horizontal(|ui| {
            ui.strong("Last turn");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    action = PageAction::Close;
                }
                if appearance::sidebar_action(ui, "RefreshCw", "Reload").clicked() {
                    action = PageAction::Reload;
                }
            });
        });
        if let Some(Ok(Some(turn))) = &page.turn {
            ui.weak(turn_facts(&page.label, turn));
        }
        ui.horizontal(|ui| {
            ui.weak("Scope");
            if ui.selectable_label(false, "Uncommitted").clicked() {
                action = PageAction::Uncommitted;
            }
            if let Some(project) = self.task_start_project(page)
                && ui.selectable_label(false, "Since task start").clicked()
            {
                action = PageAction::TaskStart(project);
            }
            let _ = ui.selectable_label(true, "Last turn");
        });
        action
    }

    /// The worktree project of the page's session, when its task base is
    /// known.
    fn task_start_project(&self, page: &LastTurn) -> Option<Project> {
        let session = self.state.sessions.iter().find(|s| s.id == page.session)?;
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == session.project_id)?;
        self.worktree_stats
            .get(&project.path)
            .and_then(|git| git.base.as_ref())
            .map(|_| project.clone())
    }

    /// The changed files. Returns the clicked one.
    fn last_turn_list(&self, ui: &mut egui::Ui, page: &LastTurn) -> Option<PathBuf> {
        let turn = match &page.turn {
            None => {
                ui.spinner();
                return None;
            }
            Some(Err(error)) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
                return None;
            }
            Some(Ok(None)) => {
                ui.weak("No agent turn in this session since the session service started.");
                return None;
            }
            Some(Ok(Some(turn))) => turn,
        };
        if turn.files.is_empty() {
            ui.weak("Local History saw no file change in this turn.");
            return None;
        }
        let mut clicked = None;
        appearance::sidebar_scroll("last-turn").show(ui, |ui| {
            for file in &turn.files {
                let shown = file
                    .path
                    .strip_prefix(&turn.project_root)
                    .unwrap_or(&file.path);
                let chosen = page.selected.as_ref() == Some(&file.path);
                let text = format!("{}  {}", marker(file), shown.display());
                if ui
                    .add(egui::Button::selectable(chosen, text).truncate())
                    .clicked()
                {
                    clicked = Some(file.path.clone());
                }
            }
            if turn.truncated {
                ui.weak("More files changed than this list shows.");
            }
        });
        clicked
    }

    fn last_turn_diff(&mut self, ui: &mut egui::Ui, page: &mut LastTurn) {
        let Some(Ok(Some(_))) = &page.turn else {
            return;
        };
        ui.horizontal(|ui| {
            let button = egui::Button::new("Revert last turn…");
            if ui.add_enabled(!page.busy, button).clicked() {
                page.confirm = true;
                page.include_index = false;
            }
            if page.busy {
                ui.spinner();
            }
        });
        let Some(path) = page.selected.clone() else {
            ui.weak("Select a file to compare its turn-start content with the file on disk.");
            return;
        };
        let doc = match &page.doc {
            None => {
                ui.spinner();
                return;
            }
            Some(Err(error)) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
                return;
            }
            Some(Ok(doc)) => std::sync::Arc::clone(doc),
        };
        self.paint_turn_doc(ui, &path, &doc);
    }

    fn paint_turn_doc(&mut self, ui: &mut egui::Ui, path: &Path, doc: &diff::DiffDocument) {
        let key = format!("last-turn:{}", path.display());
        let split = self.diff_split.contains(&key);
        diff_paint::comparison_headers(diff_paint::ComparisonHeadersInput {
            ui,
            doc,
            split,
            ratio: self.diff_split_ratio,
            path,
            editable: false,
            staged: false,
        });
        if doc.hunks.is_empty() {
            ui.weak("No differences: the file has its turn-start content.");
            return;
        }
        let colors = diff_paint::DiffColors::from_theme(&self.theme);
        let mut sync = self.diff_split_scroll.get(&key).copied().unwrap_or(0.0);
        diff_paint::paint_diff_document(diff_paint::PaintDiffDocumentInput {
            ui,
            doc,
            split,
            colors,
            scroll_key: &key,
            ratio: &mut self.diff_split_ratio,
            sync: &mut sync,
        });
        self.diff_split_scroll.insert(key, sync);
    }

    /// The confirmation that lists every file. True when confirmed.
    fn last_turn_confirm(&mut self, ctx: &egui::Context, page: &mut LastTurn) -> bool {
        let Some(Ok(Some(turn_data))) = &page.turn else {
            page.confirm = false;
            return false;
        };
        let plan = turn::plan(turn_data);
        let root = turn_data.project_root.clone();
        let count = plan.chosen(page.include_index).len();
        let blocked = (count == 0).then_some("No file to revert");
        let body = format!(
            "Restore {} to the content from before the turn?",
            appearance::count_noun(count, "file")
        );
        let detail = "Local History keeps the current content as \"Before revert\", so this can be undone there. Git is not run.";
        let button = appearance::ConfirmButton::new("Revert")
            .id("last-turn-revert")
            .disabled(blocked);
        let dialog = appearance::Confirm::new(body, detail).destructive(button);
        let window = self.popups.window(ctx, "Revert last turn");
        let mut include = page.include_index;
        let answer = appearance::confirm(ctx, window, dialog, |ui| {
            plan_lists(ui, &plan, &root, &mut include);
        });
        page.include_index = include;
        match answer {
            Some(appearance::ConfirmAnswer::Cancel) => page.confirm = false,
            Some(_) => return true,
            None => {}
        }
        false
    }
}

/// The turn's agent, start and state on one line.
fn turn_facts(label: &str, turn: &Turn) -> String {
    let now = terminator_core::local_history::now_ms();
    let state = turn.mark.ended_ms.map_or_else(
        || "running".to_owned(),
        |end| format!("ended {}", age(end, now)),
    );
    format!(
        "{label} · {} · started {} · {state} · {}",
        turn.mark.agent_kind,
        age(turn.mark.started_ms, now),
        appearance::count_noun(turn.files.len(), "file")
    )
}

/// The confirmation's file lists: reverted, Git-index-only (opt in), and
/// not reverted.
fn plan_lists(ui: &mut egui::Ui, plan: &turn::RevertPlan, root: &Path, include: &mut bool) {
    let rel = |path: &PathBuf| {
        path.strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string()
    };
    egui::ScrollArea::vertical()
        .id_salt("last-turn-confirm")
        .max_height(240.0)
        .show(ui, |ui| {
            for item in &plan.items {
                ui.label(rel(&item.path));
            }
            if !plan.index_items.is_empty() {
                ui.add_space(6.0);
                ui.checkbox(
                    include,
                    "Also restore these files to their Git index version (their content at the turn start was not recorded):",
                );
                for item in &plan.index_items {
                    ui.weak(rel(&item.path));
                }
            }
            if !plan.skipped.is_empty() {
                ui.add_space(6.0);
                ui.weak("Not reverted:");
                for (path, why) in &plan.skipped {
                    ui.weak(format!("{}  ({why})", rel(path)));
                }
            }
        });
}
