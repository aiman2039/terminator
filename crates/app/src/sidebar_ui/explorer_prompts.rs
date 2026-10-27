use super::explorer_rows::{
    explorer_segmented, explorer_toggle, prompt_button_width, skip_clipped_row,
};
#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::sidebar_cache::ExplorerRow;
use crate::{
    App, Job, appearance, file_actions::FileAction, icons, preferences::ExplorerSearchMode, search,
    workspace_ops,
};
use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};

/// Where an open name prompt sits in the explorer tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PromptSlot {
    /// In place of the row being renamed.
    Replace(usize),
    /// As a new row before `index`, indented to `depth`.
    Insert { index: usize, depth: usize },
}

/// A destructive Git action that waits for an explicit confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GitConfirm {
    Discard { path: PathBuf, untracked: bool },
    DropShelf(String),
}

impl GitConfirm {
    fn prompt(&self) -> (String, &'static str) {
        match self {
            Self::Discard { path, untracked } => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if *untracked {
                    (format!("Delete untracked {name}?"), "Delete")
                } else {
                    (format!("Discard changes to {name}?"), "Discard")
                }
            }
            Self::DropShelf(reference) => (format!("Drop shelf {reference}?"), "Drop"),
        }
    }

    fn into_op(self) -> workspace_ops::Op {
        match self {
            Self::Discard { path, untracked } => workspace_ops::Op::Discard { path, untracked },
            Self::DropShelf(reference) => workspace_ops::Op::DropShelf(reference),
        }
    }
}

impl App {
    pub(super) fn pending_git_confirm_bar(&mut self, ui: &mut egui::Ui) {
        let Some(pending) = self.git.confirm.clone() else {
            return;
        };
        let (prompt, verb) = pending.prompt();
        ui.horizontal(|ui| {
            ui.label(prompt);
            if ui.button(verb).clicked() {
                self.git.confirm = None;
                self.queue_workspace(pending.into_op());
            }
            if ui.button("Cancel").clicked() {
                self.git.confirm = None;
            }
        });
    }

    /// Put a failed commit's message back into an empty draft. A push that
    /// failed after a successful commit keeps the draft clear.
    pub(crate) fn restore_failed_commit(&mut self, error: &str) {
        let Some(message) = self.git.commit_inflight.take() else {
            return;
        };
        if !error.starts_with(workspace_ops::PUSH_FAILED) && self.git.commit.trim().is_empty() {
            self.git.commit = message;
        }
    }

