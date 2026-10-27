use super::super::*;
impl App {
    pub(super) fn image_view(&mut self, ui: &mut egui::Ui, path: &std::path::Path) {
        self.visible_images.insert(path.into());
        let mut as_text = false;
        let mut reload = false;
        let mut fit = false;
        let mut actual = false;
        appearance::wrapping_path_row(ui, &services::compact_path(path), |ui| {
            reload = appearance::sidebar_action(ui, "RefreshCw", "Reload image").clicked();
            as_text = appearance::sidebar_action(ui, "FileText", "Open as text").clicked();
            if appearance::sidebar_action(ui, "ExternalLink", "Open externally").clicked() {
                let _ = self.jobs.send(Job::External(path.into()));
            }
            if let Some(texture) = self
                .images
                .get(path)
                .and_then(|preview| preview.texture.as_ref())
            {
                ui.weak(format!(
                    "{} × {} pixels",
                    texture.size()[0],
                    texture.size()[1]
                ));
                let fit_btn = ui.button("Fit");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "image-fit", fit_btn.rect);
                fit = fit_btn.clicked();
                actual = ui.button("100%").clicked();
            }
        });
        if as_text {
            self.open_file_mode(path.into(), None, None, false, true);
        }
        if reload && let Some(preview) = self.images.get_mut(path) {
            if let Some(cancel) = preview.cancellation.take() {
                cancel.cancel();
            }
            preview.loading = false;
            preview.error = None;
        }
        if !self.images.contains_key(path) && self.images.len() >= 8 {
            ui.weak("Close another image preview to load this image.");
            return;
        }
        let preview = self.images.entry(path.into()).or_default();
        if reload || (!preview.loading && preview.texture.is_none() && preview.error.is_none()) {
            self.image_generation = self.image_generation.wrapping_add(1);
            preview.generation = self.image_generation;
            preview.cancellation = self
                .image_jobs
                .try_send((path.into(), preview.generation))
                .ok();
            preview.loading = preview.cancellation.is_some();
            if !preview.loading {
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
        }
        if fit {
            preview.scene = egui::Rect::NOTHING;
        }
        if actual && let Some(size) = preview.texture.as_ref().map(egui::TextureHandle::size_vec2) {
            let center = size.to_pos2();
            preview.scene = egui::Rect::from_center_size(
                egui::pos2(center.x * 0.5, center.y * 0.5),
                ui.available_size(),
            );
        }
        preview.show(ui);
    }
    pub(super) fn browser_view(&mut self, ui: &mut egui::Ui, key: String, target: &BrowserTarget) {
        let href = crate::browser::href(target).unwrap_or_else(|_| target.title());
        let mut draft = self
            .browser_urls
            .remove(&key)
            .unwrap_or_else(|| href.clone());
        if draft.is_empty() {
            draft = href;
        }
        let mut as_text = false;
        let mut system = false;
        let mut reload = false;
        let mut back = false;
        let mut forward = false;
        let mut go = false;
        ui.horizontal_wrapped(|ui| {
            back = appearance::sidebar_action(ui, "ArrowLeft", "Back").clicked();
            forward = appearance::sidebar_action(ui, "ArrowRight", "Forward").clicked();
            reload = appearance::sidebar_action(ui, "RefreshCw", "Reload").clicked();
            let url = ui.add(
                appearance::singleline(&mut draft)
                    .desired_width(240.0)
                    .hint_text("https://"),
            );
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "browser-url", url.rect);
            let go_btn = ui.button("Go");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "browser-go", go_btn.rect);
            go = go_btn.clicked()
                || (url.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            as_text = target.file().is_some()
                && appearance::sidebar_action(ui, "FileText", "Open as text").clicked();
            let open = appearance::sidebar_action(ui, "ExternalLink", "Open in system browser");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "html-open-browser", open.rect);
            system = open.clicked();
        });
        if go
            && let Ok(next) = BrowserTarget::from_http_url(&draft)
            && next != *target
        {
            self.browser_submit = Some((key.clone(), next));
        }
        self.browser_urls.insert(key.clone(), draft);
        if back {
            self.browser_host.go_back(&key);
        }
        if forward {
            self.browser_host.go_forward(&key);
        }
        if reload && !self.browser_host.reload_view(&key) {
            self.browser_host.drop_view(&key);
        }
        if as_text && let Some(path) = target.file() {
            self.open_file_mode(path.into(), None, None, false, true);
        }
        if system {
            match target {
                BrowserTarget::File(path) => self.open_in_browser(path),
                BrowserTarget::Url(url) => {
                    let _ = self.jobs.send(Job::Browser(url.clone()));
                }
            }
        }
        if let Some(error) = self.browser_host.error(&key) {
            ui.colored_label(ui.visuals().error_fg_color, error);
            ui.weak("Open in browser to view the page in your system browser.");
        }
        let rect = ui.available_rect_before_wrap();
        let _ = ui.allocate_rect(rect, egui::Sense::hover());
        self.visible_browsers
            .push(crate::browser_host::VisibleBrowser {
                key,
                target: target.clone(),
                rect,
            });
    }
    /// Start loading `tab`'s Git view if needed; shows a spinner and returns
    /// false until the result arrives.
    fn git_view_ready(&mut self, ui: &mut egui::Ui, key: &str, tab: &Tab) -> bool {
        if !self.git_views.contains_key(key) {
            self.git_views
                .insert(key.to_owned(), crate::GitView::Loading);
            let _ = self.jobs.send(Job::GitView(tab.clone()));
        }
        if matches!(self.git_views.get(key), Some(crate::GitView::Loading)) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Loading…");
            });
            return false;
        }
        true
    }
    pub(super) fn commit_log_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::CommitLog { cwd } = tab else {
            return;
        };
        let mut open: Option<Tab> = None;
        let mut fetch: Option<String> = None;
        let key = tab.key();
        if !self.git_view_ready(ui, &key, tab) {
            return;
        }
        let Some(crate::GitView::Log(log)) = self.git_views.get(&key) else {
            return;
        };
        if let Some(error) = &log.error {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
            return;
        }
        ui.columns(3, |cols| {
            // Left: branch tree
            let Some(col) = cols.get_mut(0) else {
                return;
            };
            col.strong("Branches");
            col.separator();
            appearance::sidebar_scroll("branches").show(col, |ui| {
                for rf in &log.refs {
                    let icon = match rf.kind {
                        git_log::RefKind::LocalBranch => "\u{2398}",
                        git_log::RefKind::RemoteBranch => "\u{1F310}",
                        git_log::RefKind::Tag => "\u{2B50}",
                    };
                    let selected = rf.name == log.active_branch;
                    let text = if selected {
                        format!("{} {} \u{2713}", icon, rf.name)
                    } else {
                        format!("{} {}", icon, rf.name)
                    };
                    let _ = ui.selectable_label(selected, &text);
                }
            });
            // Center: commit list
            let Some(col) = cols.get_mut(1) else {
                return;
            };
            col.strong("Commits");
            col.separator();
            let selected_hash = self.selected_commit.clone();
            appearance::sidebar_scroll("commits").show(col, |ui| {
                for commit in &log.commits {
                    let selected = selected_hash.as_deref() == Some(&commit.hash);
                    let response = ui.horizontal(|ui| {
                        if selected {
                            ui.colored_label(appearance::color(&self.theme.accent), "\u{25CF}");
                        } else {
                            ui.add(egui::Label::new(
                                egui::RichText::new("\u{25CF}")
                                    .size(10.0)
                                    .color(appearance::color(&self.theme.text)),
                            ));
                        }
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&commit.subject));
                            ui.horizontal(|ui| {
                                ui.weak(&commit.short_hash);
                                ui.weak(&commit.date);
                                ui.weak(&commit.author);
                            });
                        });
                    });
                    if response.response.clicked() {
                        self.selected_commit = Some(commit.hash.clone());
                    }
                }
            });
            // Right: commit details
            let Some(col) = cols.get_mut(2) else {
                return;
            };
            col.strong("Details");
            col.separator();
            if let Some(hash) = &self.selected_commit {
                if let Some(commit) = log.commits.iter().find(|c| &c.hash == hash) {
                    col.horizontal(|ui| {
                        ui.weak("Hash: ");
                        ui.label(&commit.short_hash);
                    });
                    col.horizontal(|ui| {
                        ui.weak("Author: ");
                        ui.label(&commit.author);
                    });
                    col.horizontal(|ui| {
                        ui.weak("Date: ");
                        ui.label(&commit.date);
                    });
                    col.separator();
                    col.label(&commit.message);
                    col.separator();
                    // Changed files: each opens its change in this commit.
                    match self.commit_files.get(hash) {
                        None => {
                            fetch = Some(hash.clone());
                            col.weak("Loading files…");
                        }
                        Some(None) => {
                            col.weak("Loading files…");
                        }
                        Some(Some(Err(error))) => {
                            col.colored_label(appearance::color(&self.theme.status_failed), error);
                        }
                        Some(Some(Ok(files))) => {
                            col.weak(format!(
                                "{} file{} changed",
                                files.len(),
                                if files.len() == 1 { "" } else { "s" }
                            ));
                            for (status, path) in files {
                                let name =
                                    path.strip_prefix(cwd).unwrap_or(path).display().to_string();
                                if col
                                    .add(
                                        egui::Button::selectable(
                                            false,
                                            egui::RichText::new(format!("{status}  {name}"))
                                                .monospace(),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text("Show this file's change in the commit")
                                    .clicked()
                                {
                                    open = Some(Tab::RevisionDiff {
                                        cwd: cwd.clone(),
                                        path: path.clone(),
                                        left: format!("{hash}^"),
                                        right: hash.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
            } else if let Some(first) = log.commits.first() {
                self.selected_commit = Some(first.hash.clone());
            }
        });
        if let Some(hash) = fetch {
            self.commit_files.insert(hash.clone(), None);
            let _ = self.jobs.send(Job::CommitFiles(cwd.clone(), hash));
        }
        if let Some(tab) = open {
            self.open_git_tab(tab);
        }
    }
    /// Read-only diff of one file between two revisions.
    pub(crate) fn revision_diff_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::RevisionDiff { cwd, path, .. } = tab else {
            return;
        };
        let key = tab.key();
        let mut refresh = false;
        let split = self.diff_split.contains(&key);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add(egui::Label::new(egui::RichText::new(tab.git_title()).strong()).truncate())
                .on_hover_text(cwd.join(path).display().to_string());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                refresh = appearance::sidebar_action(ui, "RefreshCw", "Reload").clicked();
                if appearance::selectable_icon(ui, "Columns2", "Side by side", split).clicked() {
                    self.diff_split.insert(key.clone());
                }
                if appearance::selectable_icon(ui, "Rows2", "Unified", !split).clicked() {
                    self.diff_split.remove(&key);
                }
            });
        });
        if refresh {
            self.git_views.remove(&key);
        }
        if !self.git_view_ready(ui, &key, tab) {
            return;
        }
        let Some(crate::GitView::Revision(result)) = self.git_views.get(&key) else {
            return;
        };
        let doc = match result {
            Ok(doc) => std::sync::Arc::clone(doc),
            Err(error) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
                return;
            }
        };
        let split = self.diff_split.contains(&key);
        super::diff_paint::comparison_headers(
            ui,
            &doc,
            split,
            self.diff_split_ratio,
            &cwd.join(path),
            false,
            false,
        );
        if doc.hunks.is_empty() {
            ui.weak("No differences");
            return;
        }
        let colors = super::diff_paint::DiffColors {
            added: appearance::color(&self.theme.git_added),
            deleted: appearance::color(&self.theme.git_deleted),
            accent: appearance::color(&self.theme.accent),
            text: appearance::color(&self.theme.text),
        };
        let mut sync = self.diff_split_scroll.get(&key).copied().unwrap_or(0.0);
        super::diff_paint::paint_diff_document(
            ui,
            &doc,
            split,
            colors,
            &key,
            &mut self.diff_split_ratio,
            &mut sync,
        );
        self.diff_split_scroll.insert(key, sync);
    }

    /// Commits that touched a file; each opens its change, or a comparison
    /// of that revision with the working tree.
    pub(crate) fn file_history_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::FileHistory { cwd, path } = tab else {
            return;
        };
        let key = tab.key();
        ui.horizontal(|ui| {
            ui.strong(tab.git_title());
            ui.weak(cwd.join(path).display().to_string());
        });
        ui.separator();
        if !self.git_view_ready(ui, &key, tab) {
            return;
        }
        let Some(crate::GitView::History(result)) = self.git_views.get(&key) else {
            return;
        };
        let entries = match result {
            Ok(entries) => entries.clone(),
            Err(error) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
                return;
            }
        };
        if entries.is_empty() {
            ui.weak("No commits touch this file.");
            return;
        }
        let mut open: Option<Tab> = None;
        appearance::sidebar_scroll("file-history").show(ui, |ui| {
            for entry in &entries {
                ui.horizontal(|ui| {
                    let row = ui
                        .add(egui::Button::selectable(false, &entry.subject).truncate())
                        .on_hover_text(format!(
                            "{} · {} · {}\nClick to show this commit's change",
                            entry.short_hash, entry.author, entry.date
                        ));
                    if row.clicked() {
                        open = Some(Tab::RevisionDiff {
                            cwd: cwd.clone(),
                            path: path.clone(),
                            left: format!("{}^", entry.hash),
                            right: entry.hash.clone(),
                        });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if appearance::sidebar_action(
                            ui,
                            "FileDiff",
                            "Compare with the working tree",
                        )
                        .clicked()
                        {
                            open = Some(Tab::RevisionDiff {
                                cwd: cwd.clone(),
                                path: path.clone(),
                                left: entry.hash.clone(),
                                right: String::new(),
                            });
                        }
                        ui.weak(&entry.date);
                        ui.weak(&entry.author);
                        ui.monospace(&entry.short_hash);
                    });
                });
            }
        });
        if let Some(tab) = open {
            self.open_git_tab(tab);
        }
    }

    pub(crate) fn blame_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::Blame { path, .. } = tab else {
            return;
        };
        ui.weak(format!("Blame: {}", path.display()));
        ui.separator();
        let key = tab.key();
        if !self.git_view_ready(ui, &key, tab) {
            return;
        }
        let Some(crate::GitView::Blame(result)) = self.git_views.get(&key) else {
            return;
        };
        match result {
            Ok(data) => {
                appearance::sidebar_scroll("blame").show(ui, |ui| {
                    for entry in &data.entries {
                        ui.horizontal(|ui| {
                            let shown = entry.hash.len().min(7);
                            ui.weak(entry.hash.get(..shown).unwrap_or(entry.hash.as_str()));
                            ui.weak(&entry.author);
                            ui.weak(&entry.date);
                            ui.weak("| ");
                            ui.monospace(&entry.code);
                        });
                    }
                });
            }
            Err(e) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), e);
            }
        }
    }
}
impl App {
    pub(super) fn draw_unsaved_close_bar(&mut self, ui: &mut egui::Ui, sid: &str) {
        let Some((target, ids, error)) = self.unsaved_close_prompt(sid) else {
            return;
        };
        let enabled = !self.editor_close_busy(&ids);
        let bar = appearance::unsaved_close_bar(
            ui,
            appearance::UnsavedCloseBar {
                theme: &self.theme,
                message: &error,
                enabled,
            },
        );
        #[cfg(feature = "test-support")]
        {
            diagnostics::record_response(ui.ctx(), "Save and close", &bar.save);
            diagnostics::record_response(ui.ctx(), "Discard changes", &bar.discard);
            diagnostics::record_response(ui.ctx(), "Cancel", &bar.cancel);
        }
        if let Some(choice) = bar.choice() {
            self.apply_unsaved_close_choice(choice, target, ids);
        }
    }
}
