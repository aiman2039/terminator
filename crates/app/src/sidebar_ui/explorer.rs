use super::attention::state_color;
use super::explorer_prompts::PromptSlot;
use super::explorer_rows::{explorer_file_row, skip_clipped_row};
use super::explorer_select::{ExplorerDrag, tree_focus_id};
use super::git_panel::{GitPanelInput, PreparedGit, git_color, git_panel};
#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::{
    Action, App, RenameSurface, Tab, appearance,
    preferences::{ExplorerSearchMode, SidebarTool},
    workspace_ops,
};
use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};
use terminator_core::{
    AgentState, NVIM_REVIEW_CAPABILITY, Project, Request, Session, SessionKind, now,
};

/// Explorer rows are one fixed height, which lets the tree paint only the
/// visible slice.
pub(super) const ROW_HEIGHT: f32 = 24.0;

/// Per-paint inputs every tree row shares.
struct RowContext<'a> {
    rows: &'a [crate::sidebar_cache::ExplorerRow],
    focus: egui::Id,
    focused_file: Option<PathBuf>,
    open_shortcut: String,
    split_shortcut: String,
}

pub(crate) struct ExplorerReveal {
    pub project: String,
    pub host: egui::Id,
    pub root: PathBuf,
    pub file: PathBuf,
    pub pending_scroll: bool,
}

impl App {
    /// The local file in the selected main pane. A focused strip shell
    /// means the visible editor is in the background, so it is not the
    /// current file for Reveal.
    pub(crate) fn focused_local_file(&self) -> Option<PathBuf> {
        let project = self.selected.as_ref()?;
        if self.active_session.as_ref().is_some_and(|sid| {
            self.state
                .sessions
                .iter()
                .any(|session| &session.id == sid && session.kind == SessionKind::Shell)
        }) {
            return None;
        }
        let pane = self.layouts.get(project)?.active_pane()?;
        match pane {
            Tab::NativeEditor { path }
            | Tab::Image { path }
            | Tab::Diff { path, .. }
            | Tab::Blame { path, .. }
            | Tab::RevisionDiff { path, .. }
            | Tab::FileHistory { path, .. } => Some(path.clone()),
            Tab::Browser { target, .. } => target.file().map(Path::to_path_buf),
            Tab::Terminal(sid) => self
                .state
                .sessions
                .iter()
                .find(|session| {
                    &session.id == sid
                        && &session.project_id == project
                        && session.kind == SessionKind::Editor
                        && !session.review
                })
                .and_then(|session| session.file.clone()),
            Tab::CommitLog { .. } | Tab::Player | Tab::Vnc { .. } => None,
        }
    }

    /// Prefer the Explorer's current root. If a file is elsewhere in its
    /// project, reveal it from the project root without changing the
    /// shell's working directory.
    pub(crate) fn explorer_reveal_candidate(&self) -> Option<(PathBuf, PathBuf)> {
        let file = self.focused_local_file()?;
        let project = self.selected_project()?;
        let cwd = self.cwd().unwrap_or_else(|| project.path.clone());
        let root = if file.starts_with(&cwd) {
            cwd
        } else if file.starts_with(&project.path) {
            project.path.clone()
        } else {
            return None;
        };
        let relative = file.strip_prefix(&root).ok()?;
        if relative.as_os_str().is_empty() || relative.components().count() > 20 {
            return None;
        }
        Some((file, root))
    }

    pub(crate) fn reveal_current_file_in_explorer(&mut self, host: egui::Id) -> bool {
        let Some((file, root)) = self.explorer_reveal_candidate() else {
            return false;
        };
        let Some(project) = self.selected.clone() else {
            return false;
        };
        let mut parent = file.parent();
        while let Some(dir) = parent.filter(|dir| *dir != root) {
            if !dir.starts_with(&root) {
                return false;
            }
            self.expanded_dirs.insert(dir.to_path_buf());
            parent = dir.parent();
        }
        self.preferences.explorer_search_mode = ExplorerSearchMode::Names;
        self.explorer_query.clear();
        self.explorer_search_last = None;
        self.explorer_reveal = Some(ExplorerReveal {
            project,
            host,
            root,
            file,
            pending_scroll: true,
        });
        self.refresh_request = None;
        true
    }
    pub(crate) fn explorer_tooltip(&self) -> String {
        let Some(cwd) = self.cwd() else {
            return "Explorer — no selected directory".into();
        };
        let unconfirmed = self.context_session().is_some_and(|s| !s.cwd_confirmed);
        format!(
            "Explorer\n{}{}",
            cwd.display(),
            if unconfirmed {
                "\nLast known directory"
            } else {
                ""
            }
        )
    }

