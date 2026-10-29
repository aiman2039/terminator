//! Drains queued [`Update`]s each frame and dispatches every one to a
//! topic handler in a submodule.

use std::time::{Duration, Instant};

use crate::*;

mod editors;
mod git;
mod notifications;
mod persistence;
mod pickers;
mod pty;
mod remote;
mod service;
mod sessions;
mod vnc;

use editors::ImageInput;
use git::{DirectoryPreviewInput, RefreshInput};
use sessions::FloatCreatedInput;

impl App {
    pub(crate) fn process_updates(&mut self, ctx: &egui::Context) {
        let processing_started = Instant::now();
        self.drain_updates(ctx);
        self.drain_pty_events(ctx);
        self.ui_service_peak_ms = self
            .ui_service_peak_ms
            .max(processing_started.elapsed().as_secs_f64() * 1000.0);
    }

    fn drain_updates(&mut self, ctx: &egui::Context) {
        let mut budget = native_jobs::ResultBudget::new();
        loop {
            if !budget.next() {
                ctx.request_repaint();
                break;
            }
            if self.service_ready.is_empty() && self.poll_service_completion(ctx).is_break() {
                continue;
            }
            let Some(update) = self.next_update() else {
                break;
            };
            self.dispatch_update(ctx, update);
        }
    }

    /// Takes the next update: a fresh state snapshot first, then ready
    /// service results, service events, and finally worker updates.
    fn next_update(&mut self) -> Option<Update> {
        if let Some(state) = self
            .service_owner
            .snapshot
            .try_lock()
            .ok()
            .and_then(|mut state| state.take())
        {
            return Some(Update::State(state));
        }
        self.service_ready
            .pop_front()
            .or_else(|| self.service_owner.events.try_recv().ok())
            .or_else(|| self.updates.try_recv().ok())
    }

