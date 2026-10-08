use super::super::*;
use super::diff_paint::{DiffColors, paint_diff_document, paint_markdown_diff_preview};
impl App {
    pub(super) fn image_view(&mut self, ui: &mut egui::Ui, path: &std::path::Path) {
        self.visible_images.insert(path.into());
        let mut as_text = false;
        let mut reload = false;
        let mut fit = false;
        let mut actual = false;
        appearance::wrapping_path_row(ui, &services::compact_path(path), |ui| {
            reload = ui.button("Reload").clicked();
            as_text = ui.button("Open as text").clicked();
            if ui.button("Open externally").clicked() {
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
            back = ui.button("Back").clicked();
            forward = ui.button("Forward").clicked();
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
            reload = ui.button("Reload").clicked();
            as_text = target.file().is_some() && ui.button("Open as text").clicked();
            let open = ui.button("Open in browser");
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
    pub(super) fn diff_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::Diff {
            cwd,
            path,
            staged: _,
        } = tab
        else {
            return;
        };
        let is_md = crate::markdown::supported(path);
        let key = tab.key();
        let split = self.diff_split.contains(&key);
        ui.horizontal(|ui| {
            ui.strong(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            );
            if let Some(letter) = self.context.as_ref().and_then(|context| {
                context
                    .decorations
                    .get(&cwd.join(path))
                    .copied()
                    .filter(|letter| *letter != ' ')
            }) {
                ui.colored_label(
                    sidebar_ui::git_color(&self.theme, letter),
                    letter.to_string(),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let more = appearance::menu_button(ui, "…", |ui| {
                    if appearance::menu_item(ui, "Unified", "Rows2", "").clicked() {
                        self.diff_split.remove(&key);
                        ui.close();
                    }
                    ui.separator();
                    let ignore = self.diff_ignore_ws.contains(&key);
                    if appearance::menu_item(
                        ui,
                        if ignore {
                            "Show whitespace"
                        } else {
                            "Ignore whitespace"
                        },
                        "Eraser",
                        "",
                    )
                    .clicked()
                    {
                        if ignore {
                            self.diff_ignore_ws.remove(&key);
                        } else {
                            self.diff_ignore_ws.insert(key.clone());
                        }
                        self.diff_preview.remove(&key);
                        self.loading.insert(key.clone());
                        self.diffs.remove(&key);
                        let _ = self.jobs.send(Job::Diff(tab.clone()));
                        ui.close();
                    }
                    if is_md {
                        let preview = self.diff_preview.contains(&key);
                        if appearance::menu_item(
                            ui,
                            if preview { "Hide preview" } else { "Preview" },
                            "FileText",
                            "",
                        )
                        .clicked()
                        {
                            if preview {
                                self.diff_preview.remove(&key);
                            } else {
                                self.diff_preview.insert(key.clone());
                            }
                            ui.close();
                        }
                    }
                    ui.separator();
                    if appearance::menu_item(ui, "Refresh", "RefreshCw", "").clicked() {
                        self.diff_preview.remove(&key);
                        self.diff_ignore_ws.remove(&key);
                        self.loading.insert(key.clone());
                        let _ = self.jobs.send(Job::Diff(tab.clone()));
                        ui.close();
                    }
                })
                .response
                .on_hover_text("Diff view");
                let _ = more;
                let side_by_side = ui.selectable_label(split, "Side by side");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "diff-side-by-side", side_by_side.rect);
                if side_by_side.clicked() {
                    if split {
                        self.diff_split.remove(&key);
                    } else {
                        self.diff_split.insert(key.clone());
                    }
                }
            });
        });
        if !self.diffs.contains_key(&key) && self.loading.insert(key.clone()) {
            let _ = self.jobs.send(Job::Diff(tab.clone()));
        }
        match self.diffs.get(&key) {
            Some(Ok(doc)) => {
                if is_md && self.diff_preview.contains(&key) {
                    let path = cwd.join(path);
                    if let Some(link) =
                        paint_markdown_diff_preview(ui, doc, &mut self.markdown, &path, &key)
                    {
                        match link {
                            markdown::Link::File(path) => self.open_file(path, None, None, false),
                            markdown::Link::Web(url) => {
                                let _ = self.jobs.send(Job::Browser(url));
                            }
                        }
                    }
                } else {
                    let split = self.diff_split.contains(&key);
                    let colors = DiffColors {
                        added: appearance::color(&self.theme.git_added),
                        deleted: appearance::color(&self.theme.git_deleted),
                        accent: appearance::color(&self.theme.accent),
                        text: appearance::color(&self.theme.text),
                    };
                    let doc = std::sync::Arc::clone(doc);
                    let mut sync = self.diff_split_scroll.get(&key).copied().unwrap_or(0.0);
                    paint_diff_document(
                        ui,
                        &doc,
                        split,
                        colors,
                        &key,
                        &mut self.diff_split_ratio,
                        &mut sync,
                    );
                    self.diff_split_scroll.insert(key.clone(), sync);
                }
            }
            Some(Err(error)) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
            }
            None => {
                ui.spinner();
            }
        }
    }

    pub(super) fn commit_log_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::CommitLog { cwd } = tab else { return };
        let key = tab.key();
        let log = self.git_logs.entry(key.clone()).or_insert_with(|| {
            match git_log::fetch_log(cwd.as_path()) {
                Ok(log) => log,
                Err(e) => git_log::CommitLog {
                    error: Some(e),
                    ..Default::default()
                },
            }
        });
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
                        git_log::RefKind::Head => "\u{1F4CC}",
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
                }
            } else if let Some(first) = log.commits.first() {
                self.selected_commit = Some(first.hash.clone());
            }
        });
    }
    pub(super) fn blame_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::Blame { cwd, path } = tab else {
            return;
        };
        ui.weak(format!("Blame: {}", path.display()));
        ui.separator();
        match git_log::fetch_blame(cwd, path) {
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
            diagnostics::record(ui.ctx(), "Save and close", bar.save.rect);
            diagnostics::record(ui.ctx(), "Discard changes", bar.discard.rect);
            diagnostics::record(ui.ctx(), "Cancel", bar.cancel.rect);
        }
        if let Some(choice) = bar.choice() {
            self.apply_unsaved_close_choice(choice, target, ids);
        }
    }
}