    pub(crate) fn sidebar_project(&self, project: &Project) -> std::sync::Arc<Project> {
        let mut cached = self.sidebar_projects.borrow_mut();
        let entry = cached
            .entry(project.id.clone())
            .or_insert_with(|| std::sync::Arc::new(project.clone()));
        if entry.as_ref() != project {
            *entry = std::sync::Arc::new(project.clone());
        }
        std::sync::Arc::clone(entry)
    }

    pub(crate) fn visible_projects(&self) -> Vec<std::sync::Arc<Project>> {
        self.cached_projects().as_ref().clone()
    }

    /// Brand plus status for a project row when one of its live sessions has an agent.
    /// A running agent wins over a finished one, so a working logo is not replaced by a folder.
    pub(super) fn project_agent_face(
        &self,
        project_id: &str,
    ) -> Option<(&'static str, AgentState)> {
        self.sidebar_index().faces.get(project_id).copied()
    }

    pub(super) fn session_row(&mut self, ui: &mut egui::Ui, session: &Session) {
        self.session_row_for_tab(ui, session, None);
    }

    /// One session row that stands in for a single-terminal tab. It offers to
    /// name that tab, since the tab node is not painted.
    pub(super) fn session_row_in_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        project_id: &str,
        tab_id: &str,
    ) {
        self.session_row_for_tab(ui, session, Some((project_id, tab_id)));
    }

    fn session_row_for_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        rename_tab: Option<(&str, &str)>,
    ) {
        let presented = self.present_session(&session.id);
        let editing = self.renaming(&session.id, RenameSurface::Sidebar);
        let height = if !editing && presented.notice_preview.is_some() {
            appearance::SESSION_ROW_DETAIL_HEIGHT
        } else {
            appearance::SESSION_ROW_HEIGHT
        };
        if !editing && skip_clipped_row(ui, height) {
            return;
        }
        let index = self.sidebar_index();
        let terminal_note = index.terminal_notes.get(&session.id);
        let unread_terminal = terminal_note.is_some_and(|n| !n.dismissed);
        let color = appearance::color(if unread_terminal {
            &self.theme.accent
        } else {
            &self.theme.secondary
        });
        let visible = self
            .layouts
            .get(&session.project_id)
            .is_some_and(|d| d.contains(&Tab::Terminal(session.id.clone())));
        let secondary = if !session.lifecycle.live() {
            "ended"
        } else if !visible {
            "background"
        } else {
            ""
        };
        // The status icon carries agent state; the dot is for terminal notices.
        let trailing = if editing {
            ""
        } else if !secondary.is_empty() {
            secondary
        } else if unread_terminal {
            "●"
        } else {
            ""
        };
        let response = appearance::session_row_spec(
            ui,
            appearance::SessionRowSpec {
                label: if editing { "" } else { &session.label },
                icon: match presented.lifecycle {
                    Some(_) => presented.status_icon,
                    None if session.kind == SessionKind::Editor => "FileCode",
                    None => "Terminal",
                },
                selected: self.active_session.as_ref() == Some(&session.id),
                trailing,
                tint: color,
                icon_tint: presented
                    .lifecycle
                    .map(|state| state_color(state, &self.theme)),
                spin: presented.spin,
                subtitle: presented.notice_preview.as_deref().filter(|_| !editing),
                brand: presented.brand_icon,
            },
        )
        .on_hover_ui(|ui| {
            ui.label(format!(
                "{}{}{}",
                presented.diagnostics(now()),
                if secondary.is_empty() {
                    String::new()
                } else {
                    format!("\n{secondary}")
                },
                terminal_note
                    .map(|n| format!("\nTerminal: {}\n{}", n.title, n.body))
                    .unwrap_or_default()
            ));
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("session-row:{}", session.id),
            response.rect,
        );
        if editing {
            self.inline_rename(
                ui,
                &session.id,
                RenameSurface::Sidebar,
                egui::Rect::from_min_max(
                    egui::pos2(response.rect.min.x + 36.0, response.rect.min.y + 4.0),
                    egui::pos2(response.rect.max.x - 6.0, response.rect.max.y - 3.0),
                ),
            );
        }
        if response.clicked() && !editing {
            self.go_session(&session.id);
        }
        appearance::context_menu(&response, |ui| {
            self.rename_action(ui, &session.id, RenameSurface::Sidebar);
            ui.separator();
            if appearance::menu_item(ui, "Open session", "Terminal", "").clicked() {
                self.go_session(&session.id);
                ui.close();
            }
            if let Some((project_id, tab_id)) = rename_tab
                && appearance::menu_item(ui, "Name tab…", "Pencil", "").clicked()
            {
                self.rename_group = Some((project_id.to_string(), tab_id.to_string()));
                self.rename_group_text.clear();
                self.rename_group_focus = true;
                ui.close();
            }
            ui.separator();
            if session.lifecycle.live() {
                if appearance::menu_item(
                    ui,
                    "Close session…",
                    "X",
                    &self.shortcut_label(Action::CloseSession),
                )
                .clicked()
                {
                    self.close_session = Some(session.id.clone());
                    ui.close();
                }
            } else if appearance::menu_item(ui, "Remove historical record", "X", "").clicked() {
                self.send(Request::Remove {
                    session: session.id.clone(),
                });
                self.remove_tab(&session.id);
                ui.close();
            }
        });
    }

    pub(super) fn worktree_card(&mut self, ui: &mut egui::Ui, project: &Project) {
        let index = self.sidebar_index();
        let live = index.live_counts.get(&project.id).copied().unwrap_or(0);
        let (waiting, running) = index
            .worktree_states
            .get(&project.id)
            .copied()
            .unwrap_or_default();
        let tint = appearance::color(if waiting {
            &self.theme.status_waiting
        } else if running {
            &self.theme.status_running
        } else {
            &self.theme.secondary
        });
        let count = live.to_string();
        let selected = self.selected.as_ref() == Some(&project.id);
        let response = appearance::project_row(
            ui,
            &project.name,
            "GitBranch",
            selected,
            self.theme.row_height(),
            &count,
            tint,
        )
        .on_hover_text(format!(
            "{}\nManaged Git worktree\n{} live terminal(s)",
            self.project_location_label(&project.id),
            live
        ));
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("worktree-row:{}", project.id),
            response.rect,
        );
        if response.clicked() {
            self.select_project(project.id.clone());
        }
        appearance::context_menu(&response, |ui| {
            if appearance::menu_item(ui, "Open", "FolderOpen", "").clicked() {
                self.select_project(project.id.clone());
                ui.close();
            }
            if appearance::menu_item(
                ui,
                "New terminal",
                "Terminal",
                &self.shortcut_label(Action::NewTerminal),
            )
            .clicked()
            {
                self.select_project(project.id.clone());
                self.create(None);
                ui.close();
            }
            ui.separator();
            if appearance::menu_item(ui, "Remove worktree…", "X", "").clicked() {
                self.confirm_remove_worktree(&project.id);
                ui.close();
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn tree(&mut self, ui: &mut egui::Ui, path: &std::path::Path, _depth: usize) {
        self.tree_with_host(ui, path, ui.scope_id());
    }

    fn tree_with_host(&mut self, ui: &mut egui::Ui, path: &Path, host: egui::Id) {
        ui.spacing_mut().interact_size.y = ROW_HEIGHT;
        ui.spacing_mut().item_spacing.y = 0.0;
        let rows = self.explorer_rows(path);
        // The tree is a keyboard target of its own: register it every frame
        // (egui forgets unpainted focus targets), then let it read keys.
        let focus = tree_focus_id(host);
        ui.interact(
            ui.available_rect_before_wrap(),
            focus,
            egui::Sense::focusable_noninteractive(),
        );
        self.explorer_keys(ui, &rows, host, path);
        let context = RowContext {
            rows: &rows,
            focus,
            focused_file: self.focused_local_file(),
            open_shortcut: self.shortcut_label(Action::OpenFile),
            split_shortcut: self.shortcut_label(Action::SplitRight),
        };
        let mut drop_dir: Option<PathBuf> = None;
        // An open rename / new-item prompt paints as a tree row: in place
        // of the renamed row, or inserted under its folder.
        let slot = self.name_prompt_slot(&rows, path);
        let insert = match slot {
            Some(PromptSlot::Insert { index, depth }) => Some((index, depth)),
            _ => None,
        };
        let replace = match slot {
            Some(PromptSlot::Replace(index)) => Some(index),
            _ => None,
        };
        let count = rows.len().saturating_add(usize::from(insert.is_some()));
        // Painted position of row `index`, and the row at a painted position
        // (`None` for the inserted prompt).
        let shown_at = |index: usize| match insert {
            Some((at, _)) if index >= at => index.saturating_add(1),
            _ => index,
        };
        let row_at = |shown: usize| match insert {
            Some((at, _)) if shown == at => None,
            Some((at, _)) if shown > at => Some(shown.saturating_sub(1)),
            _ => Some(shown),
        };
        let prompt_shown = insert.map(|(at, _)| at).or(replace.map(shown_at));
        let scroll_to = self.explorer_scroll_to.take();
        let reveal = self
            .explorer_reveal
            .as_ref()
            .filter(|reveal| reveal.host == host && reveal.root == path && reveal.pending_scroll)
            .map(|reveal| reveal.file.clone());
        // Fixed-height rows (every listing loaded) paint only the visible
        // slice; status rows ("Loading…", errors) vary in height, so a tree
        // with any of them paints every row as before.
        if rows.iter().all(|row| row.entry.is_some()) {
            let top = ui.next_widget_position().y;
            let clip = ui.clip_rect();
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            // Row indices from UI coordinates.
            let (first, last) = (
                ((clip.top() - top) / ROW_HEIGHT).floor().max(0.0) as usize,
                ((clip.bottom() - top) / ROW_HEIGHT).ceil().max(0.0) as usize,
            );
            let last = last.saturating_add(1).min(count);
            let first = first.min(last);
            // A prompt that just opened scrolls into view so its field can
            // take focus (unpainted rows cannot).
            let prompt_target = prompt_shown.filter(|_| self.name_prompt_focus);
            let targets = [scroll_to.as_ref(), reveal.as_ref()]
                .into_iter()
                .flatten()
                .map(|target| {
                    (
                        rows.iter()
                            .position(|row| &row.path == target)
                            .map(shown_at),
                        Some(target) == reveal.as_ref(),
                    )
                })
                .chain(std::iter::once((prompt_target, false)));
            for (shown, center) in targets {
                if let Some(index) = shown {
                    #[allow(clippy::cast_precision_loss)] // UI coordinate
                    let y = top + index as f32 * ROW_HEIGHT;
                    ui.scroll_to_rect(
                        egui::Rect::from_min_size(
                            egui::pos2(clip.left(), y),
                            egui::vec2(1.0, ROW_HEIGHT),
                        ),
                        center.then_some(egui::Align::Center),
                    );
                }
            }
            if reveal.is_some()
                && let Some(active) = self.explorer_reveal.as_mut()
            {
                active.pending_scroll = false;
            }
            #[allow(clippy::cast_precision_loss)] // UI coordinate
            ui.allocate_space(egui::vec2(ui.available_width(), first as f32 * ROW_HEIGHT));
            for shown in first..last {
                match row_at(shown) {
                    None => {
                        self.explorer_prompt_row(ui, insert.map_or(0, |(_, depth)| depth), focus)
                    }
                    Some(index) => {
                        let Some(row) = rows.get(index) else {
                            continue;
                        };
                        if replace == Some(index) {
                            self.explorer_prompt_row(ui, row.depth, focus);
                        } else {
                            self.explorer_tree_row(ui, row, &context, &mut drop_dir);
                        }
                    }
                }
            }
            #[allow(clippy::cast_precision_loss)] // UI coordinate
            ui.allocate_space(egui::vec2(
                ui.available_width(),
                count.saturating_sub(last) as f32 * ROW_HEIGHT,
            ));
        } else {
            for shown in 0..count {
                let Some(index) = row_at(shown) else {
                    self.explorer_prompt_row(ui, insert.map_or(0, |(_, depth)| depth), focus);
                    continue;
                };
                let Some(row) = rows.get(index) else {
                    continue;
                };
                if replace == Some(index) {
                    self.explorer_prompt_row(ui, row.depth, focus);
                    continue;
                }
                let target =
                    scroll_to.as_ref() == Some(&row.path) || reveal.as_ref() == Some(&row.path);
                if target {
                    let rect = ui.available_rect_before_wrap();
                    ui.scroll_to_rect(
                        egui::Rect::from_min_size(rect.min, egui::vec2(1.0, ROW_HEIGHT)),
                        Some(egui::Align::Center),
                    );
                    if reveal.as_ref() == Some(&row.path)
                        && let Some(active) = self.explorer_reveal.as_mut()
                    {
                        active.pending_scroll = false;
                    }
                }
                self.explorer_tree_row(ui, row, &context, &mut drop_dir);
            }
        }
        self.explorer_external_drop(ui, path, drop_dir);
        if let Some(text) = self.explorer_speed_text() {
            let clip = ui.clip_rect();
            let galley = ui.painter().layout_no_wrap(
                format!("Search: {text}"),
                egui::FontId::proportional(12.0),
                ui.visuals().strong_text_color(),
            );
            let size = galley.size();
            let rect = egui::Rect::from_min_size(
                egui::pos2(clip.right() - size.x - 12.0, clip.top() + 2.0),
                egui::vec2(size.x + 8.0, size.y + 4.0),
            );
            ui.painter().rect_filled(
                rect,
                appearance::WIDGET_RADIUS,
                ui.visuals().extreme_bg_color,
            );
            ui.painter().galley(
                egui::pos2(rect.min.x + 4.0, rect.min.y + 2.0),
                galley,
                ui.visuals().text_color(),
            );
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(1600));
        }
        if let Some(count) =
            egui::DragAndDrop::payload::<ExplorerDrag>(ui.ctx()).map(|payload| payload.0.len())
            && let Some(pointer) = ui.ctx().pointer_latest_pos()
        {
            egui::Tooltip::always_open(
                ui.ctx().clone(),
                ui.layer_id(),
                focus.with("drag-label"),
                egui::PopupAnchor::Position(egui::pos2(pointer.x + 12.0, pointer.y + 12.0)),
            )
            .show(|ui| {
                ui.label(if count == 1 {
                    "Move 1 item".to_owned()
                } else {
                    format!("Move {count} items")
                })
            });
        }
    }

    /// The open name prompt as a tree row at `depth`.
    fn explorer_prompt_row(&mut self, ui: &mut egui::Ui, depth: usize, tree_focus: egui::Id) {
        // Exactly one row tall, so virtualized row math stays exact.
        let (mut rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::hover(),
        );
        #[allow(clippy::cast_precision_loss)] // UI coordinate
        let indent = depth as f32 * ui.spacing().indent;
        rect.min.x += indent;
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center))
                .id_salt("explorer-prompt-row"),
        );
        row.set_clip_rect(rect.intersect(ui.clip_rect()));
        self.name_prompt_row(&mut row, tree_focus);
    }

    fn explorer_tree_row(
        &mut self,
        ui: &mut egui::Ui,
        row: &crate::sidebar_cache::ExplorerRow,
        context: &RowContext<'_>,
        drop_dir: &mut Option<PathBuf>,
    ) {
        let mut rect = ui.available_rect_before_wrap();
        // Explorer indent is a UI coordinate; depth can exceed the f32 mantissa.
        #[allow(clippy::cast_precision_loss)]
        let indent = row.depth as f32 * ui.spacing().indent;
        rect.min.x += indent;
        ui.push_id(&row.path, |ui| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                if let Some(entry) = &row.entry {
                    if skip_clipped_row(ui, ROW_HEIGHT) {
                        return;
                    }
                    self.explorer_entry(ui, entry, &row.label, context, drop_dir);
                } else {
                    self.explorer_directory_status(ui, &row.path);
                }
            });
        });
    }

    /// Files dropped from the OS file manager onto the tree copy into the
    /// folder under the pointer (or the tree root).
    fn explorer_external_drop(&mut self, ui: &egui::Ui, root: &Path, drop_dir: Option<PathBuf>) {
        let clip = ui.clip_rect();
        let pointer_inside = ui
            .ctx()
            .pointer_latest_pos()
            .is_none_or(|pointer| clip.contains(pointer));
        if !pointer_inside {
            return;
        }
        if ui.ctx().input(|input| !input.raw.hovered_files.is_empty()) {
            ui.painter().rect_stroke(
                clip,
                4.0,
                egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
            ui.ctx().request_repaint();
        }
        let dropped: Vec<PathBuf> = ui.ctx().input_mut(|input| {
            std::mem::take(&mut input.raw.dropped_files)
                .into_iter()
                .map(|file| file.path().to_path_buf())
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.queue_workspace(workspace_ops::Op::Copy {
                from: dropped,
                to_dir: drop_dir.unwrap_or_else(|| root.to_owned()),
            });
        }
    }

    /// Drag source and drop target wiring shared by file and folder rows.
    /// Dropping on a folder moves into it; on a file, into its folder.
    fn explorer_row_dnd(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        path: &Path,
        directory: bool,
        drop_dir: &mut Option<PathBuf>,
    ) {
        let target = if directory {
            Some(path.to_owned())
        } else {
            path.parent().map(Path::to_path_buf)
        };
        if response.contains_pointer() {
            drop_dir.clone_from(&target);
        }
        if response.dragged() {
            let paths = if self.explorer_selection.iter().any(|p| p == path) {
                self.explorer_selection.clone()
            } else {
                vec![path.to_owned()]
            };
            response.dnd_set_drag_payload(ExplorerDrag(paths));
        }
        if directory && response.dnd_hover_payload::<ExplorerDrag>().is_some() {
            ui.painter().rect_stroke(
                response.rect,
                appearance::WIDGET_RADIUS,
                egui::Stroke::new(1.5, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(payload) = response.dnd_release_payload::<ExplorerDrag>()
            && let Some(target) = target
        {
            self.explorer_drop(payload.0.clone(), target);
        }
    }

    fn explorer_directory_status(&mut self, ui: &mut egui::Ui, path: &Path) {
        if let Some(error) = self.directory_errors.get(path).cloned() {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!(
                    "Cannot refresh {} ({:?}): {}",
                    error.path.display(),
                    error.kind,
                    error.message
                ),
            );
            if self.dirs.contains_key(path) {
                ui.weak("Showing the last successful listing.");
            }
            ui.horizontal(|ui| {
                let retry = ui.button("Retry");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "directory-retry", retry.rect);
                if retry.clicked() {
                    self.refresh_request = None;
                }
                if ui.button("Choose folder again").clicked() {
                    self.add_project = true;
                }
            });
            if cfg!(target_os = "macos") {
                ui.weak("Check System Settings → Privacy & Security → Files and Folders. Full Disk Access is optional troubleshooting; this error may have another cause. Shells and editors can have separate access.");
            }
        }
        if !self.dirs.contains_key(path) && !self.directory_errors.contains_key(path) {
            ui.weak("Loading…");
        }
    }

    fn explorer_entry(
        &mut self,
        ui: &mut egui::Ui,
        entry: &terminator_git::Entry,
        label: &str,
        context: &RowContext<'_>,
        drop_dir: &mut Option<PathBuf>,
    ) {
        let modifiers = ui.input(|input| input.modifiers);
        if entry.directory {
            let expanded = self.expanded_dirs.contains(&entry.path);
            let status = self
                .context
                .as_ref()
                .and_then(|c| c.decorations.get(&entry.path))
                .copied()
                .unwrap_or(' ');
            let color = if entry.ignored {
                appearance::color(&self.theme.git_ignored)
            } else {
                git_color(&self.theme, status)
            };
            let folder = appearance::file_row(
                ui,
                label,
                if expanded { "FolderOpen" } else { "Folder" },
                self.explorer_selected(&entry.path, None),
                22.0,
                &status.to_string(),
                color,
            )
            .interact(egui::Sense::drag());
            self.explorer_row_dnd(ui, &folder, &entry.path, true, drop_dir);
            // Build the tooltip lazily: `format!` per visible row per
            // frame showed up as scroll-time allocations.
            let folder = if folder.hovered() {
                folder.on_hover_text(format!(
                    "{}\n{}",
                    entry.path.display(),
                    if entry.ignored {
                        "Ignored"
                    } else {
                        terminator_git::status_description(status)
                    }
                ))
            } else {
                folder
            };
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), &format!("explorer-folder:{label}"), folder.rect);
            if folder.clicked() {
                ui.memory_mut(|memory| memory.request_focus(context.focus));
                if self.explorer_click(context.rows, &entry.path, modifiers) {
                    if expanded {
                        self.expanded_dirs.remove(&entry.path);
                    } else {
                        self.expanded_dirs.insert(entry.path.clone());
                    }
                }
            }
            if folder.secondary_clicked() && !self.explorer_selected(&entry.path, None) {
                self.explorer_selection = vec![entry.path.clone()];
                self.explorer_anchor = Some(entry.path.clone());
            }
            let folder_path = entry.path.clone();
            appearance::context_menu(&folder, |ui| {
                if appearance::menu_item(ui, "New file", "File", "").clicked() {
                    self.open_name_prompt(workspace_ops::NamePrompt::File {
                        dir: folder_path.clone(),
                        name: String::new(),
                    });
                    ui.close();
                }
                if appearance::menu_item(ui, "New folder", "Folder", "").clicked() {
                    self.open_name_prompt(workspace_ops::NamePrompt::Folder {
                        dir: folder_path.clone(),
                        name: String::new(),
                    });
                    ui.close();
                }
                if appearance::menu_item(ui, "Reveal in file manager", "FolderOpen", "").clicked() {
                    self.queue_workspace(workspace_ops::Op::Reveal(folder_path.clone()));
                    ui.close();
                }
                if appearance::menu_item(ui, "Copy path", "Copy", "").clicked() {
                    ui.ctx().copy_text(folder_path.display().to_string());
                    ui.close();
                }
                if self.explorer_clipboard.is_some()
                    && appearance::menu_item(ui, "Paste", "ClipboardPaste", "").clicked()
                {
                    self.explorer_selection = vec![folder_path.clone()];
                    let root = folder_path.clone();
                    self.explorer_paste(&root);
                    ui.close();
                }
                ui.separator();
                if appearance::menu_item(ui, "Rename", "Pencil", "").clicked() {
                    self.explorer_rename(&folder_path);
                    ui.close();
                }
                if appearance::menu_item(ui, "Duplicate", "Copy", "").clicked() {
                    self.queue_workspace(workspace_ops::Op::Duplicate(folder_path.clone()));
                    ui.close();
                }
                if appearance::menu_item(ui, "Delete", "X", "").clicked() {
                    self.explorer_confirm_delete();
                    ui.close();
                }
                ui.separator();
                if appearance::menu_item(ui, "Collapse all", "ChevronDown", "").clicked() {
                    self.expanded_dirs.clear();
                    ui.close();
                }
            });
        } else {
            let status = self
                .context
                .as_ref()
                .and_then(|c| c.decorations.get(&entry.path))
                .copied()
                .unwrap_or(' ');
            let outcome = explorer_file_row(
                ui,
                &entry.path,
                label,
                status,
                entry.ignored,
                &self.theme,
                &context.open_shortcut,
                &context.split_shortcut,
                self.explorer_selected(&entry.path, context.focused_file.as_deref()),
                self.context
                    .as_ref()
                    .is_some_and(|context| context.perforce.is_some()),
                self.context
                    .as_ref()
                    .and_then(|context| context.root.as_ref())
                    .map(|_| self.git.branches.as_slice()),
            );
            if let Some(response) = &outcome.response {
                self.explorer_row_dnd(ui, response, &entry.path, false, drop_dir);
                if response.secondary_clicked() && !self.explorer_selected(&entry.path, None) {
                    self.explorer_selection = vec![entry.path.clone()];
                    self.explorer_anchor = Some(entry.path.clone());
                }
            }
            if let Some(action) = outcome.clicked {
                ui.memory_mut(|memory| memory.request_focus(context.focus));
                if self.explorer_click(context.rows, &entry.path, modifiers) {
                    self.activate_file_action(ui, &entry.path, action);
                }
            }
            if let Some(action) = outcome.menu {
                self.file_action(ui, action, &entry.path, None);
            }
            if let Some(op) = outcome.perforce {
                self.queue_workspace(workspace_ops::Op::Perforce(op));
            }
            if let Some(local) = outcome.local {
                self.explorer_local(&entry.path, local, ui);
            }
        }
    }

    pub(crate) fn sidebar(&mut self, ui: &mut egui::Ui) {
        let tool = self.preferences.tool;
        self.sidebar_tool(tool, ui);
    }

    /// One tool's content, hostable by either sidebar.
    pub(crate) fn sidebar_tool(&mut self, tool: SidebarTool, ui: &mut egui::Ui) {
        if self.selected_remote_project().is_some()
            && matches!(
                tool,
                SidebarTool::Explorer
                    | SidebarTool::Git
                    | SidebarTool::Structure
                    | SidebarTool::Problems
                    | SidebarTool::Info
            )
        {
            self.remote_sidebar(ui, tool);
            return;
        }
        if tool == SidebarTool::Info {
            self.info_panel(ui);
            return;
        }
        if tool == SidebarTool::History {
            let index = self.sidebar_index();
            let groups = self.cached_history();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let width = (ui.available_width() - 160.0).max(60.0);
                let response = ui.add_sized(
                    egui::vec2(width, 22.0),
                    appearance::singleline(&mut self.preferences.history_filter)
                        .id(ui.scope_id().with("history-filter"))
                        .hint_text("Filter by name"),
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "history-filter", response.rect);
                #[cfg(not(feature = "test-support"))]
                let _ = &response;
                if !self.preferences.history_filter.is_empty()
                    && appearance::sidebar_action(ui, "X", "Clear filter").clicked()
                {
                    self.preferences.history_filter.clear();
                }
                let all_expanded = !groups.is_empty()
                    && groups.iter().all(|group| {
                        self.preferences
                            .history_expanded
                            .get(&group.project.id)
                            .copied()
                            .unwrap_or(true)
                    });
                let toggle = ui
                    .add_enabled(
                        !groups.is_empty(),
                        egui::Button::new(if all_expanded {
                            "Collapse all"
                        } else {
                            "Expand all"
                        }),
                    )
                    .on_hover_text("Collapse or expand all History projects");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "history-toggle-all", toggle.rect);
                if toggle.clicked() {
                    for group in groups.iter() {
                        self.preferences
                            .history_expanded
                            .insert(group.project.id.clone(), !all_expanded);
                    }
                }
                self.history_sort_menu(ui);
            });
            ui.add_space(4.0);
            appearance::sidebar_scroll("global-history").show(ui, |ui| {
                if index.resumable.is_empty() {
                    ui.weak("No resumable sessions.");
                } else if groups.is_empty() {
                    ui.weak("No matching sessions.");
                }
                for group in groups.iter() {
                    let expanded = *self
                        .preferences
                        .history_expanded
                        .entry(group.project.id.clone())
                        .or_insert(true);
                    let header = appearance::row(
                        ui,
                        &group.project.name,
                        if expanded {
                            "ChevronDown"
                        } else {
                            "ChevronRight"
                        },
                        false,
                        26.0,
                        &group.sessions.len().to_string(),
                        appearance::color(&self.theme.text),
                    )
                    .on_hover_text(self.project_location_label(&group.project.id));
                    #[cfg(feature = "test-support")]
                    diagnostics::record(
                        ui.ctx(),
                        &format!("history-project:{}", group.project.id),
                        header.rect,
                    );
                    if header.clicked() {
                        self.preferences
                            .history_expanded
                            .insert(group.project.id.clone(), !expanded);
                    }
                    if expanded {
                        ui.indent(("history-project", &group.project.id), |ui| {
                            for session in &group.sessions {
                                self.session_row(ui, session);
                            }
                        });
                    }
                    ui.add_space(8.0);
                }
            });
            return;
        }
        if tool == SidebarTool::Problems {
            self.problems_panel(ui);
            return;
        }
        if tool == SidebarTool::Structure {
            self.structure_panel(ui);
            return;
        }
        if tool == SidebarTool::Vnc {
            self.vnc_sidebar(ui);
            return;
        }
        if tool == SidebarTool::Hosts {
            self.hosts_sidebar(ui);
            return;
        }
        if tool == SidebarTool::Agents {
            self.agents_view(ui);
            return;
        }
        if tool == SidebarTool::Explorer {
            if self.explorer_reveal.as_ref().is_some_and(|reveal| {
                self.selected.as_deref() != Some(&reveal.project)
                    || self.focused_local_file().as_ref() != Some(&reveal.file)
            }) {
                self.explorer_reveal = None;
            }
            self.pending_delete_bar(ui);
            let host = ui.scope_id();
            self.explorer_autoreveal(host);
            // Branches for "Compare with branch" (loaded once per repo).
            let git_root = self
                .context
                .as_ref()
                .and_then(|context| context.root.clone());
            self.ensure_git_lists(git_root);
            let root_override = self
                .explorer_reveal
                .as_ref()
                .filter(|reveal| reveal.host == host)
                .map(|reveal| reveal.root.clone());
            if let Some(cwd) = root_override.or_else(|| self.cwd()) {
                self.explorer_toolbar(ui, &cwd);
                let cwd = self
                    .explorer_reveal
                    .as_ref()
                    .filter(|reveal| reveal.host == host)
                    .map_or(cwd, |reveal| reveal.root.clone());
                let contents =
                    self.preferences.explorer_search_mode == ExplorerSearchMode::Contents;
                if contents {
                    appearance::sidebar_scroll("files-search")
                        .max_height(ui.available_height())
                        .show(ui, |ui| self.explorer_results(ui, &cwd));
                } else {
                    appearance::sidebar_scroll("files")
                        .max_height(ui.available_height())
                        .show(ui, |ui| self.tree_with_host(ui, &cwd, host));
                }
            }
            return;
        }
        if self.context_session().is_some_and(|s| !s.cwd_confirmed) {
            ui.label(RichText::new("Last known directory").small().weak());
        }
        ui.separator();
        if self.watch_fallback {
            ui.weak("Filesystem watch unavailable; refreshing every 3 seconds");
        }
        self.pending_delete_bar(ui);
        self.pending_git_confirm_bar(ui);
        if let Some(context) = self.context.take() {
            if context.perforce.is_some() {
                let open_shortcut = self.shortcut_label(Action::OpenFile);
                let outcome =
                    super::perforce_panel::panel(ui, &context, &self.theme, &open_shortcut);
                self.context = Some(context);
                self.perform_git_outcome(ui, outcome);
                return;
            }
            let prepared = {
                let cache = self.sidebar_cache.get_mut();
                let git = cache
                    .git
                    .get_or_insert_with(|| std::sync::Arc::new(PreparedGit::new(&context)));
                std::sync::Arc::clone(git)
            };
            self.ensure_git_lists(context.root.clone());
            let open_shortcut = self.shortcut_label(Action::OpenFile);
            let mut draft = std::mem::take(&mut self.git.commit);
            let recent_messages = self.preferences.commit_messages.clone();
            let outcome = git_panel(
                ui,
                &mut GitPanelInput {
                    context: &context,
                    prepared: Some(&prepared),
                    review_mode: self.state.settings.review_mode,
                    neovim_review: self
                        .state
                        .capabilities
                        .iter()
                        .any(|c| c == NVIM_REVIEW_CAPABILITY),
                    theme: &self.theme,
                    history: self.git.history,
                    view_list: self.preferences.git_view_list,
                    commits: &self.git.log,
                    shelves: &self.git.shelves,
                    branches: &self.git.branches,
                    commit_draft: &mut draft,
                    recent_messages: &recent_messages,
                    amend: &mut self.git.amend,
                    signoff: &mut self.git.signoff,
                    collapse_generation: self.git.collapse,
                    open_shortcut: &open_shortcut,
                    compare: self
                        .git
                        .compare
                        .as_ref()
                        .filter(|_| self.git.compare_root == context.root),
                    base_ref: self.git.base_ref.as_deref(),
                    committing: self.git.commit_inflight.is_some(),
                },
            );
            self.git.commit = draft;
            self.context = Some(context);
            self.perform_git_outcome(ui, outcome);
        } else {
            ui.weak("Select a terminal to inspect its context.");
        }
    }
}
