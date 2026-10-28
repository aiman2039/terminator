use super::super::super::*;
use crate::app::app_state::CommitFiles;
use std::collections::HashMap;
use std::path::Path;

/// The commit log's read-only inputs for one frame.
struct CommitLogPane<'a> {
    log: &'a git_log::CommitLog,
    theme: &'a AppearanceConfig,
    commit_files: &'a HashMap<String, Option<CommitFiles>>,
    cwd: &'a Path,
}

/// What the commit log asked for this frame: changed files to fetch for
/// a commit, or a file change to open.
#[derive(Default)]
struct CommitLogPick {
    fetch: Option<String>,
    open: Option<Tab>,
}

impl App {
    pub(in super::super) fn commit_log_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::CommitLog { cwd } = tab else {
            return;
        };
        let mut pick = CommitLogPick::default();
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
        let pane = CommitLogPane {
            log,
            theme: &self.theme,
            commit_files: &self.commit_files,
            cwd,
        };
        let selected = &mut self.selected_commit;
        ui.columns(3, |cols| {
            // Left: branch tree
            let Some(col) = cols.get_mut(0) else {
                return;
            };
            pane.branches(col);
            // Center: commit list
            let Some(col) = cols.get_mut(1) else {
                return;
            };
            pane.commits(col, selected);
            // Right: commit details
            let Some(col) = cols.get_mut(2) else {
                return;
            };
            pane.details(col, selected, &mut pick);
        });
        if let Some(hash) = pick.fetch {
            self.commit_files.insert(hash.clone(), None);
            let _ = self.jobs.send(Job::CommitFiles(cwd.clone(), hash));
        }
        if let Some(tab) = pick.open {
            self.open_git_tab(tab);
        }
    }
}

impl CommitLogPane<'_> {
    fn branches(&self, col: &mut egui::Ui) {
        col.strong("Branches");
        col.separator();
        appearance::sidebar_scroll("branches").show(col, |ui| {
            for rf in &self.log.refs {
                let icon = match rf.kind {
                    git_log::RefKind::LocalBranch => "\u{2398}",
                    git_log::RefKind::RemoteBranch => "\u{1F310}",
                    git_log::RefKind::Tag => "\u{2B50}",
                };
                let selected = rf.name == self.log.active_branch;
                let text = if selected {
                    format!("{} {} \u{2713}", icon, rf.name)
                } else {
                    format!("{} {}", icon, rf.name)
                };
                let _ = ui.selectable_label(selected, &text);
            }
        });
    }

    fn commits(&self, col: &mut egui::Ui, selected: &mut Option<String>) {
        col.strong("Commits");
        col.separator();
        let selected_hash = selected.clone();
        appearance::sidebar_scroll("commits").show(col, |ui| {
            for commit in &self.log.commits {
                let is_selected = selected_hash.as_deref() == Some(&commit.hash);
                if self.commit_row(ui, commit, is_selected).clicked() {
                    *selected = Some(commit.hash.clone());
                }
            }
        });
    }

    fn commit_row(
        &self,
        ui: &mut egui::Ui,
        commit: &git_log::CommitEntry,
        selected: bool,
    ) -> egui::Response {
        ui.horizontal(|ui| {
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
        })
        .response
    }

    fn details(&self, col: &mut egui::Ui, selected: &mut Option<String>, pick: &mut CommitLogPick) {
        col.strong("Details");
        col.separator();
        let Some(hash) = selected.as_ref() else {
            if let Some(first) = self.log.commits.first() {
                *selected = Some(first.hash.clone());
            }
            return;
        };
        let Some(commit) = self.log.commits.iter().find(|c| &c.hash == hash) else {
            return;
        };
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
        self.changed_files(col, hash, pick);
    }

    /// Changed files: each opens its change in this commit.
    fn changed_files(&self, col: &mut egui::Ui, hash: &str, pick: &mut CommitLogPick) {
        let files = match self.commit_files.get(hash) {
            None => {
                pick.fetch = Some(hash.to_owned());
                col.weak("Loading files…");
                return;
            }
            Some(None) => {
                col.weak("Loading files…");
                return;
            }
            Some(Some(Err(error))) => {
                col.colored_label(appearance::color(&self.theme.status_failed), error);
                return;
            }
            Some(Some(Ok(files))) => files,
        };
        col.weak(format!(
            "{} file{} changed",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        ));
        let cwd = self.cwd;
        for (status, path) in files {
            let name = path.strip_prefix(cwd).unwrap_or(path).display().to_string();
            if col
                .add(
                    egui::Button::selectable(
                        false,
                        egui::RichText::new(format!("{status}  {name}")).monospace(),
                    )
                    .truncate(),
                )
                .on_hover_text("Show this file's change in the commit")
                .clicked()
            {
                pick.open = Some(Tab::RevisionDiff {
                    cwd: cwd.to_path_buf(),
                    path: path.clone(),
                    left: format!("{hash}^"),
                    right: hash.to_owned(),
                });
            }
        }
    }
}