    pub(super) fn pending_delete_bar(&mut self, ui: &mut egui::Ui) {
        let Some(path) = self.pending_delete.clone() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.label(if self.pending_delete_more.is_empty() {
                format!(
                    "Move {} to Trash?",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )
            } else {
                format!(
                    "Move {} items to Trash?",
                    self.pending_delete_more.len().saturating_add(1)
                )
            });
            if ui.button("Move to Trash").clicked() {
                self.queue_workspace(workspace_ops::Op::Delete(path));
                for more in std::mem::take(&mut self.pending_delete_more) {
                    self.queue_workspace(workspace_ops::Op::Delete(more));
                }
                self.pending_delete = None;
                self.explorer_selection.clear();
            }
            if ui.button("Cancel").clicked() {
                self.pending_delete = None;
                self.pending_delete_more.clear();
            }
        });
    }

    pub(crate) fn explorer_toolbar(&mut self, ui: &mut egui::Ui, cwd: &std::path::Path) {
        let host = ui.scope_id();
        let reveal_enabled = self.explorer_reveal_candidate().is_some();
        let reveal_disabled_note = if self.focused_local_file().is_some() {
            "The focused file is outside the Explorer folder"
        } else {
            "Focus a local file to reveal it in Explorer"
        };
        let mut focus_find = false;
        // Pin the row height so the icon buttons, search field, and text
        // toggles all share one baseline.
        ui.spacing_mut().interact_size.y = appearance::TOOLBAR_BUTTON;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let cwd = cwd.to_path_buf();
                let more = appearance::compact_menu_button(ui, "…", |ui| {
                    if appearance::menu_item(ui, "Reveal in file manager", "FolderOpen", "")
                        .clicked()
                    {
                        self.queue_workspace(workspace_ops::Op::Reveal(cwd.clone()));
                        ui.close();
                    }
                    if appearance::menu_item(ui, "Copy path", "Copy", "").clicked() {
                        ui.ctx().copy_text(cwd.display().to_string());
                        ui.close();
                    }
                    if appearance::menu_item(ui, "Copy relative path", "Copy", "").clicked() {
                        let root = self.op_root().unwrap_or_else(|| cwd.clone());
                        ui.ctx()
                            .copy_text(workspace_ops::relative_display(&root, &cwd));
                        ui.close();
                    }
                    ui.separator();
                    if appearance::menu_item(ui, "Find in folder", "Search", "").clicked() {
                        focus_find = true;
                        ui.close();
                    }
                    if appearance::menu_item(ui, "Collapse all", "ChevronsDownUp", "").clicked() {
                        self.expanded_dirs.clear();
                        ui.close();
                    }
                    ui.separator();
                    let mut follow = self.preferences.explorer_autoreveal;
                    if ui
                        .checkbox(&mut follow, "Always select opened file")
                        .clicked()
                    {
                        self.preferences.explorer_autoreveal = follow;
                        self.explorer_followed = None;
                        ui.close();
                    }
                })
                .response
                .on_hover_text("Folder actions");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "explorer-more", more.rect);
                let _ = more;
                let ignored_icon = if self.preferences.show_ignored {
                    "Eye"
                } else {
                    "EyeOff"
                };
                let ignored = appearance::selectable_icon(
                    ui,
                    ignored_icon,
                    "Show ignored files (excluded by Git ignore rules and Git metadata)",
                    self.preferences.show_ignored,
                );
                let reveal = ui
                    .add_enabled_ui(reveal_enabled, |ui| {
                        appearance::sidebar_action(
                            ui,
                            "LocateFixed",
                            "Reveal current file in Explorer",
                        )
                    })
                    .inner
                    .on_disabled_hover_text(reveal_disabled_note);
                let refresh = appearance::sidebar_action(ui, "RefreshCw", "Refresh");
                let collapse = appearance::sidebar_action(ui, "ChevronsDownUp", "Collapse all");
                let new_folder = appearance::sidebar_action(ui, "Folder", "New folder");
                let new_file = appearance::sidebar_action(ui, "File", "New file");
                #[cfg(feature = "test-support")]
                {
                    diagnostics::record(ui.ctx(), "explorer-new-file", new_file.rect);
                    diagnostics::record(ui.ctx(), "explorer-new-folder", new_folder.rect);
                    diagnostics::record(ui.ctx(), "explorer-collapse", collapse.rect);
                    diagnostics::record(ui.ctx(), "explorer-refresh", refresh.rect);
                    diagnostics::record(ui.ctx(), "explorer-reveal-current", reveal.rect);
                    if self
                        .explorer_reveal
                        .as_ref()
                        .is_some_and(|active| active.host == host)
                    {
                        diagnostics::record(ui.ctx(), "explorer-reveal-active", reveal.rect);
                    }
                    diagnostics::record(ui.ctx(), "explorer-show-ignored", ignored.rect);
                }
                if ignored.clicked() {
                    self.preferences.show_ignored = !self.preferences.show_ignored;
                    self.explorer_search_last = None;
                }
                if reveal.clicked() && self.reveal_current_file_in_explorer(host) {
                    ui.ctx().request_repaint();
                }
                // New items land in the selected folder, like `IntelliJ`.
                if new_file.clicked() {
                    self.open_name_prompt(workspace_ops::NamePrompt::File {
                        dir: self.explorer_target_dir(&cwd),
                        name: String::new(),
                    });
                }
                if new_folder.clicked() {
                    self.open_name_prompt(workspace_ops::NamePrompt::Folder {
                        dir: self.explorer_target_dir(&cwd),
                        name: String::new(),
                    });
                }
                if collapse.clicked() {
                    self.expanded_dirs.clear();
                }
                if refresh.clicked() {
                    self.refresh_request = None;
                }
            });
        });
        let contents = self.preferences.explorer_search_mode == ExplorerSearchMode::Contents;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let icon = 18.0;
            ui.add_sized(
                [icon, icon],
                egui::Image::new(icons::source("Search"))
                    .tint(appearance::color(&self.theme.secondary)),
            );
            let toggles = if contents { 78.0 } else { 0.0 };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let pad = ui.spacing().button_padding.x.round() as i8;
            let find = ui
                .scope(|ui| {
                    // egui 0.37 floors a text edit at `interact_size.y`; keep
                    // the toolbar row at the shared button height.
                    ui.spacing_mut().interact_size.y = appearance::TOOLBAR_BUTTON;
                    ui.add_sized(
                        [ui.available_width() - toggles, appearance::TOOLBAR_BUTTON],
                        appearance::singleline(&mut self.explorer_query)
                            .hint_text(if contents { "Search" } else { "Find in folder" })
                            .margin(egui::Margin::symmetric(pad, 2)),
                    )
                })
                .inner;
            if focus_find {
                find.request_focus();
            }
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "explorer-search", find.rect);
            if contents {
                let case = explorer_toggle(
                    ui,
                    "Aa",
                    "Match case",
                    self.preferences.explorer_match_case,
                    "explorer-match-case",
                );
                let word = explorer_toggle(
                    ui,
                    "ab",
                    "Match whole word",
                    self.preferences.explorer_whole_word,
                    "explorer-whole-word",
                );
                let regex = explorer_toggle(
                    ui,
                    ".*",
                    "Use regular expression",
                    self.preferences.explorer_regex,
                    "explorer-regex",
                );
                if case.clicked() {
                    self.preferences.explorer_match_case = !self.preferences.explorer_match_case;
                }
                if word.clicked() {
                    self.preferences.explorer_whole_word = !self.preferences.explorer_whole_word;
                }
                if regex.clicked() {
                    self.preferences.explorer_regex = !self.preferences.explorer_regex;
                }
            }
        });
        ui.add_space(2.0);
        explorer_segmented(ui, &mut self.preferences.explorer_search_mode);
        if contents {
            ui.add_space(4.0);
            ui.label(
                RichText::new("FILES TO INCLUDE")
                    .small()
                    .color(appearance::color(&self.theme.secondary)),
            );
            let include = ui.add_sized(
                [ui.available_width(), 24.0],
                appearance::singleline(&mut self.preferences.explorer_include)
                    .hint_text("files to include (e.g. *.ts, src/**)"),
            );
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "explorer-include", include.rect);
            #[cfg(not(feature = "test-support"))]
            let _ = &include;
            ui.add_space(4.0);
            ui.label(
                RichText::new("FILES TO EXCLUDE")
                    .small()
                    .color(appearance::color(&self.theme.secondary)),
            );
            let exclude = ui.add_sized(
                [ui.available_width(), 24.0],
                appearance::singleline(&mut self.preferences.explorer_exclude)
                    .hint_text("files to exclude (e.g. *.min.js, dist/**)"),
            );
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "explorer-exclude", exclude.rect);
            #[cfg(not(feature = "test-support"))]
            let _ = &exclude;
        }
        self.refresh_explorer_search(cwd);
        self.name_prompt_bar(ui, cwd);
    }

    /// Queue a Contents search when the query or filters changed. Results are
    /// tagged with a generation so stale workers are dropped.
    fn refresh_explorer_search(&mut self, cwd: &std::path::Path) {
        if self.preferences.explorer_search_mode != ExplorerSearchMode::Contents {
            return;
        }
        let query = search::Query {
            text: self.explorer_query.clone(),
            match_case: self.preferences.explorer_match_case,
            whole_word: self.preferences.explorer_whole_word,
            use_regex: self.preferences.explorer_regex,
            include: self.preferences.explorer_include.clone(),
            exclude: self.preferences.explorer_exclude.clone(),
        };
        if self.explorer_search_last.as_ref() == Some(&query) {
            return;
        }
        self.explorer_search_last = Some(query.clone());
        self.explorer_search_generation = self.explorer_search_generation.wrapping_add(1);
        if query.is_empty() {
            self.explorer_search.clear();
            self.explorer_search_error = None;
            self.explorer_search_pending = false;
            return;
        }
        self.explorer_search_pending = true;
        let id = self.explorer_search_generation;
        let show_ignored = self.preferences.show_ignored;
        let _ = self.jobs.send(Job::Search {
            id,
            root: cwd.to_path_buf(),
            query,
            show_ignored,
        });
    }

    /// Contents search results, grouped by file. Clicking a hit opens that file.
    pub(crate) fn explorer_results(&mut self, ui: &mut egui::Ui, cwd: &std::path::Path) {
        if let Some(error) = &self.explorer_search_error {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
            return;
        }
        if self.explorer_search.is_empty() {
            ui.weak(if self.explorer_search_pending {
                "Searching…"
            } else if self.explorer_query.trim().is_empty() {
                "Type to search in files"
            } else {
                "No results"
            });
            return;
        }
        if self.explorer_search_pending {
            ui.weak("Searching…");
        }
        let root = self.op_root().unwrap_or_else(|| cwd.to_path_buf());
        let mut open: Option<PathBuf> = None;
        let mut last: Option<&Path> = None;
        for hit in &self.explorer_search {
            if last != Some(hit.path.as_path()) {
                ui.spacing_mut().item_spacing.y = 2.0;
                let header_height = ui
                    .text_style_height(&egui::TextStyle::Small)
                    .max(ui.spacing().interact_size.y);
                if !skip_clipped_row(ui, header_height) {
                    let relative = workspace_ops::relative_display(&root, &hit.path);
                    ui.add_sized(
                        [ui.available_width(), header_height],
                        egui::Label::new(
                            RichText::new(relative)
                                .small()
                                .color(appearance::color(&self.theme.secondary)),
                        )
                        .truncate(),
                    )
                    .on_hover_text(hit.path.display().to_string());
                }
                last = Some(hit.path.as_path());
            }
            if skip_clipped_row(ui, 20.0) {
                continue;
            }
            let response = appearance::file_row(
                ui,
                &hit.text,
                icons::file_icon(&hit.path),
                false,
                20.0,
                &hit.line.to_string(),
                appearance::color(&self.theme.text),
            )
            .on_hover_text(format!("{}:{}", hit.path.display(), hit.line));
            #[cfg(feature = "test-support")]
            diagnostics::record(
                ui.ctx(),
                &format!("search-hit:{}:{}", hit.path.display(), hit.line),
                response.rect,
            );
            if response.clicked() {
                open = Some(hit.path.clone());
            }
        }
        if let Some(path) = open {
            self.activate_file_action(ui, &path, FileAction::Open);
        }
    }

    /// Scoped to the hosting sidebar: Explorer can render on both sides in
    /// one frame, and an absolute ID would clash between the two copies.
    pub(crate) fn explorer_name_prompt_id(ui: &egui::Ui) -> egui::Id {
        ui.scope_id().with("explorer-name-prompt")
    }

    pub(crate) fn open_name_prompt(&mut self, prompt: workspace_ops::NamePrompt) {
        // A new item appears inline under its folder, so open the folder.
        if let workspace_ops::NamePrompt::File { dir, .. }
        | workspace_ops::NamePrompt::Folder { dir, .. } = &prompt
        {
            self.expanded_dirs.insert(dir.clone());
        }
        self.name_prompt = Some(prompt);
        self.name_prompt_focus = true;
    }

    /// Where the open name prompt renders in the tree under `root`: on
    /// the renamed row, or as a new row at the top of its folder. `None`
    /// keeps it in the toolbar bar (Contents search, or the target row is
    /// not in the tree).
    pub(super) fn name_prompt_slot(&self, rows: &[ExplorerRow], root: &Path) -> Option<PromptSlot> {
        if self.preferences.explorer_search_mode == ExplorerSearchMode::Contents {
            return None;
        }
        match self.name_prompt.as_ref()? {
            workspace_ops::NamePrompt::Rename { from, .. } => rows
                .iter()
                .position(|row| row.entry.is_some() && row.path == *from)
                .map(PromptSlot::Replace),
            workspace_ops::NamePrompt::File { dir, .. }
            | workspace_ops::NamePrompt::Folder { dir, .. } => {
                if dir == root {
                    return rows
                        .first()
                        .is_some_and(|row| row.entry.is_some())
                        .then_some(PromptSlot::Insert { index: 0, depth: 0 });
                }
                let index = rows.iter().position(|row| {
                    row.path == *dir && row.entry.as_ref().is_some_and(|entry| entry.directory)
                })?;
                let row = rows.get(index)?;
                self.expanded_dirs
                    .contains(dir)
                    .then(|| PromptSlot::Insert {
                        index: index.saturating_add(1),
                        depth: row.depth.saturating_add(1),
                    })
            }
        }
    }

    /// The name field of the open prompt. Focuses it (selecting a renamed
    /// file's stem) when the prompt just opened; returns the edited name,
    /// the field, and whether Escape was pressed in it.
    fn name_prompt_field(
        &mut self,
        ui: &mut egui::Ui,
        prompt: &workspace_ops::NamePrompt,
        width: f32,
    ) -> (String, egui::Response, bool) {
        let mut name = match prompt {
            workspace_ops::NamePrompt::File { name, .. }
            | workspace_ops::NamePrompt::Folder { name, .. }
            | workspace_ops::NamePrompt::Rename { name, .. } => name.clone(),
        };
        let focus = std::mem::take(&mut self.name_prompt_focus);
        let prompt_id = Self::explorer_name_prompt_id(ui);
        self.name_prompt_widget = Some(prompt_id);
        let field = ui.add(
            appearance::singleline(&mut name)
                .id(prompt_id)
                .hint_text("Name")
                .desired_width(width),
        );
        if focus {
            field.request_focus();
            // Rename selects the name without its extension, so typing
            // replaces the stem (`IntelliJ`).
            if let workspace_ops::NamePrompt::Rename { from, .. } = prompt
                && let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), prompt_id)
            {
                let stem = if from.is_dir() {
                    name.chars().count()
                } else {
                    from.file_stem()
                        .map_or(0, |stem| stem.to_string_lossy().chars().count())
                };
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(stem),
                    )));
                state.store(ui.ctx(), prompt_id);
            }
        }
        let escape = field.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Escape));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "explorer-name-field", field.rect);
        (name, field, escape)
    }

    /// Store the edited name, then cancel or submit the prompt.
    fn finish_name_prompt(
        &mut self,
        prompt: workspace_ops::NamePrompt,
        name: String,
        submit: bool,
        cancel: bool,
    ) {
        if cancel {
            self.name_prompt = None;
            return;
        }
        self.name_prompt = Some(match prompt {
            workspace_ops::NamePrompt::File { dir, .. } => {
                workspace_ops::NamePrompt::File { dir, name }
            }
            workspace_ops::NamePrompt::Folder { dir, .. } => {
                workspace_ops::NamePrompt::Folder { dir, name }
            }
            workspace_ops::NamePrompt::Rename { from, .. } => {
                workspace_ops::NamePrompt::Rename { from, name }
            }
        });
        if submit {
            self.submit_name_prompt();
        }
    }

    /// The prompt as a tree row (`IntelliJ` / VS Code inline rename): an
    /// icon and the name field at the row's depth. Enter submits; Escape
    /// or clicking elsewhere cancels; either returns focus to the tree.
    pub(super) fn name_prompt_row(&mut self, ui: &mut egui::Ui, tree_focus: egui::Id) {
        let Some(prompt) = self.name_prompt.clone() else {
            return;
        };
        let icon = match &prompt {
            workspace_ops::NamePrompt::File { .. } => "File",
            workspace_ops::NamePrompt::Folder { .. } => "Folder",
            workspace_ops::NamePrompt::Rename { from, .. } => {
                if from.is_dir() {
                    "Folder"
                } else {
                    icons::file_icon(from)
                }
            }
        };
        ui.add_space(4.0);
        ui.add_sized(
            [16.0, 16.0],
            egui::Image::new(icons::source(icon)).tint(ui.visuals().text_color()),
        );
        let width = (ui.available_width() - 4.0).max(48.0);
        let (name, field, escape) = self.name_prompt_field(ui, &prompt, width);
        let submit = field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
        let cancel = escape || (field.lost_focus() && !submit);
        if submit || cancel {
            ui.memory_mut(|memory| memory.request_focus(tree_focus));
        }
        self.finish_name_prompt(prompt, name, submit, cancel);
    }

    fn name_prompt_bar(&mut self, ui: &mut egui::Ui, cwd: &Path) {
        let Some(prompt) = self.name_prompt.clone() else {
            return;
        };
        let rows = self.explorer_rows(cwd);
        if self.name_prompt_slot(&rows, cwd).is_some() {
            return;
        }
        let title = match prompt {
            workspace_ops::NamePrompt::File { .. } => "New file",
            workspace_ops::NamePrompt::Folder { .. } => "New folder",
            workspace_ops::NamePrompt::Rename { .. } => "Rename",
        };
        let mut cancel = false;
        let mut submit = false;
        let mut name = String::new();
        let save_w = prompt_button_width(ui, "Save");
        let cancel_w = prompt_button_width(ui, "Cancel");
        let gap = ui.spacing().item_spacing.x;
        let reserve = save_w + cancel_w + gap + gap;
        let min_field = 96.0;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            let title_response = ui.label(title);
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "explorer-name-title", title_response.rect);
            #[cfg(not(feature = "test-support"))]
            let _ = &title_response;
            // `available_width` is the whole row in a wrapping layout. The
            // space left beside the title is `available_size_before_wrap`.
            let row_left = ui.available_size_before_wrap().x;
            let row_full = ui.available_width();
            let field_w = if row_left >= reserve + min_field {
                row_left - reserve
            } else if row_left >= min_field {
                row_left
            } else {
                row_full.max(min_field)
            };
            let (edited, field, escape) = self.name_prompt_field(ui, &prompt, field_w);
            name = edited;
            cancel = escape;
            let save = ui.button("Save");
            let cancel_button = ui.button("Cancel");
            #[cfg(feature = "test-support")]
            {
                diagnostics::record(ui.ctx(), "explorer-name-save", save.rect);
                diagnostics::record(ui.ctx(), "explorer-name-cancel", cancel_button.rect);
            }
            submit = save.clicked()
                || (field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
            cancel |= cancel_button.clicked();
        });
        self.finish_name_prompt(prompt, name, submit, cancel);
    }

    fn submit_name_prompt(&mut self) {
        let Some(prompt) = self.name_prompt.clone() else {
            return;
        };
        let op = match prompt {
            workspace_ops::NamePrompt::File { dir, name } => {
                workspace_ops::Op::CreateFile(dir.join(name.trim()))
            }
            workspace_ops::NamePrompt::Folder { dir, name } => {
                workspace_ops::Op::CreateDir(dir.join(name.trim()))
            }
            workspace_ops::NamePrompt::Rename { from, name } => {
                let Some(parent) = from.parent().map(std::path::Path::to_path_buf) else {
                    return;
                };
                workspace_ops::Op::Rename {
                    from,
                    to: parent.join(name.trim()),
                }
            }
        };
        self.name_prompt = None;
        self.queue_workspace(op);
    }
}
