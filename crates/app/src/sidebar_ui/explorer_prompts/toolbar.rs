//! The Explorer toolbar: folder actions, search field, and filters.
use super::*;

impl App {
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
                    let mut compact = !self.preferences.explorer_flat_folders;
                    if ui.checkbox(&mut compact, "Compact folders").clicked() {
                        self.preferences.explorer_flat_folders = !compact;
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
            if focus_find {
                find.request_focus();
            }
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "explorer-search", find.rect);
            if contents {
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
}
