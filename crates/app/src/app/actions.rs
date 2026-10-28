use super::super::appearance;
use crate::app::OpenFileInput;
use crate::app::OpenFileModeInput;
use crate::native_editor::OpenNativeInput;
use eframe::egui::{self};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) struct TerminalActionInput<'a> {
    pub(crate) ctx: &'a egui::Context,
    pub(crate) session: &'a Session,
    pub(crate) target: &'a services::Target,
    pub(crate) action: FileAction,
}

pub(crate) struct FileActionInput<'a> {
    pub(crate) ui: &'a egui::Ui,
    pub(crate) action: FileAction,
    pub(crate) path: &'a std::path::Path,
    pub(crate) line: Option<u32>,
}

pub(crate) struct NoteFileActivationInput {
    pub(crate) path: PathBuf,
    pub(crate) staged: Option<bool>,
    pub(crate) action: FileAction,
    pub(crate) interval: Duration,
}

use terminator_core::*;

use super::super::*;
impl App {
    pub(crate) fn remove_tab(&mut self, sid: &str) {
        for workspace in self.layouts.values_mut() {
            workspace.remove_session(sid);
        }
        let target = Tab::Terminal(sid.into());
        for window in &mut self.floating {
            if let Some(dock) = window.dock.as_mut() {
                while let Some(path) = dock.find_tab(&target) {
                    dock.remove_tab(path);
                }
            }
        }
        self.floating.retain(|window| {
            window
                .dock
                .as_ref()
                .is_none_or(|dock| dock.iter_all_tabs().next().is_some())
        });
        self.backends.remove(sid);
        self.drop_strip_session(sid);
        if self.active_session.as_deref() == Some(sid) {
            self.active_session = None;
            // The closed terminal was the selection. A main pane remembered
            // from earlier must not block recovery.
            self.non_terminal_selected = false;
        }
    }

    pub(crate) fn terminal_action(&mut self, input: TerminalActionInput<'_>) {
        let TerminalActionInput {
            ctx,
            session,
            target,
            action,
        } = input;
        if action == FileAction::Copy {
            ctx.copy_text(target.display());
            return;
        }
        match target {
            services::Target::Url(url) => {
                let _ = self.jobs.send(Job::Browser(url.clone()));
            }
            services::Target::File(path, line, column) => {
                if action == FileAction::Browser {
                    self.open_in_browser(path);
                    return;
                }
                if image_preview::supported(path)
                    && matches!(
                        action,
                        FileAction::Open | FileAction::Split | FileAction::Here
                    )
                {
                    self.open_image(&session.project_id, path.clone(), Self::split_for(action));
                    return;
                }
                if crate::browser::supported_file(path)
                    && matches!(
                        action,
                        FileAction::Open | FileAction::Split | FileAction::Here
                    )
                {
                    self.open_html(&session.project_id, path.clone(), Self::split_for(action));
                    return;
                }
                if action == FileAction::External
                    || self.state.settings.editor_mode == EditorMode::External
                {
                    let _ = self.jobs.send(Job::External(path.clone()));
                } else if self.state.settings.editor_mode == EditorMode::Native {
                    self.open_native(OpenNativeInput {
                        project: &session.project_id,
                        path: path.clone(),
                        line: *line,
                        split: Self::split_for(action),
                    });
                } else {
                    if Self::split_for(action).is_none()
                        && self.reuse_file_session(&session.project_id, path)
                    {
                        return;
                    }
                    let origin = Tab::Terminal(session.id.clone());
                    let after = self.editor_target(
                        &session.project_id,
                        Some(&origin),
                        Self::split_for(action),
                    );
                    let _ = self.jobs.send(Job::rpc(
                        Request::Create {
                            project: session.project_id.clone(),
                            cwd: Some(session.cwd.clone()),
                            file: Some(path.clone()),
                            line: *line,
                            column: *column,
                            editor: true,
                        },
                        after,
                    ));
                }
            }
        }
    }