    // sarah-allow(large_function): flat dispatch, one single-call arm per Update variant
    fn dispatch_update(&mut self, ctx: &egui::Context, update: Update) {
        match update {
            #[cfg(test)]
            Update::TestBarrier(reply) => reply.send(()).unwrap_or_default(),
            Update::LayoutsPrepared(generation, layouts) => {
                self.handle_layouts_prepared(generation, layouts);
            }
            Update::LayoutSaved(project, text, result) => {
                self.handle_layout_saved(project, text, result);
            }
            Update::RadioCatalog(catalog) => self.player.radio_base = catalog,
            Update::InstallationRepaired(result) => self.handle_installation_repaired(result),
            Update::ExitDrained(id, serial) => self.handle_exit_drained(id, serial),
            Update::ExitSaved(id, result) => self.handle_exit_saved(ctx, id, result),
            Update::PreferencesSaved(result) => self.handle_preferences_saved(*result),
            Update::ResolvedTarget(key, target) => self.handle_resolved_target(key, target),
            Update::EditorsClosed(target, ids, result) => {
                self.editors_closed(target, ids, result);
            }
            #[cfg(test)]
            Update::UiRequest(request, reply, deadline) => reply
                .send(self.handle_ui_request(ctx, request, deadline))
                .unwrap_or_default(),
            Update::AsyncUiRequest(request, reply, deadline) => reply
                .send(self.handle_ui_request(ctx, request, deadline))
                .unwrap_or_default(),
            Update::Metadata(generation, data) => self.handle_metadata(generation, data),
            Update::Resources(sample) => self.resources = Some(sample),
            Update::OpenImage(project, path, after) => {
                self.open_gui_tab(project, Tab::Image { path }, after);
            }
            Update::FocusViewport(viewport) => Self::handle_focus_viewport(ctx, viewport),
            Update::RevealFileSession(session, project) => {
                self.handle_reveal_file_session(ctx, session, project);
            }
            Update::OpenNativeEditor(project, path, after) => {
                self.handle_open_native_editor(project, path, after);
            }
            Update::OpenBrowser(project, target, after) => {
                self.handle_open_browser(project, target, after);
            }
            Update::RemoteAdded(project, generation, result) => {
                self.handle_remote_added(project, generation, result);
            }
            Update::RemoteReply(project, operation, result) => {
                self.apply_remote_reply(project, operation, result);
            }
            Update::HostKeyScanned(ssh, result) => self.apply_host_key_scan(&ssh, result),
            Update::HostKeyTrusted(ssh, result) => self.apply_host_key_trust(&ssh, result),
            Update::SshConfigHosts(generation, hosts) => {
                self.handle_ssh_config_hosts(generation, hosts);
            }
            Update::SshControlStatus(results) => self.apply_host_status(results),
            Update::Forwards(key, result) => self.apply_forwards(key, result),
            Update::RemoteAsset(project, path, after) => {
                self.handle_remote_asset(project, path, after);
            }
            Update::RemoteTransfer(project, id, result) => {
                self.apply_remote_transfer(project, id, result);
            }
            Update::VncTransfer(session, result) => self.handle_vnc_transfer(&session, result),
            Update::VncConnected(session, after) => {
                self.handle_vnc_connected(ctx, session, after);
            }
            Update::VncTabConnected(tab_id, result) => {
                self.handle_vnc_tab_connected(ctx, tab_id, result);
            }
            Update::VncFrame(session, result) => self.handle_vnc_frame(ctx, &session, result),
            Update::Image(path, generation, result) => self.handle_image(
                ctx,
                ImageInput {
                    path,
                    generation,
                    result,
                },
            ),
            Update::TestPickerClosed => self.picker_active = false,
            Update::HookStatus(status) => self.hook_status = status,
            Update::Appearance(file) => self.handle_appearance(ctx, *file),
            Update::Activation(notice) => self.handle_activation(ctx, notice),
            Update::AttentionMigrated(result) => self.handle_attention_migrated(result),
            Update::TypographyMigrated => self.preferences.typography_migrated = true,
            Update::IdleClosed(target, ids, result) => self.idle_closed(target, ids, result),
            Update::OpenedProject(state, project, generation) => {
                self.handle_opened_project(*state, project, generation);
            }
            Update::PickedProject(path, generation) => {
                self.handle_picked_project(path, generation);
            }
            Update::PickedPath { path, target } => self.handle_picked_path(path, target),
            Update::PickedAudio(paths) => self.handle_picked_audio(paths),
            Update::PickedVncFiles(paths) => self.vnc_import_files(paths),
            Update::PickedFile { path, project, cwd } => {
                self.handle_picked_file(path, project, cwd);
            }
            Update::State(state) => self.apply_state(*state),
            Update::ProjectDirectories(directories) => {
                self.apply_project_directories(directories);
            }
            Update::WorkspaceCreated(session, id, anchors) => {
                self.handle_workspace_created(session, id, anchors);
            }
            Update::Created(session, split, target) => {
                self.handle_created(session, split, target);
            }
            Update::StripCreated(session, split, anchors) => {
                self.handle_strip_created(session, split, anchors);
            }
            Update::FloatCreated(session, viewport, split, anchors) => {
                self.handle_float_created(FloatCreatedInput {
                    session,
                    viewport,
                    split,
                    anchors,
                });
            }
            Update::Text(key, text) => self.handle_text(key, text),
            Update::Diff(key, result) => self.handle_diff(key, result),
            Update::DiffActionFailed(key, error) => self.handle_diff_action_failed(key, error),
            Update::DiffAction(tab, undoing, result) => {
                self.handle_diff_action(tab, undoing, result);
            }
            Update::DirectoryPreview(generation, cwd, path, entries) => {
                self.handle_directory_preview(DirectoryPreviewInput {
                    generation,
                    cwd,
                    path,
                    entries,
                });
            }
            Update::Refresh(generation, context, directories, fallback) => {
                self.handle_refresh(RefreshInput {
                    generation,
                    context,
                    directories,
                    fallback,
                });
            }
            Update::ClipboardPaste(session, text) => self.handle_clipboard_paste(&session, &text),
            Update::WorktreeCreated(state, project, open_terminal) => {
                self.handle_worktree_created(*state, project, open_terminal);
            }
            Update::RestartFinished(message) => self.handle_restart_finished(&message),
            Update::ServiceStarted(result) => self.handle_service_started(result),
            Update::Error(e) => self.handle_error(e),
            Update::Info(i) => self.info = Some(i),
            Update::Workspace(op, result) => self.handle_workspace(op, result),
            Update::GitView(key, view) => self.handle_git_view(&key, view),
            Update::CommitFiles(hash, files) => {
                self.commit_files.insert(hash, Some(files));
            }
            Update::EditorBlame(path, revision, entries) => {
                self.apply_editor_blame(&path, revision, entries);
            }
            Update::GitBase(path, generation, base) => {
                self.apply_git_base(path, generation, base);
            }
            Update::LinePreviews(generation, lines) => {
                self.apply_line_previews(generation, lines);
            }
            Update::Search(id, result) => self.handle_search(id, result),
        }
    }

    pub(crate) fn migrate_attention(&mut self) {
        if self.preferences_writable
            && !self.preferences.attention_migrated
            && !self.attention_pending
            && self
                .attention_requested
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(5))
        {
            self.attention_requested = Some(Instant::now());
            self.attention_pending = self.jobs.send(Job::MigrateAttention).is_ok();
        }
    }
}
