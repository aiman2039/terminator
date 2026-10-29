//! The Explorer toolbar: folder actions, search field, and filters.
use super::*;

/// Toolbar buttons painted this frame, acted on after painting.
struct ToolbarButtons {
    ignored: egui::Response,
    reveal: egui::Response,
    refresh: egui::Response,
    collapse: egui::Response,
    new_folder: egui::Response,
    new_file: egui::Response,
}

impl App {
    pub(crate) fn explorer_toolbar(&mut self, ui: &mut egui::Ui, cwd: &std::path::Path) {
        let host = ui.scope_id();
        // Pin the row height so the icon buttons, search field, and text
        // toggles all share one baseline.
        ui.spacing_mut().interact_size.y = appearance::TOOLBAR_BUTTON;
        let focus_find = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.explorer_toolbar_actions(ui, cwd, host)
                })
                .inner
            })
            .inner;
        let contents = self.preferences.explorer_search_mode == ExplorerSearchMode::Contents;
        ui.horizontal(|ui| self.explorer_search_row(ui, contents, focus_find));
        ui.add_space(2.0);
        explorer_segmented(ui, &mut self.preferences.explorer_search_mode);
        if contents {
            self.explorer_search_filters(ui);
        }
        self.refresh_explorer_search(cwd);
        self.name_prompt_bar(ui, cwd);
    }

    /// The right-aligned action buttons. Returns whether "Find in folder"
    /// was picked.
    fn explorer_toolbar_actions(&mut self, ui: &mut egui::Ui, cwd: &Path, host: egui::Id) -> bool {
        ui.spacing_mut().item_spacing.x = 4.0;
        let cwd = cwd.to_path_buf();
        let mut focus_find = false;
        let more = appearance::compact_menu_button(ui, "…", |ui| {
            focus_find = self.explorer_more_menu(ui, &cwd);
        })
        .response
        .on_hover_text("Folder actions");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "explorer-more", more.rect);
        let _ = more;
        let buttons = self.explorer_toolbar_buttons(ui);
        #[cfg(feature = "test-support")]
        self.record_toolbar_buttons(ui, &buttons, host);
        self.explorer_toolbar_clicks(ui, &buttons, (&cwd, host));
        focus_find
    }

    /// The "…" folder menu. Returns whether "Find in folder" was picked.
    fn explorer_more_menu(&mut self, ui: &mut egui::Ui, cwd: &Path) -> bool {
        let mut focus_find = false;
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Reveal in file manager",
            icon: "FolderOpen",
            shortcut: "",
        })
        .clicked()
        {
            self.queue_workspace(workspace_ops::Op::Reveal(cwd.to_path_buf()));
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Copy path",
            icon: "Copy",
            shortcut: "",
        })
        .clicked()
        {
            ui.ctx().copy_text(cwd.display().to_string());
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Copy relative path",
            icon: "Copy",
            shortcut: "",
        })
        .clicked()
        {
            let root = self.op_root().unwrap_or_else(|| cwd.to_path_buf());
            ui.ctx()
                .copy_text(workspace_ops::relative_display(&root, cwd));
            ui.close();
        }
        ui.separator();
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Find in folder",
            icon: "Search",
            shortcut: "",
        })
        .clicked()
        {
            focus_find = true;
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Collapse all",
            icon: "ChevronsDownUp",
            shortcut: "",
        })
        .clicked()
        {
            self.expanded_dirs.clear();
            ui.close();
        }
        self.explorer_bookmarks_menu(ui, cwd);
        ui.separator();
        self.explorer_view_options(ui);
        focus_find
    }

    fn explorer_view_options(&mut self, ui: &mut egui::Ui) {
        let mut follow = self.preferences.explorer_autoreveal;
        if ui
            .checkbox(&mut follow, "Always select opened file")
            .clicked()
        {
            self.preferences.explorer_autoreveal = follow;
            self.explorer_followed = None;
            ui.close();
        }
        let mut compact = !self.preferences.explorer_flat_folders;
        if ui.checkbox(&mut compact, "Compact folders").clicked() {
            self.preferences.explorer_flat_folders = !compact;
            ui.close();
        }
        let view = &mut self.preferences.explorer_view;
        let mut folders_first = !view.folders_mixed;
        if ui.checkbox(&mut folders_first, "Folders first").clicked() {
            view.folders_mixed = !folders_first;
            ui.close();
        }
        if ui
            .checkbox(&mut view.sort_by_type, "Sort by type")
            .clicked()
        {
            ui.close();
        }
        let mut nesting = !view.nesting_off;
        if ui.checkbox(&mut nesting, "Nest related files").clicked() {
            view.nesting_off = !nesting;
            ui.close();
        }
        if ui
            .checkbox(&mut view.changed_only, "Changed files only")
            .clicked()
        {
            ui.close();
        }
    }

    /// Bookmarked files and folders. A pick reveals it in the tree and
    /// opens a file.
    fn explorer_bookmarks_menu(&mut self, ui: &mut egui::Ui, cwd: &Path) {
        if self.preferences.explorer_bookmarks.is_empty() {
            return;
        }
        let mut picked: Option<PathBuf> = None;
        ui.menu_button("Bookmarks", |ui| {
            for path in &self.preferences.explorer_bookmarks {
                let label = workspace_ops::relative_display(cwd, path);
                if ui
                    .button(label)
                    .on_hover_text(path.display().to_string())
                    .clicked()
                {
                    picked = Some(path.clone());
                    ui.close();
                }
            }
        });
        if let Some(path) = picked {
            self.explorer_jump_to(ui, cwd, path);
        }
    }

    /// Open the folders above `path`, select it, and open it if a file.
    fn explorer_jump_to(&mut self, ui: &mut egui::Ui, cwd: &Path, path: PathBuf) {
        if !path.starts_with(cwd) {
            self.error = Some(format!("{} is outside the Explorer folder", path.display()));
            return;
        }
        let mut parent = path.parent();
        while let Some(dir) = parent.filter(|dir| dir.starts_with(cwd) && *dir != cwd) {
            self.expanded_dirs.insert(dir.to_path_buf());
            parent = dir.parent();
        }
        self.explorer_query.clear();
        self.preferences.explorer_search_mode = ExplorerSearchMode::Names;
        self.explorer_selection = vec![path.clone()];
        self.explorer_anchor = Some(path.clone());
        self.explorer_scroll_to = Some(path.clone());
        if path.is_dir() {
            self.expanded_dirs.insert(path);
        } else {
            self.activate_file_action(ui, &path, FileAction::Open);
        }
    }

    fn explorer_toolbar_buttons(&self, ui: &mut egui::Ui) -> ToolbarButtons {
        let reveal_enabled = self.explorer_reveal_candidate().is_some();
        let reveal_disabled_note = if self.focused_local_file().is_some() {
            "The focused file is outside the Explorer folder"
        } else {
            "Focus a local file to reveal it in Explorer"
        };
        let ignored_icon = if self.preferences.show_ignored {
            "Eye"
        } else {
            "EyeOff"
        };
        let ignored = appearance::selectable_icon(appearance::SelectableIconInput {
            ui,
            icon: ignored_icon,
            tip: "Show ignored files (excluded by Git ignore rules and Git metadata)",
            selected: self.preferences.show_ignored,
        });
        let reveal = ui
            .add_enabled_ui(reveal_enabled, |ui| {
                appearance::sidebar_action(ui, "LocateFixed", "Reveal current file in Explorer")
            })
            .inner
            .on_disabled_hover_text(reveal_disabled_note);
        let refresh = appearance::sidebar_action(ui, "RefreshCw", "Refresh");
        let collapse = appearance::sidebar_action(ui, "ChevronsDownUp", "Collapse all");
        let new_folder = appearance::sidebar_action(ui, "Folder", "New folder");
        let new_file = appearance::sidebar_action(ui, "File", "New file");
        ToolbarButtons {
            ignored,
            reveal,
            refresh,
            collapse,
            new_folder,
            new_file,
        }
    }

    #[cfg(feature = "test-support")]
    fn record_toolbar_buttons(&self, ui: &egui::Ui, buttons: &ToolbarButtons, host: egui::Id) {
        diagnostics::record(ui.ctx(), "explorer-new-file", buttons.new_file.rect);
        diagnostics::record(ui.ctx(), "explorer-new-folder", buttons.new_folder.rect);
        diagnostics::record(ui.ctx(), "explorer-collapse", buttons.collapse.rect);
        diagnostics::record(ui.ctx(), "explorer-refresh", buttons.refresh.rect);
        diagnostics::record(ui.ctx(), "explorer-reveal-current", buttons.reveal.rect);
        if self
            .explorer_reveal
            .as_ref()
            .is_some_and(|active| active.host == host)
        {
            diagnostics::record(ui.ctx(), "explorer-reveal-active", buttons.reveal.rect);
        }
        diagnostics::record(ui.ctx(), "explorer-show-ignored", buttons.ignored.rect);
    }

    /// Act on toolbar clicks for the folder `cwd` shown in `host`.
    fn explorer_toolbar_clicks(
        &mut self,
        ui: &egui::Ui,
        buttons: &ToolbarButtons,
        (cwd, host): (&Path, egui::Id),
    ) {
        if buttons.ignored.clicked() {
            self.preferences.show_ignored = !self.preferences.show_ignored;
            self.explorer_search_last = None;
        }
        if buttons.reveal.clicked() && self.reveal_current_file_in_explorer(host) {
            ui.ctx().request_repaint();
        }
        // New items land in the selected folder, like `IntelliJ`.
        if buttons.new_file.clicked() {
            self.open_name_prompt(workspace_ops::NamePrompt::File {
                dir: self.explorer_target_dir(cwd),
                name: String::new(),
            });
        }
        if buttons.new_folder.clicked() {
            self.open_name_prompt(workspace_ops::NamePrompt::Folder {
                dir: self.explorer_target_dir(cwd),
                name: String::new(),
            });
        }
        if buttons.collapse.clicked() {
            self.expanded_dirs.clear();
        }
        if buttons.refresh.clicked() {
            self.refresh_request = None;
        }
    }

    /// Search icon, query field, and (Contents mode) match toggles.
    fn explorer_search_row(&mut self, ui: &mut egui::Ui, contents: bool, focus_find: bool) {
        ui.spacing_mut().item_spacing.x = 4.0;
        let icon = 18.0;
        ui.add_sized(
            [icon, icon],
            egui::Image::new(icons::source("Search"))
                .tint(appearance::color(&self.theme.secondary)),
        );
        let toggles = if contents { 78.0 } else { 0.0 };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Button padding is a few points"
        )]
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
        if focus_find || std::mem::take(&mut self.explorer_focus_find) {
            find.request_focus();
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "explorer-search", find.rect);
        if contents {
            self.explorer_match_toggles(ui);
        }
    }

    fn explorer_match_toggles(&mut self, ui: &mut egui::Ui) {
        let case = explorer_toggle(ExplorerToggleInput {
            ui,
            text: "Aa",
            tip: "Match case",
            selected: self.preferences.explorer_match_case,
            target: "explorer-match-case",
        });
        let word = explorer_toggle(ExplorerToggleInput {
            ui,
            text: "ab",
            tip: "Match whole word",
            selected: self.preferences.explorer_whole_word,
            target: "explorer-whole-word",
        });
        let regex = explorer_toggle(ExplorerToggleInput {
            ui,
            text: ".*",
            tip: "Use regular expression",
            selected: self.preferences.explorer_regex,
            target: "explorer-regex",
        });
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

    /// Include/exclude glob fields for a Contents search.
    fn explorer_search_filters(&mut self, ui: &mut egui::Ui) {
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
}