    pub(crate) fn file_action(&mut self, input: FileActionInput<'_>) {
        let FileActionInput {
            ui,
            action,
            path,
            line,
        } = input;
        match action {
            FileAction::Open => self.open_file(OpenFileInput {
                path: path.into(),
                line,
                split: None,
                external: false,
            }),
            FileAction::Text => self.open_file_mode(OpenFileModeInput {
                path: path.into(),
                line,
                split: None,
                external: false,
                text: true,
            }),
            FileAction::Split => self.open_file(OpenFileInput {
                path: path.into(),
                line,
                split: Some("right"),
                external: false,
            }),
            FileAction::Here => self.open_file(OpenFileInput {
                path: path.into(),
                line,
                split: Some(Self::SPLIT_HERE),
                external: false,
            }),
            FileAction::External => self.open_file(OpenFileInput {
                path: path.into(),
                line,
                split: None,
                external: true,
            }),
            FileAction::Copy => ui.ctx().copy_text(path.display().to_string()),
            FileAction::StagedDiff | FileAction::WorkingDiff => {
                if let Some(root) = self.git_root() {
                    let available = self
                        .state
                        .capabilities
                        .iter()
                        .any(|c| c == NVIM_REVIEW_CAPABILITY);
                    self.spawn_diff(SpawnDiff {
                        cwd: root,
                        path: path.into(),
                        staged: action == FileAction::StagedDiff,
                        native: !available,
                    });
                    if !available {
                        self.info = Some("Using native diff: the running session service does not support Neovim reviews. Update the service after finishing your live sessions.".into());
                    }
                }
            }
            FileAction::NativeStagedDiff | FileAction::NativeWorkingDiff => {
                if let Some(root) = self.git_root() {
                    self.spawn_diff(SpawnDiff {
                        cwd: root,
                        path: path.into(),
                        staged: action == FileAction::NativeStagedDiff,
                        native: true,
                    });
                }
            }
            FileAction::Browser => self.open_in_browser(path),
        }
    }
    pub(crate) fn open_in_browser(&mut self, path: &Path) {
        let Some(url) = file_actions::file_url(path, &self.dialog_directory()) else {
            return;
        };
        let _ = self.jobs.send(Job::Browser(url));
    }
    pub(crate) fn perform_git_outcome(
        &mut self,
        ui: &egui::Ui,
        outcome: sidebar_ui::GitPanelOutcome,
    ) {
        for op in outcome.perforce {
            self.queue_workspace(workspace_ops::Op::Perforce(op));
        }
        if outcome.refresh {
            self.refresh_request = None;
            self.git.list_root = None;
        }
        if outcome.view_log {
            self.open_commit_log();
        }
        for clicked in outcome.clicked {
            self.activate_file_action(ui, &clicked.path, clicked.action);
        }
        for picked in outcome.menu {
            self.file_action(FileActionInput {
                ui,
                action: picked.action,
                path: &picked.path,
                line: None,
            });
        }
        if let Some(path) = outcome.resolve.clone() {
            self.open_merge(path);
        }
        if let Some(history) = outcome.history {
            self.git.history = history;
        }
        if outcome.collapse {
            self.git.collapse = self.git.collapse.wrapping_add(1);
        }
        if let Some(name) = outcome.switch {
            self.queue_workspace(workspace_ops::Op::Switch(name));
        }
        if outcome.load_head_message {
            self.queue_workspace(workspace_ops::Op::HeadMessage);
        }
        if outcome.commit || outcome.commit_push {
            let message = std::mem::take(&mut self.git.commit);
            let kept = message.trim();
            if !kept.is_empty() {
                let history = &mut self.preferences.commit_messages;
                history.retain(|old| old.trim() != kept);
                history.insert(0, kept.to_owned());
                history.truncate(20);
            }
            self.git.commit_inflight = Some(message.clone());
            self.queue_workspace(workspace_ops::Op::Commit {
                message,
                amend: self.git.amend,
                signoff: self.git.signoff,
                push: outcome.commit_push,
            });
            self.git.amend = false;
        }
        if outcome.shelve {
            let paths: Vec<PathBuf> = self
                .context
                .as_ref()
                .map(|context| {
                    context
                        .changes
                        .iter()
                        .filter(|change| change.in_group(terminator_git::GitGroup::Staged))
                        .map(|change| change.path.clone())
                        .collect()
                })
                .unwrap_or_default();
            self.queue_workspace(workspace_ops::Op::Shelve(paths));
        }
        if let Some(reference) = outcome.unshelve {
            self.queue_workspace(workspace_ops::Op::Unshelve(reference));
        }
        if let Some(reference) = outcome.drop_shelf {
            self.git.confirm = Some(crate::sidebar_ui::GitConfirm::DropShelf(reference));
        }
        for path in outcome.stage {
            self.queue_workspace(workspace_ops::Op::Stage(path));
        }
        if outcome.stage_all {
            let paths: Vec<PathBuf> = self
                .context
                .as_ref()
                .map(|context| {
                    context
                        .changes
                        .iter()
                        .filter(|change| !change.conflict())
                        .map(|change| change.path.clone())
                        .collect()
                })
                .unwrap_or_default();
            for path in paths {
                self.queue_workspace(workspace_ops::Op::Stage(path));
            }
        }
        if outcome.toggle_list {
            self.preferences.git_view_list = !self.preferences.git_view_list;
        }
        if outcome.clear_base {
            self.git.base_ref = None;
            self.queue_workspace(workspace_ops::Op::Compare { base: None });
        }
        if let Some(base) = outcome.set_base {
            self.git.base_ref = Some(base.clone());
            self.queue_workspace(workspace_ops::Op::Compare { base: Some(base) });
        }
        if outcome.refresh_compare {
            self.queue_workspace(workspace_ops::Op::Compare {
                base: self.git.base_ref.clone(),
            });
        }
        for path in outcome.unstage {
            self.queue_workspace(workspace_ops::Op::Unstage(path));
        }
        if let Some((path, untracked)) = outcome.discard.into_iter().last() {
            self.git.confirm = Some(crate::sidebar_ui::GitConfirm::Discard { path, untracked });
        }
        for path in outcome.delete {
            self.pending_delete = Some(path);
        }
        for text in outcome.copy {
            ui.ctx().copy_text(text);
        }
    }
    /// Open (or focus) a Git view tab in the selected project.
    pub(crate) fn open_git_tab(&mut self, tab: Tab) {
        let Some(project) = self.selected.clone() else {
            return;
        };
        let workspace = self
            .layouts
            .entry(project.clone())
            .or_insert_with(Workspace::empty);
        if !workspace.contains(&tab) {
            workspace.add(terminator_core::id(), tab.clone());
        }
        self.insert(&project, tab, None);
    }

    pub(crate) fn open_commit_log(&mut self) {
        let Some(project) = self.selected.clone() else {
            return;
        };
        let Some(context) = &self.context else {
            return;
        };
        let Some(root) = &context.root else {
            return;
        };
        let workspace = self
            .layouts
            .entry(project.clone())
            .or_insert_with(Workspace::empty);
        let tab = Tab::CommitLog { cwd: root.clone() };
        if !workspace.contains(&tab) {
            workspace.add(terminator_core::id(), tab.clone());
        }
        self.insert(&project, tab, None);
    }
    /// Performs a row-click action, collapsing rapid repeats the way a
    /// double-click does. Menu picks bypass this and go to `file_action`.
    pub(crate) fn activate_file_action(&mut self, ui: &egui::Ui, path: &Path, action: FileAction) {
        let staged = match action {
            FileAction::StagedDiff | FileAction::NativeStagedDiff => Some(true),
            FileAction::WorkingDiff | FileAction::NativeWorkingDiff => Some(false),
            _ => None,
        };
        let interval = Duration::from_secs_f64(
            ui.ctx()
                .options(|options| options.input_options.max_double_click_delay),
        );
        if !self.note_file_activation(NoteFileActivationInput {
            path: path.to_path_buf(),
            staged,
            action,
            interval,
        }) {
            return;
        }
        if matches!(
            action,
            FileAction::StagedDiff
                | FileAction::WorkingDiff
                | FileAction::NativeStagedDiff
                | FileAction::NativeWorkingDiff
        ) {
            self.hide_center_overlay();
        }
        self.file_action(FileActionInput {
            ui,
            action,
            path,
            line: None,
        });
    }
    /// Records a click activation; returns false when it repeats the previous
    /// one within the double-click interval.
    pub(crate) fn note_file_activation(&mut self, input: NoteFileActivationInput) -> bool {
        let NoteFileActivationInput {
            path,
            staged,
            action,
            interval,
        } = input;
        if self.file_activation.as_ref().is_some_and(|last| {
            last.project == self.selected
                && last.path == path
                && last.staged == staged
                && last.action == action
                && last.at.elapsed() < interval
        }) {
            return false;
        }
        self.file_activation = Some(FileActivation {
            project: self.selected.clone(),
            path,
            staged,
            action,
            at: Instant::now(),
        });
        true
    }
    pub(crate) fn spawn_diff(
        &mut self,
        SpawnDiff {
            cwd,
            path,
            staged,
            native,
        }: SpawnDiff,
    ) {
        let Some(project) = self.selected.clone() else {
            return;
        };
        if native {
            let tab = Tab::Diff {
                remote: None,
                cwd,
                path,
                staged,
            };
            if self.focus_file_view(&project, &tab) {
                return;
            }
            self.layouts
                .entry(project)
                .or_insert_with(Workspace::empty)
                .add(id(), tab.clone());
            self.active_session = None;
            self.diffs.remove(&tab.key());
            self.diff_preview.remove(&tab.key());
            self.loading.insert(tab.key());
            if self.state.settings.diff_split_default {
                self.diff_split.insert(tab.key());
            } else {
                self.diff_split.remove(&tab.key());
            }
            self.error = None;
            if self.neovim_review_unavailable() {
                self.info = Some("Using built-in diff. Neovim review needs the updated daemon; restart it after finishing your live sessions.".into());
            }
            let _ = self.jobs.send(Job::Diff(tab));
            return;
        }
        let _ = self.jobs.send(Job::rpc(
            Request::CreateReview {
                project,
                cwd,
                path,
                staged,
            },
            After::Workspace(id(), vec![]),
        ));
    }
    pub(crate) fn neovim_review_unavailable(&self) -> bool {
        self.state.settings.review_mode == ReviewMode::Neovim
            && !self
                .state
                .capabilities
                .iter()
                .any(|c| c == NVIM_REVIEW_CAPABILITY)
    }
    pub(crate) fn git_root(&self) -> Option<PathBuf> {
        if self.selected_remote_project().is_some() {
            return None;
        }
        self.context.as_ref().and_then(|context| {
            context
                .perforce
                .as_ref()
                .map(|workspace| workspace.command_root.clone())
                .or_else(|| context.root.clone())
        })
    }
    pub(crate) fn editors_only(&self, ids: &[String]) -> bool {
        !ids.is_empty()
            && !self.state.agents.iter().any(|agent| {
                ids.contains(&agent.session_id)
                    && !matches!(
                        agent.state,
                        AgentState::Completed | AgentState::Failed | AgentState::Stopped
                    )
            })
            && ids.iter().all(|id| {
                self.state
                    .sessions
                    .iter()
                    .any(|s| &s.id == id && s.kind == SessionKind::Editor)
            })
    }
    pub(crate) fn editor_close_busy(&self, ids: &[String]) -> bool {
        ids.iter().any(|id| self.editor_close_sessions.contains(id))
    }

    pub(crate) fn editor_close_prompted(&self, ids: &[String]) -> bool {
        self.editor_close_prompts
            .iter()
            .any(|(_, prompted, _)| prompted.iter().any(|id| ids.contains(id)))
    }

    pub(crate) fn skip_editor_close_request(&self, ids: &[String]) -> bool {
        self.editor_close_busy(ids) || self.editor_close_prompted(ids)
    }

    pub(crate) fn unsaved_close_prompt(
        &self,
        sid: &str,
    ) -> Option<(editor_close::Target, Vec<String>, String)> {
        self.editor_close_prompts
            .iter()
            .find(|(_, ids, _)| ids.iter().any(|id| id == sid))
            .cloned()
    }

    pub(crate) fn upsert_unsaved_close(
        &mut self,
        target: editor_close::Target,
        ids: Vec<String>,
        error: String,
    ) {
        if let Some(prompt) = self
            .editor_close_prompts
            .iter_mut()
            .find(|(_, prompted, _)| prompted == &ids || prompted.iter().any(|id| ids.contains(id)))
        {
            *prompt = (target, ids, error);
            return;
        }
        self.editor_close_prompts.push((target, ids, error));
    }

    pub(crate) fn apply_unsaved_close_choice(
        &mut self,
        choice: appearance::UnsavedCloseChoice,
        target: editor_close::Target,
        ids: Vec<String>,
    ) {
        match choice {
            appearance::UnsavedCloseChoice::Cancel => {
                self.editor_close_prompts
                    .retain(|(_, prompted, _)| prompted != &ids);
                if matches!(target, editor_close::Target::Workspace(..)) {
                    self.abort_workspace_close();
                }
            }
            appearance::UnsavedCloseChoice::Save => {
                self.close_editors(target, ids, editor_close::Mode::Save);
            }
            appearance::UnsavedCloseChoice::Discard => {
                self.close_editors(target, ids, editor_close::Mode::Discard);
            }
        }
    }

    pub(crate) fn editors_closed(
        &mut self,
        target: editor_close::Target,
        ids: Vec<String>,
        result: Result<(), String>,
    ) {
        for id in &ids {
            self.editor_close_sessions.remove(id);
        }
        match result {
            Ok(()) => {
                self.editor_close_prompts
                    .retain(|(_, prompted, _)| !prompted.iter().any(|id| ids.contains(id)));
                match target {
                    editor_close::Target::Workspace(project, id) => {
                        self.close_workspace_tab_now(&project, &id);
                    }
                    editor_close::Target::Pane(sid) => self.remove_tab(&sid),
                }
            }
            Err(error) => self.upsert_unsaved_close(target, ids, error),
        }
    }

    pub(crate) fn close_editors(
        &mut self,
        target: editor_close::Target,
        ids: Vec<String>,
        mode: editor_close::Mode,
    ) {
        if self.editor_close_busy(&ids) {
            return;
        }
        self.editor_close_sessions.extend(ids.iter().cloned());
        let timeout = Duration::from_secs(self.state.settings.editor_close_timeout_secs);
        let _ = self
            .jobs
            .send(Job::CloseEditors(target, ids, mode, timeout));
    }
}
